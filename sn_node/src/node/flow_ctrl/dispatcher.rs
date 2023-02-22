// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{messaging::Peers, Cmd, Error, MyNode, Result, STANDARD_CHANNEL_SIZE};

use sn_interface::{
    messaging::{AntiEntropyMsg, NetworkMsg},
    network_knowledge::SectionTreeUpdate,
    types::{DataAddress, Peer},
};

use std::{sync::Arc, time::Instant};
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    RwLock,
};

// Cmd Dispatcher.
pub(crate) struct Dispatcher {
    node: Arc<RwLock<MyNode>>,
    data_replication_sender: Sender<(Vec<DataAddress>, Peer)>,
}

impl Dispatcher {
    /// Creates dispatcher and returns a receiver for enqueing DataAddresses for replication to specific peers
    pub(crate) fn new(node: Arc<RwLock<MyNode>>) -> (Self, Receiver<(Vec<DataAddress>, Peer)>) {
        let (data_replication_sender, data_replication_receiver) = channel(STANDARD_CHANNEL_SIZE);
        let dispatcher = Self {
            node,
            data_replication_sender,
        };

        (dispatcher, data_replication_receiver)
    }

    pub(crate) fn node(&self) -> Arc<RwLock<MyNode>> {
        self.node.clone()
    }

    /// Handles a single cmd.
    pub(crate) async fn process_cmd(&self, cmd: Cmd) -> Result<Vec<Cmd>> {
        let start = Instant::now();
        let cmd_string = format!("{cmd}");
        let result = match cmd {
            Cmd::TryJoinNetwork => {
                info!("[NODE READ]: getting lock for try_join_section");
                let context = self.node().read().await.context();
                info!("[NODE READ]: got lock for try_join_section");
                Ok(MyNode::try_join_section(context, None)
                    .into_iter()
                    .collect())
            }
            Cmd::UpdateCaller {
                caller,
                correlation_id,
                kind,
                section_tree_update,
                context,
            } => {
                info!("Sending ae response msg for {correlation_id:?}");
                Ok(vec![Cmd::send_network_msg(
                    NetworkMsg::AntiEntropy(AntiEntropyMsg::AntiEntropy {
                        section_tree_update,
                        kind,
                    }),
                    Peers::Single(caller),
                    context,
                )])
            }
            Cmd::UpdateCallerOnStream {
                caller,
                msg_id,
                kind,
                section_tree_update,
                correlation_id,
                stream,
                context,
            } => Ok(MyNode::send_ae_response(
                AntiEntropyMsg::AntiEntropy {
                    kind,
                    section_tree_update,
                },
                msg_id,
                caller,
                correlation_id,
                stream,
                context,
            )
            .await?
            .into_iter()
            .collect()),
            Cmd::SendMsg {
                msg,
                msg_id,
                recipients,
                context,
            } => {
                MyNode::send_msg(msg, msg_id, recipients, context)?;
                Ok(vec![])
            }
            Cmd::SendMsgEnqueueAnyResponse {
                msg,
                msg_id,
                recipients,
                context,
            } => {
                MyNode::send_and_enqueue_any_response(msg, msg_id, context, recipients)?;
                Ok(vec![])
            }
            Cmd::SendAndForwardResponseToClient {
                wire_msg,
                context,
                targets,
                client_stream,
                source_client,
            } => {
                MyNode::send_and_forward_response_to_client(
                    wire_msg,
                    context,
                    targets,
                    client_stream,
                    source_client,
                )?;
                Ok(vec![])
            }
            Cmd::SendNodeMsgResponse {
                msg,
                msg_id,
                correlation_id,
                recipient,
                send_stream,
                context,
            } => Ok(MyNode::send_node_msg_response(
                msg,
                msg_id,
                correlation_id,
                recipient,
                context,
                send_stream,
            )
            .await?
            .into_iter()
            .collect()),
            Cmd::SendDataResponse {
                msg,
                msg_id,
                correlation_id,
                send_stream,
                context,
                source_client,
            } => Ok(MyNode::send_data_response(
                msg,
                msg_id,
                correlation_id,
                send_stream,
                context,
                source_client,
            )
            .await?
            .into_iter()
            .collect()),
            Cmd::TrackNodeIssue { name, issue } => {
                let node = self.node.read().await;
                trace!("[NODE READ]: fault tracking read got");
                node.track_node_issue(name, issue);
                Ok(vec![])
            }
            Cmd::HandleMsg {
                origin,
                wire_msg,
                send_stream,
            } => MyNode::handle_msg(self.node.clone(), origin, wire_msg, send_stream).await,
            Cmd::UpdateNetworkAndHandleValidClientMsg {
                proof_chain,
                signed_sap,
                msg_id,
                msg,
                origin,
                auth,
                send_stream,
                context,
            } => {
                debug!("Updating network knowledge before handling message");
                // we create a block to make sure the node's lock is released
                let updated = {
                    let mut node = self.node.write().await;
                    let name = node.name();
                    trace!("[NODE WRITE]: update client write got");
                    node.network_knowledge.update_knowledge_if_valid(
                        SectionTreeUpdate::new(signed_sap, proof_chain),
                        None,
                        &name,
                    )?
                };
                info!("Network knowledge was updated: {updated}");

                MyNode::handle_client_msg_for_us(context, msg_id, msg, auth, origin, send_stream)
                    .await
            }
            Cmd::HandleSectionDecisionAgreement { proposal, sig } => {
                trace!("[NODE WRITE]: section decision agreements node write...");
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: section decision agreements node write got");
                node.handle_section_decision_agreement(proposal, sig)
            }
            Cmd::HandleMembershipDecision(decision) => {
                trace!("[NODE WRITE]: membership decision agreements write...");
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: membership decision agreements write got...");
                node.handle_membership_decision(decision).await
            }
            Cmd::HandleNewEldersAgreement { new_elders, sig } => {
                trace!("[NODE WRITE]: new elders decision agreements write...");
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: new elders decision agreements write got...");
                node.handle_new_elders_agreement(new_elders, sig).await
            }
            Cmd::HandleNewSectionsAgreement {
                sap1,
                sig1,
                sap2,
                sig2,
            } => {
                trace!("[NODE WRITE]: new sections decision agreements write...");
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: new sections decision agreements write got...");
                node.handle_new_sections_agreement(sap1, sig1, sap2, sig2)
                    .await
            }
            Cmd::HandleCommsError { peer, error } => {
                trace!("Comms error {error}");
                let node = self.node.read().await;
                debug!("[NODE READ]: HandleCommsError read got...");
                node.handle_comms_error(peer, error);
                Ok(vec![])
            }
            Cmd::HandleDkgOutcome {
                section_auth,
                outcome,
            } => {
                trace!("[NODE WRITE]: HandleDKg agreements write...");
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: HandleDKg agreements write got...");
                node.handle_dkg_outcome(section_auth, outcome).await
            }
            Cmd::EnqueueDataForReplication {
                recipient,
                data_batch,
            } => {
                self.data_replication_sender
                    .send((data_batch, recipient))
                    .await
                    .map_err(|_| Error::DataReplicationChannel)?;
                Ok(vec![])
            }
            Cmd::ProposeVoteNodesOffline(names) => {
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: propose offline write got");
                node.cast_offline_proposals(&names)
            }
            Cmd::SetJoinsAllowed(joins_allowed) => {
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: Setting joins allowed..");
                node.joins_allowed = joins_allowed;
                Ok(vec![])
            }
            Cmd::SetJoinsAllowedUntilSplit(joins_allowed_until_split) => {
                let mut node = self.node.write().await;
                trace!("[NODE WRITE]: Setting joins allowed until split..");
                node.joins_allowed = joins_allowed_until_split;
                node.joins_allowed_until_split = joins_allowed_until_split;
                Ok(vec![])
            }
        };

        let elapsed = start.elapsed();
        trace!("Cmd {cmd_string:?} took {:?}", elapsed);

        result
    }
}
