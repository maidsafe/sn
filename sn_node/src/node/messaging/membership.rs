// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{
    flow_ctrl::cmds::Cmd, membership, relocation::ChurnId, Event, MembershipEvent, Node, Result,
};

use bls::Signature;
use sn_consensus::{Decision, Generation, SignedVote, VoteResponse};
use sn_interface::{
    messaging::system::{
        JoinResponse, KeyedSig, MembershipState, NodeState, SectionAuth, SystemMsg,
    },
    network_knowledge::MIN_ADULT_AGE,
    types::{log_markers::LogMarker, Peer},
};

use std::{collections::BTreeSet, vec};

// Message handling
impl Node {
    pub(crate) fn propose_membership_change(&mut self, node_state: NodeState) -> Result<Vec<Cmd>> {
        info!(
            "Proposing membership change: {} - {:?}",
            node_state.name, node_state.state
        );
        let prefix = self.network_knowledge.prefix();
        if let Some(membership) = self.membership.as_mut() {
            let membership_vote = match membership.propose(node_state, &prefix) {
                Ok(vote) => vote,
                Err(e) => {
                    warn!("Membership - failed to propose change: {e:?}");
                    return Ok(vec![]);
                }
            };

            let cmds =
                self.send_msg_to_our_elders(SystemMsg::MembershipVotes(vec![membership_vote]))?;
            Ok(vec![cmds])
        } else {
            error!("Membership - Failed to propose membership change, no membership instance");
            Ok(vec![])
        }
    }

    /// Get our latest vote if any at this generation, and get cmds to resend to all elders
    /// (which should in turn trigger them to resend their votes)
    #[instrument(skip_all)]
    pub(crate) async fn resend_our_last_vote_to_elders(&self) -> Result<Vec<Cmd>> {
        let membership = self.membership.clone();

        if let Some(membership) = membership {
            if let Some(prev_vote) = membership.get_our_latest_vote() {
                trace!("{}", LogMarker::ResendingLastMembershipVote);
                let cmds = self
                    .send_msg_to_our_elders(SystemMsg::MembershipVotes(vec![prev_vote.clone()]))?;
                return Ok(vec![cmds]);
            }
        }

        Ok(vec![])
    }

    pub(crate) fn handle_membership_votes(
        &mut self,
        peer: Peer,
        signed_votes: Vec<SignedVote<NodeState>>,
    ) -> Result<Vec<Cmd>> {
        debug!(
            "{:?} {signed_votes:?} from {peer}",
            LogMarker::MembershipVotesBeingHandled
        );
        let prefix = self.network_knowledge.prefix();

        let mut cmds = vec![];

        for signed_vote in signed_votes {
            let mut vote_broadcast = None;
            if let Some(membership) = self.membership.as_mut() {
                let (vote_response, decision) = match membership
                    .handle_signed_vote(signed_vote, &prefix)
                {
                    Ok(result) => result,
                    Err(membership::Error::RequestAntiEntropy) => {
                        debug!("Membership - We are behind the voter, requesting AE");
                        // We hit an error while processing this vote, perhaps we are missing information.
                        // We'll send a membership AE request to see if they can help us catch up.
                        let sap = self.network_knowledge.authority_provider();
                        let dst_section_pk = sap.section_key();
                        let section_name = prefix.name();
                        let msg = SystemMsg::MembershipAE(membership.generation());
                        let cmd = self.send_direct_msg_to_nodes(
                            vec![peer],
                            msg,
                            section_name,
                            dst_section_pk,
                        )?;

                        debug!("{:?}", LogMarker::MembershipSendingAeUpdateRequest);
                        cmds.push(cmd);
                        // return the vec w/ the AE cmd there so as not to loop and generate AE for
                        // any subsequent commands
                        return Ok(cmds);
                    }
                    Err(e) => {
                        error!("Membership - error while processing vote {e:?}, dropping this and all votes in this batch thereafter");
                        break;
                    }
                };

                match vote_response {
                    VoteResponse::Broadcast(response_vote) => {
                        vote_broadcast = Some(SystemMsg::MembershipVotes(vec![response_vote]));
                    }
                    VoteResponse::WaitingForMoreVotes => {
                        // do nothing
                    }
                };

                if let Some(decision) = decision {
                    // process the membership change
                    debug!(
                        "Handling Membership Decision {:?}",
                        BTreeSet::from_iter(decision.proposals.keys())
                    );
                    let public_key = self.network_knowledge.section_key();
                    for (value, signature) in decision.proposals.clone() {
                        let sig = KeyedSig {
                            public_key,
                            signature,
                        };
                        if membership.is_leaving_section(&value, prefix) {
                            cmds.push(Cmd::HandleNodeLeft(SectionAuth { value, sig }));
                        }
                    }

                    cmds.push(Cmd::HandleJoinDecision(decision));
                }
            } else {
                error!(
                    "Attempted to handle membership vote when we don't yet have a membership instance"
                );
            };

            if let Some(vote_msg) = vote_broadcast {
                cmds.push(self.send_msg_to_our_elders(vote_msg)?);
            }
        }

        Ok(cmds)
    }

    pub(crate) fn handle_membership_anti_entropy(
        &self,
        peer: Peer,
        gen: Generation,
    ) -> Result<Vec<Cmd>> {
        debug!(
            "{:?} membership anti-entropy request for gen {:?} from {}",
            LogMarker::MembershipAeRequestReceived,
            gen,
            peer
        );

        let cmds = if let Some(membership) = self.membership.as_ref() {
            match membership.anti_entropy(gen) {
                Ok(catchup_votes) => {
                    vec![self.send_direct_msg(
                        peer,
                        SystemMsg::MembershipVotes(catchup_votes),
                        self.network_knowledge.section_key(),
                    )?]
                }
                Err(e) => {
                    error!("Membership - Error while processing anti-entropy {:?}", e);
                    vec![]
                }
            }
        } else {
            error!(
                "Attempted to handle membership anti-entropy when we don't yet have a membership instance"
            );
            vec![]
        };

        Ok(cmds)
    }

    pub(crate) async fn handle_join_decision(
        &mut self,
        decision: Decision<NodeState>,
    ) -> Result<Vec<Cmd>> {
        debug!("{}", LogMarker::AgreementOfOnline);
        let mut cmds = vec![];

        cmds.extend(self.send_node_approval(decision.clone()));
        let joining_nodes = Vec::from_iter(
            decision
                .proposals
                .clone()
                .into_iter()
                .filter(|(n, _)| n.state == MembershipState::Joined),
        );

        for (new_info, signature) in joining_nodes.iter().cloned() {
            cmds.extend(self.handle_joining_node(new_info, signature).await?);
        }

        self.log_section_stats();

        // Do not disable node joins in first section.
        let our_prefix = self.network_knowledge.prefix();
        if !our_prefix.is_empty() {
            // ..otherwise, switch off joins_allowed on a node joining.
            // TODO: fix racing issues here? https://github.com/maidsafe/safe_network/issues/890
            self.joins_allowed = false;
        }

        if let Some((_, sig)) = joining_nodes.iter().max_by_key(|(_, sig)| sig) {
            let churn_id = ChurnId(sig.to_bytes().to_vec());
            let excluded_from_relocation =
                BTreeSet::from_iter(joining_nodes.iter().map(|(n, _)| n.name));

            cmds.extend(self.relocate_peers(churn_id, excluded_from_relocation)?);
        }

        let result = self.promote_and_demote_elders_except(&BTreeSet::default())?;

        // TODO: this should move into the `promote_and_demote_elders_except()` call
        if result.is_empty() {
            // Send AE-Update to our section
            cmds.extend(self.send_ae_update_to_our_section());
        }

        cmds.extend(result);

        info!("cmds in queue for Accepting node {:?}", cmds);

        self.print_network_stats();

        Ok(cmds)
    }

    async fn handle_joining_node(
        &mut self,
        new_info: NodeState,
        signature: Signature,
    ) -> Result<Vec<Cmd>> {
        if let Some(old_info) = self
            .network_knowledge
            .is_either_member_or_archived(&new_info.name)
        {
            // We would approve and relocate it only if half its age is at least MIN_ADULT_AGE
            let new_age = old_info.age() / 2;
            if new_age >= MIN_ADULT_AGE {
                return self.relocate_rejoining_peer(old_info.value, new_age);
            }
        }

        let sig = KeyedSig {
            public_key: self.network_knowledge.section_key(),
            signature,
        };

        let new_info = SectionAuth {
            value: new_info.into_state(),
            sig,
        };

        if !self.network_knowledge.update_member(new_info.clone()) {
            info!("ignore Online: {}", new_info.peer());
            return Ok(vec![]);
        }

        self.add_new_adult_to_trackers(new_info.name());

        info!("handle Online: {}", new_info.peer());

        // still used for testing
        self.send_event(Event::Membership(MembershipEvent::MemberJoined {
            name: new_info.name(),
            previous_name: new_info.previous_name(),
            age: new_info.age(),
        }))
        .await;

        Ok(vec![])
    }

    // Send `NodeApproval` to a joining node which makes it a section member
    pub(crate) fn send_node_approval(&self, decision: Decision<NodeState>) -> Vec<Cmd> {
        let peers = Vec::from_iter(
            decision
                .proposals
                .keys()
                .filter(|n| n.state == MembershipState::Joined)
                .map(|n| n.peer()),
        );
        let prefix = self.network_knowledge.prefix();
        info!("Section {prefix:?} has approved new peers {peers:?}.");

        let node_msg = SystemMsg::JoinResponse(Box::new(JoinResponse::Approval {
            genesis_key: *self.network_knowledge.genesis_key(),
            section_auth: self
                .network_knowledge
                .section_signed_authority_provider()
                .into_authed_msg(),
            section_chain: self.network_knowledge.section_chain(),
            decision,
        }));

        let sap = self.network_knowledge.authority_provider();
        let dst_section_pk = sap.section_key();
        let section_name = sap.prefix().name();

        trace!("{}", LogMarker::SendNodeApproval);
        match self.send_direct_msg_to_nodes(peers.clone(), node_msg, section_name, dst_section_pk) {
            Ok(cmd) => vec![cmd],
            Err(err) => {
                error!("Failed to send join approval to new peers {peers:?}: {err:?}");
                vec![]
            }
        }
    }
}
