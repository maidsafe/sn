// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::comm::Comm;
use crate::node::{
    messaging::{OutgoingMsg, Peers},
    Cmd, Error, MyNode, Result,
};

#[cfg(feature = "traceroute")]
use sn_interface::{messaging::Entity, messaging::Traceroute};
use sn_interface::{
    messaging::{AuthKind, Dst, MsgId, SectionTreeUpdate, WireMsg},
    types::Peer,
};

use qp2p::UsrMsgBytes;

use bytes::Bytes;
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::RwLock;

// Cmd Dispatcher.
pub(crate) struct Dispatcher {
    node: Arc<RwLock<MyNode>>,
    comm: Comm,
}

impl Dispatcher {
    pub(crate) fn new(node: Arc<RwLock<MyNode>>, comm: Comm) -> Self {
        Self { node, comm }
    }

    pub(crate) fn node(&self) -> Arc<RwLock<MyNode>> {
        self.node.clone()
    }

    /// Handles a single cmd.
    pub(crate) async fn process_cmd(&self, cmd: Cmd) -> Result<Vec<Cmd>> {
        match cmd {
            Cmd::CleanupPeerLinks => {
                let members = { self.node.read().await.network_knowledge.section_members() };
                self.comm.cleanup_peers(members).await;
                Ok(vec![])
            }
            Cmd::SendMsg {
                msg,
                msg_id,
                recipients,
                #[cfg(feature = "traceroute")]
                traceroute,
            } => {
                // ClientMsgs are only used for the communication between Client and Elders
                let is_msg_for_client = matches!(msg, OutgoingMsg::Client(_));

                trace!("Sending msg: {msg_id:?}");
                let peer_msgs = {
                    let node = self.node.read().await;
                    into_msg_bytes(
                        &node,
                        msg,
                        msg_id,
                        recipients,
                        #[cfg(feature = "traceroute")]
                        traceroute,
                    )?
                };

                let tasks = peer_msgs
                    .into_iter()
                    .map(|(peer, msg)| self.comm.send(peer, msg_id, msg));
                let results = futures::future::join_all(tasks).await;

                // Any failed sends are tracked via Cmd::HandlePeerFailedSend, which will log dysfunction for any peers
                // in the section (otherwise ignoring failed send to out of section nodes or clients)
                let cmds = results
                    .into_iter()
                    .filter_map(|result| match result {
                        Err(Error::FailedSend(peer)) => {
                            if is_msg_for_client {
                                warn!("Client msg send failed to: {peer}, for {msg_id:?}");
                                None
                            } else {
                                Some(Cmd::HandleFailedSendToNode { peer, msg_id })
                            }
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
            Cmd::AddToPendingQueries {
                operation_id,
                origin,
                target_adult,
            } => {
                let mut node = self.node.write().await;
                // cleanup
                node.pending_data_queries.remove_expired();

                if let Some(peers) = node
                    .pending_data_queries
                    .get_mut(&(operation_id, origin.name()))
                {
                    trace!(
                        "Adding to pending data queries for op id: {:?}",
                        operation_id
                    );
                    let _ = peers.insert(origin);
                } else {
                    let _prior_value = node.pending_data_queries.set(
                        (operation_id, target_adult),
                        BTreeSet::from([origin]),
                        None,
                    );
                };

                Ok(vec![])
            }
            Cmd::ValidateMsg { origin, wire_msg } => {
                let node = self.node.read().await;
                node.validate_msg(origin, wire_msg).await
            }
            Cmd::HandleValidClientMsg {
                msg_id,
                msg,
                origin,
                auth,
                #[cfg(feature = "traceroute")]
                traceroute,
            } => {
                let node = self.node.read().await;
                match node
                    .handle_valid_client_msg(
                        msg_id,
                        msg,
                        auth,
                        origin,
                        #[cfg(feature = "traceroute")]
                        traceroute.clone(),
                    )
                    .await
                {
                    Ok(cmds) => Ok(cmds),
                    Err(err) => {
                        debug!("Will send error response back to client");
                        let cmd = node.cmd_error_response(
                            err,
                            origin,
                            msg_id,
                            #[cfg(feature = "traceroute")]
                            traceroute,
                        );
                        Ok(vec![cmd])
                    }
                }
            }
            Cmd::UpdateNetworkAndHandleValidClientMsg {
                proof_chain,
                signed_sap,
                msg_id,
                msg,
                origin,
                auth,
                #[cfg(feature = "traceroute")]
                traceroute,
            } => {
                debug!("Updating network knowledge before handling message");
                let mut node = self.node.write().await;
                let name = node.name();
                let updated = node.network_knowledge.update_knowledge_if_valid(
                    SectionTreeUpdate::new(signed_sap, proof_chain),
                    None,
                    &name,
                )?;
                debug!("Network knowledge was updated: {updated}");
                node.handle_valid_client_msg(
                    msg_id,
                    msg,
                    auth,
                    origin,
                    #[cfg(feature = "traceroute")]
                    traceroute,
                )
                .await
            }
            Cmd::HandleValidNodeMsg {
                origin,
                msg_id,
                msg,
                msg_authority,
                wire_msg_payload,
                #[cfg(feature = "traceroute")]
                traceroute,
            } => {
                debug!("init of handling valid msg {:?}", msg_id);
                let mut node = self.node.write().await;

                if let Some(msg_authority) = node
                    .aggregate_system_msg(msg_id, msg_authority, wire_msg_payload)
                    .await
                {
                    debug!("handling valid msg {:?}", msg_id);
                    node.handle_valid_system_msg(
                        msg_id,
                        msg_authority,
                        msg,
                        origin,
                        &self.comm,
                        #[cfg(feature = "traceroute")]
                        traceroute.clone(),
                    )
                    .await
                } else {
                    Ok(vec![])
                }
            }
            Cmd::HandleAgreement { proposal, sig } => {
                let mut node = self.node.write().await;
                node.handle_general_agreements(proposal, sig)
                    .await
                    .map(|c| c.into_iter().collect())
            }
            Cmd::HandleMembershipDecision(decision) => {
                let mut node = self.node.write().await;
                node.handle_membership_decision(decision).await
            }
            Cmd::HandleNewEldersAgreement { new_elders, sig } => {
                let mut node = self.node.write().await;
                node.handle_new_elders_agreement(new_elders, sig).await
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
                node.handle_dkg_outcome(section_auth, outcome).await
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

// Serializes and signs the msg,
// and produces one [`WireMsg`] instance per recipient -
// the last step before passing it over to comms module.
fn into_msg_bytes(
    node: &MyNode,
    msg: OutgoingMsg,
    msg_id: MsgId,
    recipients: Peers,
    #[cfg(feature = "traceroute")] traceroute: Traceroute,
) -> Result<Vec<(Peer, UsrMsgBytes)>> {
    let (auth, payload) = node.sign_msg(msg)?;
    let recipients = match recipients {
        Peers::Single(peer) => vec![peer],
        Peers::Multiple(peers) => peers.into_iter().collect(),
    };
    // we first generate the XorName
    let dst = Dst {
        name: xor_name::rand::random(),
        section_key: bls::SecretKey::random().public_key(),
    };

    #[cfg(feature = "traceroute")]
    let trace = Trace {
        entity: node.identity(),
        traceroute,
    };

    let mut initial_wire_msg = wire_msg(
        msg_id,
        payload,
        auth,
        dst,
        #[cfg(feature = "traceroute")]
        trace,
    );

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

#[cfg(feature = "traceroute")]
struct Trace {
    entity: Entity,
    traceroute: Traceroute,
}

fn wire_msg(
    msg_id: MsgId,
    payload: Bytes,
    auth: AuthKind,
    dst: Dst,
    #[cfg(feature = "traceroute")] trace: Trace,
) -> WireMsg {
    #[allow(unused_mut)]
    let mut wire_msg = WireMsg::new_msg(msg_id, payload, auth, dst);
    #[cfg(feature = "traceroute")]
    {
        let mut traceroute = trace.traceroute;
        traceroute.0.push(trace.entity);
        wire_msg.append_trace(&mut traceroute);
    }
    #[cfg(feature = "test-utils")]
    let wire_msg = wire_msg.set_payload_debug(msg);
    wire_msg
}
