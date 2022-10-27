// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::{
    comm::Comm,
    node::{
        flow_ctrl::cmds::Cmd,
        messaging::{OutgoingMsg, Peers},
        Event, MembershipEvent, MyNode, Proposal as CoreProposal, Result, MIN_LEVEL_WHEN_FULL,
    },
};

use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "traceroute")]
use sn_interface::messaging::Traceroute;
use sn_interface::{
    messaging::{
        data::StorageLevel,
        system::{JoinResponse, NodeCmd, NodeEvent, NodeMsg, NodeQuery, Proposal as ProposalMsg},
        MsgId,
    },
    network_knowledge::NetworkKnowledge,
    types::{log_markers::LogMarker, Keypair, Peer, PublicKey},
};

impl MyNode {
    /// Send a (`NodeMsg`) message to all Elders in our section
    pub(crate) fn send_msg_to_our_elders(&self, msg: NodeMsg) -> Cmd {
        let recipients = self.network_knowledge.elders();
        self.send_system_msg(msg, Peers::Multiple(recipients))
    }

    /// Send a (`NodeMsg`) message to a node
    pub(crate) fn send_system_msg(&self, msg: NodeMsg, recipients: Peers) -> Cmd {
        self.trace_system_msg(
            msg,
            recipients,
            #[cfg(feature = "traceroute")]
            Traceroute(vec![]),
        )
    }

    /// Send a (`NodeMsg`) message to a node
    pub(crate) fn trace_system_msg(
        &self,
        msg: NodeMsg,
        recipients: Peers,
        #[cfg(feature = "traceroute")] traceroute: Traceroute,
    ) -> Cmd {
        trace!("{}: {:?}", LogMarker::SendToNodes, msg);
        Cmd::send_traced_msg(
            OutgoingMsg::Node(msg),
            recipients,
            #[cfg(feature = "traceroute")]
            traceroute,
        )
    }

    // Handler for data messages which have successfully
    // passed all signature checks and msg verifications
    pub(crate) async fn handle_valid_system_msg(
        node: Arc<RwLock<MyNode>>,
        msg_id: MsgId,
        msg: NodeMsg,
        sender: Peer,
        comm: &Comm,
        #[cfg(feature = "traceroute")] traceroute: Traceroute,
    ) -> Result<Vec<Cmd>> {
        trace!("{:?}: {msg_id:?}", LogMarker::NodeMsgToBeHandled);

        #[cfg(feature = "traceroute")]
        {
            if !traceroute.0.is_empty() {
                info!(
                    "Handling NodeMsg {}:{:?} with trace \n{:?}",
                    msg, msg_id, traceroute
                );
            }
        }

        match msg {
            NodeMsg::Relocate(node_state) => {
                let mut node = node.write().await;

                trace!("Handling msg: Relocate from {}: {:?}", sender, msg_id);
                Ok(node
                    .handle_relocate(node_state)
                    .await?
                    .into_iter()
                    .collect())
            }
            NodeMsg::JoinAsRelocatedResponse(join_response) => {
                let mut node = node.write().await;
                trace!("Handling msg: JoinAsRelocatedResponse from {}", sender);
                if let Some(ref mut joining_as_relocated) = node.relocate_state {
                    if let Some(cmd) =
                        joining_as_relocated.handle_join_response(*join_response, sender.addr())?
                    {
                        return Ok(vec![cmd]);
                    }
                } else {
                    error!(
                        "No relocation in progress upon receiving {:?}",
                        join_response
                    );
                }

                Ok(vec![])
            }
            NodeMsg::AntiEntropy {
                section_tree_update,
                kind,
            } => {
                trace!("Handling msg: AE from {sender}: {msg_id:?}");
                let mut node = node.write().await;
                node.handle_anti_entropy_msg(
                    section_tree_update,
                    kind,
                    sender,
                    #[cfg(feature = "traceroute")]
                    traceroute,
                )
                .await
            }
            // Respond to a probe msg
            // We always respond to probe msgs if we're an elder as health checks use this to see if a node is alive
            // and repsonsive, as well as being a method of keeping nodes up to date.
            NodeMsg::AntiEntropyProbe(section_key) => {
                let node = node.read().await;
                let mut cmds = vec![];
                if !node.is_elder() {
                    // early return here as we do not get health checks as adults,
                    // normal AE rules should have applied
                    return Ok(cmds);
                }

                trace!("Received Probe message from {}: {:?}", sender, msg_id);
                let mut recipients = BTreeSet::new();
                let _existed = recipients.insert(sender);
                cmds.push(node.send_ae_update_to_nodes(recipients, section_key));
                Ok(cmds)
            }
            // The AcceptedOnlineShare for relocation will be received here.
            NodeMsg::JoinResponse(join_response) => {
                let mut node = node.write().await;
                match *join_response {
                    JoinResponse::Approved {
                        section_tree_update,
                        ..
                    } => {
                        info!(
                            "Relocation: Aggregating received ApprovalShare from {:?}",
                            sender
                        );
                        info!("Relocation: Successfully aggregated ApprovalShares for joining the network");

                        if let Some(ref mut joining_as_relocated) = node.relocate_state {
                            let new_node = joining_as_relocated.node.clone();
                            let new_name = new_node.name();
                            let previous_name = node.info().name();
                            let new_keypair = new_node.keypair.clone();

                            info!(
                                "Relocation: switching from {:?} to {:?}",
                                previous_name, new_name
                            );

                            let recipients: Vec<_> =
                                section_tree_update.signed_sap.elders().cloned().collect();

                            let section_tree = node.network_knowledge.section_tree().clone();
                            let new_network_knowledge =
                                NetworkKnowledge::new(section_tree, section_tree_update)?;

                            // TODO: confirm whether carry out the switch immediately here
                            //       or still using the cmd pattern.
                            //       As the sending of the JoinRequest as notification
                            //       may require the `node` to be switched to new already.

                            node.relocate(new_keypair.clone(), new_network_knowledge)?;

                            trace!(
                                "Relocation: Sending aggregated JoinRequest to {:?}",
                                recipients
                            );

                            node.send_event(Event::Membership(MembershipEvent::Relocated {
                                previous_name,
                                new_keypair,
                            }))
                            .await;

                            trace!("{}", LogMarker::RelocateEnd);
                        } else {
                            warn!("Relocation:  node.relocate_state is not in Progress");
                        }

                        Ok(vec![])
                    }
                    _ => {
                        debug!("Relocation: Ignoring unexpected join response message: {join_response:?}");
                        Ok(vec![])
                    }
                }
            }
            NodeMsg::HandoverVotes(votes) => {
                let mut node = node.write().await;

                node.handle_handover_msg(sender, votes).await
            }
            NodeMsg::HandoverAE(gen) => {
                let node = node.read().await;
                Ok(node
                    .handle_handover_anti_entropy(sender, gen)
                    .into_iter()
                    .collect())
            }
            NodeMsg::JoinRequest(join_request) => {
                trace!("Handling msg {:?}: JoinRequest from {}", msg_id, sender);
                MyNode::handle_join_request(node, sender, join_request)
                    .await
                    .map(|c| c.into_iter().collect())
            }
            NodeMsg::JoinAsRelocatedRequest(join_request) => {
                trace!("Handling msg: JoinAsRelocatedRequest from {}", sender);

                if node.read().await.is_not_elder()
                    && join_request.section_key == node.read().await.network_knowledge.section_key()
                {
                    return Ok(vec![]);
                }

                Ok(
                    MyNode::handle_join_as_relocated_request(node, sender, *join_request, comm)
                        .await
                        .into_iter()
                        .collect(),
                )
            }
            NodeMsg::MembershipVotes(votes) => {
                let mut node = node.write().await;
                let mut cmds = vec![];
                cmds.extend(node.handle_membership_votes(sender, votes)?);
                Ok(cmds)
            }
            NodeMsg::MembershipAE(gen) => {
                let node = node.read().await;
                Ok(node
                    .handle_membership_anti_entropy(sender, gen)
                    .into_iter()
                    .collect())
            }
            NodeMsg::Propose {
                proposal,
                sig_share,
            } => {
                let mut node = node.write().await;
                if node.is_not_elder() {
                    trace!("Adult handling a Propose msg from {}: {:?}", sender, msg_id);
                }

                trace!(
                    "Handling proposal msg: {proposal:?} from {}: {:?}",
                    sender,
                    msg_id
                );

                // lets convert our message into a usable proposal for core
                let core_proposal = match proposal {
                    ProposalMsg::VoteNodeOffline(node_state) => {
                        CoreProposal::VoteNodeOffline(node_state)
                    }
                    ProposalMsg::SectionInfo(sap) => CoreProposal::SectionInfo(sap),
                    ProposalMsg::NewElders(sap) => CoreProposal::NewElders(sap),
                    ProposalMsg::JoinsAllowed(allowed) => CoreProposal::JoinsAllowed(allowed),
                };

                node.handle_proposal(msg_id, core_proposal, sig_share, sender)
            }
            NodeMsg::DkgStart(session_id, elder_sig) => {
                trace!(
                    "Handling msg: DkgStart s{} {:?}: {} elders from {}",
                    session_id.sh(),
                    session_id.prefix,
                    session_id.elders.len(),
                    sender
                );

                let mut node = node.write().await;
                node.log_dkg_session(&sender.name());
                node.handle_dkg_start(session_id, elder_sig)
            }
            NodeMsg::DkgEphemeralPubKey {
                session_id,
                section_auth,
                pub_key,
                sig,
            } => {
                trace!(
                    "{} s{} from {}",
                    LogMarker::DkgHandleEphemeralPubKey,
                    session_id.sh(),
                    sender
                );
                let mut node = node.write().await;
                node.handle_dkg_ephemeral_pubkey(&session_id, section_auth, pub_key, sig, sender)
            }
            NodeMsg::DkgVotes {
                session_id,
                pub_keys,
                votes,
            } => {
                trace!(
                    "{} s{} from {}: {:?}",
                    LogMarker::DkgVotesHandling,
                    session_id.sh(),
                    sender,
                    votes
                );
                let mut node = node.write().await;
                node.log_dkg_session(&sender.name());
                node.handle_dkg_votes(&session_id, pub_keys, votes, sender)
            }
            NodeMsg::DkgAE(session_id) => {
                let node = node.read().await;
                trace!("Handling msg: DkgAE s{} from {}", session_id.sh(), sender);
                node.handle_dkg_anti_entropy(session_id, sender)
            }
            NodeMsg::NodeCmd(NodeCmd::RecordStorageLevel { node_name, level }) => {
                let mut node = node.write().await;
                let changed = node.set_storage_level(node_name, level);
                if changed && level.value() == MIN_LEVEL_WHEN_FULL {
                    // ..then we accept a new node in place of the full node
                    node.joins_allowed = true;
                }
                Ok(vec![])
            }
            NodeMsg::NodeCmd(NodeCmd::ReceiveMetadata { metadata }) => {
                let mut node = node.write().await;
                info!("Processing received MetadataExchange packet: {:?}", msg_id);
                node.set_adult_levels(metadata);
                Ok(vec![])
            }
            NodeMsg::NodeEvent(NodeEvent::CouldNotStoreData {
                node_name,
                data,
                full,
            }) => {
                let mut node = node.write().await;
                info!(
                    "Processing CouldNotStoreData event with MsgId: {:?}",
                    msg_id
                );
                if node.is_not_elder() {
                    error!("Received unexpected message while Adult");
                    return Ok(vec![]);
                }

                if full {
                    let changed =
                        node.set_storage_level(node_name, StorageLevel::from(StorageLevel::MAX)?);
                    if changed {
                        // ..then we accept a new node in place of the full node
                        node.joins_allowed = true;
                    }
                }

                let targets = node.target_data_holders(data.name());

                Ok(vec![node.replicate_data(
                    data,
                    targets,
                    #[cfg(feature = "traceroute")]
                    traceroute,
                )])
            }
            NodeMsg::NodeCmd(NodeCmd::ReplicateData(data_collection)) => {
                let node = node.read().await;

                if node.is_elder() {
                    error!(
                        "Received unexpected message while Elder, msg id: {:?}",
                        msg_id
                    );
                    return Ok(vec![]);
                }

                info!("ReplicateData MsgId: {:?}", msg_id);

                let section_pk = PublicKey::Bls(node.network_knowledge.section_key());
                let node_keypair = Keypair::Ed25519(node.keypair.clone());

                drop(node);

                let cmds = data_collection
                    .into_iter()
                    .map(|data| {
                        Cmd::Data(crate::data::Cmd::Store {
                            data,
                            section_pk,
                            node_keypair: node_keypair.clone(),
                        })
                    })
                    .collect();

                Ok(cmds)
            }
            NodeMsg::NodeCmd(NodeCmd::ReturnMissingData(known_data_addresses)) => {
                let node = node.read().await;

                info!(
                    "{:?} MsgId: {:?}",
                    LogMarker::RequestForAnyMissingData,
                    msg_id
                );
                Ok(vec![
                    node.get_missing_data_for_node(sender, known_data_addresses)
                ])
            }
            NodeMsg::NodeQuery(NodeQuery::Data {
                query,
                auth,
                operation_id,
            }) => {
                // A request from EndUser - via elders - for locally stored data
                debug!(
                    "Handle NodeQuery with msg_id {:?}, operation_id {}",
                    msg_id, operation_id
                );
                // The query is directed to the data sub module.
                // The response is later sent back to the requesting elder.
                Ok(vec![Cmd::Data(crate::data::Cmd::HandleQuery {
                    query,
                    auth, // There is no point in verifying a sig from a sender A or B here, but the auth is included anyway for now..
                    requesting_elder: sender,
                    operation_id,
                    #[cfg(feature = "traceroute")]
                    traceroute,
                })])
            }
            NodeMsg::NodeQueryResponse {
                response,
                operation_id,
            } => {
                let mut node = node.write().await;

                debug!(
                    "{:?}: op_id {}, sender: {sender} origin msg_id: {msg_id:?}",
                    LogMarker::ChunkQueryResponseReceviedFromAdult,
                    operation_id
                );

                Ok(node
                    .handle_query_response_at_elder(
                        operation_id,
                        response,
                        sender.name(),
                        #[cfg(feature = "traceroute")]
                        traceroute,
                    )
                    .await
                    .into_iter()
                    .collect())
            }
        }
    }
}
