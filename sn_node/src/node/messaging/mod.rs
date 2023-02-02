// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod anti_entropy;
mod client_msgs;
mod data;
mod dkg;
mod handover;
mod join_section;
mod joining_nodes;
mod membership;
pub(crate) mod node_msgs;
mod promotion;
mod relocation;
mod section_state;
mod serialize;
mod signature;
mod streams;
mod update_section;

use crate::node::{flow_ctrl::cmds::Cmd, Error, MyNode, Result};

use qp2p::SendStream;
use sn_interface::{
    messaging::{MsgType, WireMsg},
    types::Peer,
};

use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::RwLock;

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

impl IntoIterator for Peers {
    type Item = Peer;

    type IntoIter = Box<dyn Iterator<Item = Self::Item>>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Peers::Single(p) => Box::new(std::iter::once(p)),
            Peers::Multiple(ps) => Box::new(ps.into_iter()),
        }
    }
}

// Message handling
impl MyNode {
    #[instrument(skip(node))]
    pub(crate) async fn handle_msg(
        node: Arc<RwLock<MyNode>>,
        origin: Peer,
        wire_msg: WireMsg,
        send_stream: Option<SendStream>,
    ) -> Result<Vec<Cmd>> {
        let msg_id = wire_msg.msg_id();
        let msg_kind = wire_msg.kind();

        trace!("Handling msg {msg_id:?}. Checking for AE first...");

        let context = node.read().await.context();
        trace!("[NODE READ]: Handle msg read lock attempt success");

        // first check for AE, if this isn't an ae msg itself
        if !msg_kind.is_ae_msg() {
            let entropy =
                MyNode::check_for_entropy(&wire_msg, &context.network_knowledge, &origin)?;
            if let Some((update, ae_kind)) = entropy {
                debug!("bailing early, AE found for {msg_id:?}");
                return MyNode::generate_anti_entropy_cmds(
                    &context,
                    &wire_msg,
                    origin,
                    update,
                    ae_kind,
                    send_stream,
                );
            }
        }

        // Deserialize the payload of the incoming message
        let msg_type = match wire_msg.into_msg() {
            Ok(msg_type) => msg_type,
            Err(error) => {
                error!("Failed to deserialize message payload ({msg_id:?}): {error:?}");
                return Ok(vec![]);
            }
        };

        match msg_type {
            MsgType::Node { msg, .. } => {
                MyNode::handle_valid_node_msg(node, context, msg_id, msg, origin, send_stream).await
            }
            MsgType::Client {
                msg_id, msg, auth, ..
            } => {
                debug!("Valid client msg {msg_id:?}");

                let Some(stream) = send_stream else {
                    return Err(Error::NoClientResponseStream);
                };

                MyNode::handle_valid_client_msg(context, msg_id, msg, auth, origin, stream).await
            }
            MsgType::NodeDataResponse { msg, .. } => {
                let mut node = node.write().await;
                debug!("[NODE WRITE]: handle_msg NodeDataResponse write got");
                Ok(node
                    .update_data_response(origin, msg)
                    .await?
                    .into_iter()
                    .collect())
            }
            other @ MsgType::ClientDataResponse { .. } => {
                error!(
                    "Client data response {msg_id:?}, from {}, has been dropped since it's not \
                    meant to be handled by a node: {other:?}",
                    origin.addr()
                );
                Ok(vec![])
            }
        }
    }

    /// Utility to split a list of peers between others and ourself
    pub(crate) fn split_peers_and_self(
        &self,
        all_peers: Vec<Peer>,
    ) -> (BTreeSet<Peer>, Option<Peer>) {
        let our_name = self.info().name();
        let (peers, ourself): (BTreeSet<_>, BTreeSet<_>) = all_peers
            .into_iter()
            .partition(|peer| peer.name() != our_name);
        let optional_self = ourself.into_iter().next();
        (peers, optional_self)
    }
}
