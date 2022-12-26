// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::FlowCtrl;

use crate::node::{
    core::NodeContext, flow_ctrl::cmds::Cmd, membership::Membership, node_starter::CmdChannel,
    MyNode,
};

use sn_interface::{messaging::system::NodeMsg, types::log_markers::LogMarker};

use std::{sync::Arc, time::Duration};
use tokio::{sync::RwLock, time::Instant};

const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const MISSING_VOTE_INTERVAL: Duration = Duration::from_secs(5);
const MISSING_DKG_MSG_INTERVAL: Duration = Duration::from_secs(5);
const SECTION_PROBE_INTERVAL: Duration = Duration::from_secs(300);
const DYSFUNCTION_CHECK_INTERVAL: Duration = Duration::from_secs(5);
// 30 adult nodes checked per minute., so each node should be queried 10x in 10 mins
// Which should hopefully trigger dysfunction if we're not getting responses back
// const ADULT_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const ELDER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(3);

pub(super) struct PeriodicChecksTimestamps {
    last_probe: Instant,
    last_section_probe: Instant,
    // last_adult_health_check: Instant,
    last_elder_health_check: Instant,
    last_vote_check: Instant,
    last_dkg_msg_check: Instant,
    last_dysfunction_check: Instant,
}

impl PeriodicChecksTimestamps {
    pub(super) fn now() -> Self {
        Self {
            last_probe: Instant::now(),
            last_section_probe: Instant::now(),
            // last_adult_health_check: Instant::now(),
            last_elder_health_check: Instant::now(),
            last_vote_check: Instant::now(),
            last_dkg_msg_check: Instant::now(),
            last_dysfunction_check: Instant::now(),
        }
    }
}

impl FlowCtrl {
    /// Generate and fire commands for all types of periodic checks
    pub(super) async fn perform_periodic_checks(&mut self) {
        let (context, membership_context) = {
            let read_locked_node = self.node.read().await;
            (
                &read_locked_node.context(),
                read_locked_node.membership.clone(),
            )
        };

        if !context.is_elder {
            self.enqueue_cmds_for_adult_periodic_checks(context).await;

            // we've pushed what we have as an adult and processed incoming msgs
            // and cmds... so we can return already
            return;
        }

        self.enqueue_cmds_for_elder_periodic_checks(context, membership_context)
            .await;
    }

    /// Periodic tasks run for adults only
    async fn enqueue_cmds_for_adult_periodic_checks(&mut self, context: &NodeContext) {
        let mut cmds = vec![];

        // if we've passed enough time, section probe
        if self.timestamps.last_section_probe.elapsed() > SECTION_PROBE_INTERVAL {
            self.timestamps.last_section_probe = Instant::now();
            cmds.push(Self::probe_the_section(context).await);
        }

        for cmd in cmds {
            if let Err(error) = self.cmd_sender_channel.send((cmd, vec![])).await {
                error!("Error queuing adult periodic check: {error:?}");
            }
        }
    }

    /// Periodic tasks run for elders only
    async fn enqueue_cmds_for_elder_periodic_checks(
        &mut self,
        context: &NodeContext,
        membership_context: Option<Membership>,
    ) {
        let now = Instant::now();
        let mut cmds = vec![];

        if self.timestamps.last_probe.elapsed() > PROBE_INTERVAL {
            self.timestamps.last_probe = now;
            if let Some(cmd) = Self::probe_the_network(context).await {
                cmds.push(cmd);
            }
        }

        // TODO: reevaluate health checks. Does this make sense mocking a client req?
        // how do we handle the need for a bidi stream (if using the whole client flow?)

        // if self.timestamps.last_adult_health_check.elapsed() > ADULT_HEALTH_CHECK_INTERVAL {
        //     debug!(" ----> adult health periodics sgtart");
        //     self.timestamps.last_adult_health_check = now;
        //     let health_cmds = match Self::perform_health_checks(self.node.clone()).await {
        //         Ok(cmds) => cmds,
        //         Err(error) => {
        //             error!("Error handling client msg to perform health check: {error:?}");
        //             vec![]
        //         }
        //     };
        //     cmds.extend(health_cmds);
        //     debug!(" ----> adult health periodics done");
        // }

        // The above health check only queries for chunks
        // here we specifically ask for AE prob msgs and manually
        // track dysfunction
        if self.timestamps.last_elder_health_check.elapsed() > ELDER_HEALTH_CHECK_INTERVAL {
            self.timestamps.last_elder_health_check = now;
            for cmd in Self::health_check_elders_in_section(context).await {
                cmds.push(cmd);
            }
        }

        if self.timestamps.last_vote_check.elapsed() > MISSING_VOTE_INTERVAL {
            debug!(" ----> vote periodics start");
            self.timestamps.last_vote_check = now;
            for cmd in self
                .check_for_missed_votes(context, membership_context)
                .await
            {
                cmds.push(cmd);
            }
            debug!(" ----> vote periodics done");
        }

        if self.timestamps.last_dkg_msg_check.elapsed() > MISSING_DKG_MSG_INTERVAL {
            debug!(" ----> dkg msg periodics start");
            self.timestamps.last_dkg_msg_check = now;
            Self::check_for_missed_dkg_messages(self.node.clone(), self.cmd_sender_channel.clone())
                .await;
            debug!(" ----> dkg msg periodics done");
        }

        if self.timestamps.last_dysfunction_check.elapsed() > DYSFUNCTION_CHECK_INTERVAL {
            self.timestamps.last_dysfunction_check = now;
            let dysf_cmds = self.check_for_dysfunction().await;
            cmds.extend(dysf_cmds);
        }

        for cmd in cmds {
            if let Err(error) = self.cmd_sender_channel.send((cmd, vec![])).await {
                error!("Error queuing std periodic check: {error:?}");
            }
        }
    }

    // /// Initiates and generates all the subsequent Cmds to perform a healthcheck
    // async fn perform_health_checks(node: Arc<RwLock<MyNode>>) -> Result<Vec<Cmd>> {
    //     info!("Starting to check the section's health");
    //     let node = node.read().await;
    //     let our_prefix = node.network_knowledge.prefix();

    //     // random chunk addr will be sent to relevant nodes in the section.
    //     let chunk_addr = xor_name::rand::random();

    //     let chunk_addr = our_prefix.substituted_in(chunk_addr);

    //     let msg = ClientMsg::Query(DataQuery {
    //         variant: DataQueryVariant::GetChunk(ChunkAddress(chunk_addr)),
    //         holder_index: 0,
    //     });

    //     let msg_id = MsgId::new();
    //     let our_info = node.info();
    //     let origin = our_info.peer();

    //     let auth = auth(&node, &msg)?;

    //     // generate the cmds, and ensure we go through dysfunction tracking
    //     node.handle_valid_client_msg(
    //         msg_id,
    //         msg,
    //         auth,
    //         origin,
    //     )
    //     .await
    // }

    /// Generates a probe msg, which goes to a random section in order to
    /// passively maintain network knowledge over time
    async fn probe_the_network(context: &NodeContext) -> Option<Cmd> {
        let prefix = context.network_knowledge.prefix();

        // Send a probe message if we are an elder
        // but dont bother if we're the first section
        if !prefix.is_empty() {
            info!("Probing network");
            match MyNode::generate_probe_msg(context) {
                Ok(cmd) => Some(cmd),
                Err(error) => {
                    error!("Could not generate probe msg: {error:?}");
                    None
                }
            }
        } else {
            None
        }
    }

    /// Generates a probe msg, which goes to our elders in order to
    /// passively maintain network knowledge over time
    async fn probe_the_section(context: &NodeContext) -> Cmd {
        // Send a probe message to an elder
        info!("Starting to probe section");
        MyNode::generate_section_probe_msg(context)
    }

    /// Generates a probe msg, which goes to all section elders in order to
    /// passively maintain network knowledge over time and track dysfunction
    /// Tracking dysfunction while awaiting a response
    async fn health_check_elders_in_section(context: &NodeContext) -> Vec<Cmd> {
        let mut cmds = vec![];

        // Send a probe message to an elder
        debug!("Going to health check elders");

        let elders = context.network_knowledge.elders();
        for elder in elders {
            // we track a knowledge issue
            // whhich is countered when an AE-Update is
            cmds.push(Cmd::TrackNodeIssueInDysfunction {
                name: elder.name(),
                issue: sn_dysfunction::IssueType::AeProbeMsg,
            });
        }

        // Send a probe message to an elder
        cmds.push(MyNode::generate_section_probe_msg(context));

        cmds
    }

    /// Checks the interval since last vote received during a generation
    async fn check_for_missed_votes(
        &self,
        context: &NodeContext,
        membership_context: Option<Membership>,
    ) -> Vec<Cmd> {
        info!("Checking for missed votes");
        let mut cmds = vec![];
        if let Some(membership) = &membership_context {
            let last_received_vote_time = membership.last_received_vote_time();

            if let Some(time) = last_received_vote_time {
                // we want to resend the prev vote
                if time.elapsed() >= MISSING_VOTE_INTERVAL {
                    debug!("Vote consensus appears stalled...");
                    if let Some(cmd) =
                        MyNode::membership_gossip_votes(context, &membership_context).await
                    {
                        trace!("Vote resending cmd: {cmd:?}");

                        cmds.push(cmd);
                    }
                    // we may also be behind, so lets request AE incase that is the case!
                    let msg = NodeMsg::MembershipAE(membership.generation());
                    cmds.push(MyNode::send_msg_to_our_elders(context, msg));
                }
            }
        }
        cmds
    }

    /// Checks the interval since last dkg vote received
    async fn check_for_missed_dkg_messages(node: Arc<RwLock<MyNode>>, cmd_channel: CmdChannel) {
        info!("Checking for DKG missed messages");

        // DKG checks can be long running, move off thread to unblock the main loop
        let _handle = tokio::task::spawn(async move {
            debug!("[NODE READ]: dkg msg lock attempt");
            let node = node.read().await;
            debug!("[NODE READ]: dkg msg lock got");

            let dkg_voter = &node.dkg_voter;

            let last_received_dkg_message = dkg_voter.last_received_dkg_message();

            if let Some(time) = last_received_dkg_message {
                if time.elapsed() >= MISSING_DKG_MSG_INTERVAL {
                    debug!("Dkg voting appears stalled...");
                    let cmds = node.dkg_gossip_msgs();
                    if !cmds.is_empty() {
                        trace!("Dkg msg resending cmd");
                    }

                    for cmd in cmds {
                        if let Err(error) = cmd_channel.send((cmd, vec![])).await {
                            error!("Error sending DKG gossip msgs {error:?}");
                        }
                    }
                }
            }
        });
    }

    async fn check_for_dysfunction(&mut self) -> Vec<Cmd> {
        info!("Performing dysfunction checking");
        let mut cmds = vec![];
        let dysfunctional_nodes = self.get_dysfunctional_node_names().await;

        if !dysfunctional_nodes.is_empty() {
            debug!("{:?} : {dysfunctional_nodes:?}", LogMarker::ProposeOffline);
            cmds.push(Cmd::ProposeVoteNodesOffline(dysfunctional_nodes))
        }

        cmds
    }
}

// fn auth(node: &MyNode, msg: &ClientMsg) -> Result<AuthorityProof<ClientAuth>> {
//     let keypair = node.keypair.clone();
//     let payload = WireMsg::serialize_msg_payload(&msg)?;
//     let signature = keypair.sign(&payload);

//     let auth = ClientAuth {
//         public_key: PublicKey::Ed25519(keypair.public),
//         signature: Signature::Ed25519(signature),
//     };

//     Ok(AuthorityProof::verify(auth, payload)?)
// }
