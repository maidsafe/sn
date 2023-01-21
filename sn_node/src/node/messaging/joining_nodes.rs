// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{
    core::NodeContext, flow_ctrl::cmds::Cmd, messaging::Peers, Error, MyNode, Result,
};

use sn_interface::{
    messaging::system::{JoinRejectReason, JoinResponse, NodeMsg},
    network_knowledge::{NodeState, RelocationProof, MIN_ADULT_AGE},
    types::{log_markers::LogMarker, Peer},
};

use std::sync::Arc;
use tokio::sync::RwLock;

// Message handling
impl MyNode {
    pub(crate) async fn handle_join(
        node: Arc<RwLock<MyNode>>,
        context: &NodeContext,
        peer: Peer,
        relocation: Option<RelocationProof>,
    ) -> Result<Option<Cmd>> {
        debug!("Handling join from {peer:?}");

        // Ignore a join request if we are not elder.
        if !context.is_elder {
            warn!("Join request received to our section, but I am not an elder...");
            // Note: We don't bounce this message because the current bounce-resend
            // mechanism wouldn't preserve the original SocketAddr which is needed for
            // properly handling this message.
            // This is OK because in the worst case the join request just timeouts and the
            // joining node sends it again.
            return Ok(None);
        }
        let our_prefix = context.network_knowledge.prefix();
        if !our_prefix.matches(&peer.name()) {
            debug!("Unreachable path; {peer} name doesn't match our prefix. Should be covered by AE. Dropping the msg.");
            return Ok(None);
        }

        let previous_name = if let Some(proof) = relocation {
            // Relocation validation

            // verify that we know the src key
            let src_key = proof.signed_by();
            if !context
                .network_knowledge
                .verify_section_key_is_known(src_key)
            {
                warn!("Peer {} is trying to join with signature by unknown source section key {src_key:?}. Message is dropped.", peer.name());
                return Ok(None);
            }
            // verify the signatures
            proof.verify()?;

            // verify the age
            let name = peer.name();
            let peer_age = peer.age();
            let previous_age = proof.previous_age();

            // We require node name to match the relocation proof age.
            // Which is one less than the age within the relocation proof.
            if peer_age != previous_age.saturating_add(1) {
                info!(
        		    "Invalid relocation from {name} - peer new age ({peer_age}) should be one more than peer's previous age ({previous_age}), or same if {}.", u8::MAX
        		);
                return Err(Error::InvalidRelocationDetails);
            }

            // Finally do reachability check
            // NB: This can be moved out of this clause to also apply to new nodes.
            if context.comm.is_reachable(&peer.addr()).await.is_err() {
                let msg = NodeMsg::JoinResponse(JoinResponse::Rejected(
                    JoinRejectReason::NodeNotReachable(peer.addr()),
                ));
                trace!(
                    "Relocation reachability check, sending {:?} to {}",
                    msg,
                    peer
                );
                return Ok(Some(Cmd::send_msg(
                    msg,
                    Peers::Single(peer),
                    context.clone(),
                )));
            };

            Some(proof.previous_name())
        } else {
            // infant node validation
            if !MyNode::verify_infant_node_age(&peer) {
                debug!("Unreachable path; {peer} age is invalid: {}. This should be a hard coded value in join logic. Dropping the msg.", peer.age());
                return Ok(None);
            }

            if !context.joins_allowed {
                debug!("Rejecting join request from {peer} - joins currently not allowed.");
                let msg = NodeMsg::JoinResponse(JoinResponse::Rejected(
                    JoinRejectReason::JoinsDisallowed,
                ));
                trace!("{}", LogMarker::SendJoinRejected);
                trace!("Sending {msg:?} to {peer}");
                return Ok(Some(Cmd::send_msg(
                    msg,
                    Peers::Single(peer),
                    context.clone(),
                )));
            }

            None
        };

        // NB: No reachability check has been made here
        // We propose membership
        let node_state = NodeState::joined(peer, previous_name);

        debug!("[NODE WRITE]: join propose membership write...");
        let mut node = node.write().await;
        debug!("[NODE WRITE]: join propose membership write gottt...");
        Ok(node.propose_membership_change(node_state))
    }

    pub(crate) fn verify_infant_node_age(peer: &Peer) -> bool {
        // Age should be MIN_ADULT_AGE for joining infant.
        peer.age() == MIN_ADULT_AGE
    }
}
