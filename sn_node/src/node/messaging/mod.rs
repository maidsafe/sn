// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod agreement;
mod anti_entropy;
mod client_msgs;
mod dkg;
mod handover;
mod join;
mod membership;
mod node_msgs;
mod proposal;
mod relocation;
mod serialize;
mod update_section;

use crate::node::{flow_ctrl::cmds::Cmd, Error, MyNode, Result};

use qp2p::SendStream;
use sn_interface::{
    messaging::{data::ClientMsg, system::NodeMsg, Dst, MsgType, WireMsg},
    types::Peer,
};

use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum OutgoingMsg {
    Node(NodeMsg),
}

#[derive(Debug, Clone)]
pub(crate) enum Peers {
    Single(Peer),
    Multiple(BTreeSet<Peer>),
}

impl Peers {
    #[allow(unused)]
    pub(crate) fn get(&self) -> BTreeSet<Peer> {
        match self {
            Self::Single(peer) => BTreeSet::from([*peer]),
            Self::Multiple(peers) => peers.clone(),
        }
    }
}

// Message handling
impl MyNode {
    #[instrument(skip(self))]
    pub(crate) async fn validate_msg(
        &self,
        origin: Peer,
        wire_msg: WireMsg,
        send_stream: Option<Arc<Mutex<SendStream>>>,
    ) -> Result<Vec<Cmd>> {
        // Deserialize the payload of the incoming message
        let msg_id = wire_msg.msg_id();
        debug!("validating msg {msg_id:?}");

        let msg_type = match wire_msg.into_msg() {
            Ok(msg_type) => msg_type,
            Err(error) => {
                error!(
                    "Failed to deserialize message payload ({:?}): {:?}",
                    msg_id, error
                );
                return Ok(vec![]);
            }
        };

        match msg_type {
            MsgType::Node { msg_id, dst, msg } => {
                // Check for entropy before we proceed further
                // Anythign returned here means there's an issue and we should
                // short-circuit below
                let ae_cmds = self
                    .apply_ae(&origin, &msg, &wire_msg, &dst, send_stream.clone())
                    .await?;

                if !ae_cmds.is_empty() {
                    // short circuit and send those AE responses
                    return Ok(ae_cmds);
                }

                // this needs write access...
                Ok(vec![Cmd::HandleValidNodeMsg {
                    origin,
                    msg_id,
                    msg,
                    send_stream,
                }])
            }
            MsgType::Client {
                msg_id,
                msg,
                dst,
                auth,
            } => {
                debug!("valid client msg");

                let Some(send_stream) = send_stream else {
                    return Err(Error::NoClientResponseStream)
                };

                if self.is_not_elder() {
                    trace!("Redirecting from Adult to section Elders");
                    self.ae_redirect_client_to_our_elders(
                        origin,
                        send_stream,
                        wire_msg.serialize()?,
                    )
                    .await?;
                    return Ok(vec![]);
                }

                // We shall perform AE checks only if this is a query coming from the client,
                // if it's otherwise a response for a client we shall skip drop it.
                let dst_name = match &msg {
                    ClientMsg::Cmd(cmd) => cmd.dst_name(),

                    ClientMsg::Query(query) => query.variant.dst_name(),
                    other => {
                        error!(
                            "Client msg {:?}, from {}, has been dropped since it's not meant \
                            to be handled by a node: {:?}",
                            msg_id,
                            origin.addr(),
                            other
                        );
                        return Ok(vec![]);
                    }
                };

                // Then we perform AE checks
                // TODO: make entropy check for client specifically w/r/t response stream
                if let Some(cmd) = self.check_for_entropy(
                    &wire_msg,
                    &dst.section_key,
                    dst_name,
                    &origin,
                    Some(send_stream.clone()),
                )? {
                    // short circuit and send those AE responses
                    return Ok(vec![cmd]);
                }

                debug!("no aeeee");
                self.handle_valid_client_msg(msg_id, msg, auth, origin, send_stream.clone())
                    .await
            }
        }
    }

    /// Check if the origin needs to be updated on network structure/members.
    /// Returns an ae cmd if we need to halt msg validation and update the origin instead.
    #[instrument(skip_all)]
    async fn apply_ae(
        &self,
        origin: &Peer,
        msg: &NodeMsg,
        wire_msg: &WireMsg,
        dst: &Dst,
        send_stream: Option<Arc<Mutex<SendStream>>>,
    ) -> Result<Vec<Cmd>> {
        // Adult nodes don't need to carry out entropy checking,
        // however the message shall always be handled.
        if !self.is_elder() {
            return Ok(vec![]);
        }
        // For the case of receiving a join request not matching our prefix,
        // we just let the join request handler to deal with it later on.
        // We also skip AE check on Anti-Entropy messages
        //
        // TODO: consider changing the join and "join as relocated" flows to
        // make use of AntiEntropy retry/redirect responses.
        match msg {
            NodeMsg::AntiEntropy { .. }
            | NodeMsg::JoinRequest(_)
            | NodeMsg::JoinAsRelocatedRequest(_) => {
                trace!(
                    "Entropy check skipped for {:?}, handling message directly",
                    wire_msg.msg_id()
                );
                Ok(vec![])
            }
            _ => {
                debug!("Checking {:?} for entropy", wire_msg.msg_id());
                if let Some(ae_cmd) = self.check_for_entropy(
                    wire_msg,
                    &dst.section_key,
                    dst.name,
                    origin,
                    send_stream,
                )? {
                    // we want to log issues with any node repeatedly out of sync here...
                    let cmds = vec![
                        Cmd::TrackNodeIssueInDysfunction {
                            name: origin.name(),
                            issue: sn_dysfunction::IssueType::Knowledge,
                        },
                        ae_cmd,
                    ];

                    return Ok(cmds);
                }

                trace!(
                    "Entropy check passed. Handling verified msg {:?}",
                    wire_msg.msg_id()
                );

                Ok(vec![])
            }
        }
    }
}
