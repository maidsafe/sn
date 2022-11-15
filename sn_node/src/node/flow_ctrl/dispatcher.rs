// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{
    messaging::{OutgoingMsg, Peers},
    Cmd, Error, MyNode, Result,
};

use sn_interface::{
    messaging::{Dst, MsgId, MsgKind, WireMsg},
    network_knowledge::SectionTreeUpdate,
    types::Peer,
};

use qp2p::UsrMsgBytes;

use bytes::Bytes;
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::RwLock;

// Cmd Dispatcher.
pub(crate) struct Dispatcher {
    node: Arc<RwLock<MyNode>>,
}

impl Dispatcher {
    pub(crate) fn new(node: Arc<RwLock<MyNode>>) -> Self {
        Self { node }
    }

    pub(crate) fn node(&self) -> Arc<RwLock<MyNode>> {
        self.node.clone()
    }

    /// Handles a single cmd.
    pub(crate) async fn process_cmd(&self, cmd: Cmd) -> Result<Vec<Cmd>> {
        trace!("doing actual processing cmd: {cmd:?}");

        match cmd {
            Cmd::SendMsg {
                msg,
                msg_id,
                recipients,
                send_stream,
            } => {
                trace!("Sending msg: {msg_id:?}");
                let peer_msgs = {
                    let node = self.node.read().await;
                    into_msg_bytes(&node, msg, msg_id, recipients)?
                };

                let comm = self.node.read().await.comm.clone();

                let tasks = peer_msgs
                    .into_iter()
                    .map(|(peer, msg)| comm.send_out_bytes(peer, msg_id, msg, send_stream.clone()));
                let results = futures::future::join_all(tasks).await;

                // Any failed sends are tracked via Cmd::HandlePeerFailedSend, which will log dysfunction for any peers
                // in the section (otherwise ignoring failed send to out of section nodes or clients)
                let cmds = results
                    .into_iter()
                    .filter_map(|result| match result {
                        Err(Error::FailedSend(peer)) => {
                            Some(Cmd::HandleFailedSendToNode { peer, msg_id })
                        }
                        _ => None,
                    })
                    .collect();

                Ok(cmds)
            }
            Cmd::TrackNodeIssueInDysfunction { name, issue } => {
                let mut node = self.node.write().await;
                node.log_node_issue(name, issue);
                Ok(vec![])
            }
            Cmd::ValidateMsg {
                origin,
                wire_msg,
                send_stream,
            } => {
                debug!("validatinge msg: {:?}", wire_msg.msg_id());
                let node = self.node.read().await;
                debug!("got node read lock for: {:?}", wire_msg.msg_id());
                node.validate_msg(origin, wire_msg, send_stream).await
            }
            Cmd::UpdateNetworkAndHandleValidClientMsg {
                proof_chain,
                signed_sap,
                msg_id,
                msg,
                origin,
                auth,
                send_stream,
            } => {
                debug!("Updating network knowledge before handling message");
                let mut node = self.node.write().await;
                let name = node.name();
                let updated = node.network_knowledge.update_knowledge_if_valid(
                    SectionTreeUpdate::new(signed_sap, proof_chain),
                    None,
                    &name,
                )?;
                // drop the write lock
                drop(node);

                let node = self.node.read().await;

                debug!("Network knowledge was updated: {updated}");
                node.handle_valid_client_msg(msg_id, msg, auth, origin, send_stream)
                    .await
            }
            Cmd::HandleValidNodeMsg {
                origin,
                msg_id,
                msg,
                send_stream,
            } => {
                debug!("handling valid msg {:?}", msg_id);
                MyNode::handle_valid_system_msg(self.node.clone(), msg_id, msg, origin, send_stream)
                    .await
            }
            Cmd::HandleAgreement { proposal, sig } => {
                let mut node = self.node.write().await;
                node.handle_general_agreements(proposal, sig)
            }
            Cmd::HandleMembershipDecision(decision) => {
                let mut node = self.node.write().await;
                node.handle_membership_decision(decision)
            }
            Cmd::HandleNewEldersAgreement { new_elders, sig } => {
                let mut node = self.node.write().await;
                node.handle_new_elders_agreement(new_elders, sig)
            }
            Cmd::HandleFailedSendToNode { peer, msg_id } => {
                warn!("Message sending failed to {peer}, for {msg_id:?}");
                let mut node = self.node.write().await;
                node.handle_failed_send(&peer.addr());
                Ok(vec![])
            }
            Cmd::HandleDkgOutcome {
                section_auth,
                outcome,
            } => {
                let mut node = self.node.write().await;
                node.handle_dkg_outcome(section_auth, outcome)
            }
            Cmd::EnqueueDataForReplication {
                // throttle_duration,
                recipient,
                data_batch,
            } => {
                // we should queue this
                for data in data_batch {
                    trace!("data being enqueued for replication {:?}", data);
                    let mut node = self.node.write().await;
                    if let Some(peers_set) = node.pending_data_to_replicate_to_peers.get_mut(&data)
                    {
                        debug!("data already queued, adding peer");
                        let _existed = peers_set.insert(recipient);
                    } else {
                        let mut peers_set = BTreeSet::new();
                        let _existed = peers_set.insert(recipient);
                        let _existed = node
                            .pending_data_to_replicate_to_peers
                            .insert(data, peers_set);
                    };
                }
                Ok(vec![])
            }
            Cmd::ProposeVoteNodesOffline(names) => {
                let mut node = self.node.write().await;
                node.cast_offline_proposals(&names)
            }
        }
    }
}

// Serializes and signs the msg if it's a Client message,
// and produces one [`WireMsg`] instance per recipient -
// the last step before passing it over to comms module.
fn into_msg_bytes(
    node: &MyNode,
    msg: OutgoingMsg,
    msg_id: MsgId,
    recipients: Peers,
) -> Result<Vec<(Peer, UsrMsgBytes)>> {
    let (kind, payload) = match msg {
        OutgoingMsg::Node(msg) => node.serialize_node_msg(msg)?,
    };
    let recipients = match recipients {
        Peers::Single(peer) => vec![peer],
        Peers::Multiple(peers) => peers.into_iter().collect(),
    };
    // we first generate the XorName
    let dst = Dst {
        name: xor_name::rand::random(),
        section_key: bls::SecretKey::random().public_key(),
    };

    let mut initial_wire_msg = wire_msg(msg_id, payload, kind, dst);

    let _bytes = initial_wire_msg.serialize_and_cache_bytes()?;

    let mut msgs = vec![];
    for peer in recipients {
        match node.network_knowledge.generate_dst(&peer.name()) {
            Ok(dst) => {
                // TODO log errror here isntead of throwing
                let all_the_bytes = initial_wire_msg.serialize_with_new_dst(&dst)?;
                msgs.push((peer, all_the_bytes));
            }
            Err(error) => {
                error!("Could not get route for {peer:?}: {error}");
            }
        }
    }

    Ok(msgs)
}

fn wire_msg(msg_id: MsgId, payload: Bytes, auth: MsgKind, dst: Dst) -> WireMsg {
    #[allow(unused_mut)]
    let mut wire_msg = WireMsg::new_msg(msg_id, payload, auth, dst);

    #[cfg(feature = "test-utils")]
    let wire_msg = wire_msg.set_payload_debug(msg);
    wire_msg
}
