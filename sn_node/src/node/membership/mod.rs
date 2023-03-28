// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.
use bls::{PublicKeySet, SecretKeyShare};
use core::fmt::Debug;
use sn_consensus::mvba::{
    bundle::{Bundle, Outgoing},
    consensus::Consensus,
    tag::Domain,
    Decision, NodeId,
};
use sn_interface::{
    messaging::system::DkgSessionId,
    network_knowledge::{
        node_state::MembershipProposal, partition_by_prefix, recommended_section_size,
        MembershipState, NodeState, SectionAuthorityProvider,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use std::{sync::Mutex, time::Instant};
use thiserror::Error;
use xor_name::{Prefix, XorName};

pub(crate) type Generation = u64;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Consensus error {0}")]
    Consensus(#[from] sn_consensus::mvba::error::Error),
    #[error("We are behind the voter, caller should request anti-entropy")]
    RequestAntiEntropy,
    #[error("Invalid proposal")]
    InvalidProposal,
    #[error("Invalid generation {0}")]
    InvalidGeneration(u64),
    #[error("Network Knowledge error {0:?}")]
    NetworkKnowledge(#[from] sn_interface::network_knowledge::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

fn get_split_info(
    prefix: Prefix,
    members: &BTreeMap<XorName, NodeState>,
) -> Option<(BTreeSet<NodeState>, BTreeSet<NodeState>)> {
    let (zero, one) = partition_by_prefix(&prefix, members.keys().copied())?;

    // make sure the sections contain enough entries
    let split_threshold = recommended_section_size();
    if zero.len() < split_threshold || one.len() < split_threshold {
        return None;
    }

    Some((
        BTreeSet::from_iter(zero.into_iter().map(|n| members[&n].clone())),
        BTreeSet::from_iter(one.into_iter().map(|n| members[&n].clone())),
    ))
}

/// Checks if we can split the section
/// If we have enough nodes for both subsections, returns the `DkgSessionId`'s
pub(crate) fn try_split_dkg(
    members: &BTreeMap<XorName, NodeState>,
    sap: &SectionAuthorityProvider,
    section_chain_len: u64,
    membership_gen: Generation,
) -> Option<(DkgSessionId, DkgSessionId)> {
    let prefix = sap.prefix();

    let (zero, one) = get_split_info(prefix, members)?;

    // get elders for section ...0
    let zero_prefix = prefix.pushed(false);
    let zero_elders = elder_candidates(zero.iter().cloned(), sap);

    // get elders for section ...1
    let one_prefix = prefix.pushed(true);
    let one_elders = elder_candidates(one.iter().cloned(), sap);

    // create the DKG session IDs
    let zero_id = DkgSessionId {
        prefix: zero_prefix,
        elders: BTreeMap::from_iter(zero_elders.iter().map(|node| (node.name(), node.addr()))),
        section_chain_len,
        bootstrap_members: zero,
        membership_gen,
    };
    let one_id = DkgSessionId {
        prefix: one_prefix,
        elders: BTreeMap::from_iter(one_elders.iter().map(|node| (node.name(), node.addr()))),
        section_chain_len,
        bootstrap_members: one,
        membership_gen,
    };

    Some((zero_id, one_id))
}

/// Returns the nodes that should be candidates to become the next elders, sorted by names.
pub(crate) fn elder_candidates(
    candidates: impl IntoIterator<Item = NodeState>,
    current_elders: &SectionAuthorityProvider,
) -> BTreeSet<NodeState> {
    use itertools::Itertools;
    use std::cmp::Ordering;

    // Compare candidates for the next elders. The one comparing `Less` wins.
    fn cmp_elder_candidates(
        lhs: &NodeState,
        rhs: &NodeState,
        current_elders: &SectionAuthorityProvider,
    ) -> Ordering {
        // Older nodes are preferred. In case of a tie, prefer current elders. If still a tie, break
        // it comparing by the signed signatures because it's impossible for a node to predict its
        // signature and therefore game its chances of promotion.
        rhs.age()
            .cmp(&lhs.age())
            .then_with(|| {
                let lhs_is_elder = current_elders.contains_elder(&lhs.name());
                let rhs_is_elder = current_elders.contains_elder(&rhs.name());

                match (lhs_is_elder, rhs_is_elder) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => Ordering::Equal,
                }
            })
            .then_with(|| lhs.name().cmp(&rhs.name()))
        // TODO: replace name cmp above with sig cmp.
        // .then_with(|| lhs.sig.signature.cmp(&rhs.sig.signature))
    }

    candidates
        .into_iter()
        .sorted_by(|lhs, rhs| cmp_elder_candidates(lhs, rhs, current_elders))
        .take(sn_interface::elder_count())
        .collect()
}

// 1- Proposal is a `NodeState`
// 2- Define Decision in sn_consensus
// 3- We can define Generic for proposal in Consensus<T>
//       * We don't need Ser/Des
//

#[derive(Clone)]
pub(crate) struct Membership {
    consensus: Arc<Mutex<Consensus<MembershipProposal>>>,
    bootstrap_members: BTreeSet<NodeState>,
    pub(crate) gen: Generation, // current generation
    history: BTreeMap<Generation, Decision<MembershipProposal>>,
    // last membership vote timestamp
    last_received_vote_time: Option<Instant>,
    outgoings: Vec<Outgoing<MembershipProposal>>,
}

fn checker(_: NodeId, _: &MembershipProposal) -> bool {
    // We need to pass current state:
    //   1- Clone: 3rd  argument as Any
    //   2- Closure: To not pass 3rd argument?
    //   3- Generic: 3rd  argument as generic

    // cast any to something that possible to cast
    true
}

impl Membership {
    pub(crate) fn from(
        secret_key: (NodeId, SecretKeyShare),
        elders: PublicKeySet,
        n_elders: usize,
        bootstrap_members: BTreeSet<NodeState>,
    ) -> Self {
        trace!("Membership - Creating new membership instance");
        let domain = Domain::new("membership", 0);
        let mut elders_id = Vec::new();
        for i in 0..n_elders {
            elders_id.push(i);
        }

        let consensus = Arc::new(Mutex::new(Consensus::init(
            domain,
            secret_key.0,
            secret_key.1,
            elders,
            elders_id,
            checker,
        )));
        Membership {
            consensus,
            bootstrap_members,
            gen: 0,
            history: BTreeMap::default(),
            last_received_vote_time: None,
            outgoings: Vec::new(),
        }
    }

    pub(crate) fn section_key_set(&self) -> PublicKeySet {
        self.consensus.lock().unwrap().pub_key_set() // TODO: no unwrap
    }

    pub(crate) fn last_received_vote_time(&self) -> Option<Instant> {
        self.last_received_vote_time
    }

    pub(crate) fn generation(&self) -> Generation {
        self.gen
    }

    #[cfg(test)]
    pub(crate) fn is_churn_in_progress(&self) -> bool {
        self.consensus.lock().unwrap().decided_proposal().is_none() // TODO: no unwrap
    }

    #[cfg(test)]
    pub(crate) fn force_bootstrap(&mut self, state: NodeState) {
        let _ = self.bootstrap_members.insert(state);
    }

    // fn consensus_at_gen(&self, gen: Generation) -> Result<&Consensus<NodeState>> {
    //     if gen == self.gen + 1 {
    //         Ok(&self.consensus)
    //     } else {
    //         self.history
    //             .get(&gen)
    //             .map(|(_, c)| c)
    //             .ok_or(Error::Consensus(sn_consensus::Error::BadGeneration {
    //                 requested_gen: gen,
    //                 gen: self.gen,
    //             }))
    //     }
    // }

    // fn consensus_at_gen_mut(&mut self, gen: Generation) -> Result<&mut Consensus<NodeState>> {
    //     if gen == self.gen + 1 {
    //         Ok(&mut self.consensus)
    //     } else {
    //         self.history
    //             .get_mut(&gen)
    //             .map(|(_, c)| c)
    //             .ok_or(Error::Consensus(sn_consensus::Error::BadGeneration {
    //                 requested_gen: gen,
    //                 gen: self.gen,
    //             }))
    //     }
    // }

    pub(crate) fn archived_members(&self) -> BTreeSet<XorName> {
        let mut members = BTreeSet::from_iter(
            self.bootstrap_members
                .iter()
                .filter(|n| {
                    matches!(
                        n.state(),
                        MembershipState::Left | MembershipState::Relocated(..)
                    )
                })
                .map(|n| n.name()),
        );

        for decision in self.history.values() {
            let node_state = &decision.proposal;
            match node_state.state() {
                MembershipState::Joined => {
                    continue;
                }
                MembershipState::Left | MembershipState::Relocated(_) => {
                    let _ = members.insert(node_state.name());
                }
            }
        }

        members
    }

    /// get only section members reporting Joined till gen
    fn section_members(&self, gen: Generation) -> Result<BTreeMap<XorName, NodeState>> {
        let mut members = BTreeMap::from_iter(
            self.bootstrap_members
                .iter()
                .cloned()
                .filter(|n| matches!(n.state(), MembershipState::Joined))
                .map(|n| (n.name(), n)),
        );

        if gen == 0 {
            return Ok(members);
        }

        for (history_gen, decision) in &self.history {
            let node_state = &decision.proposal.1;
            match node_state.state() {
                MembershipState::Joined => {
                    let _ = members.insert(node_state.name(), node_state.clone());
                }
                MembershipState::Left => {
                    let _ = members.remove(&node_state.name());
                }
                MembershipState::Relocated(_) => {
                    let _ = members.remove(&node_state.name());
                }
            }

            if history_gen == &gen {
                return Ok(members);
            }
        }

        Err(Error::InvalidGeneration(gen))
    }

    pub(crate) fn propose(
        &mut self,
        node_state: NodeState,
        prefix: &Prefix,
    ) -> Result<Vec<Outgoing<MembershipProposal>>> {
        let proposal = MembershipProposal(self.gen + 1, node_state);
        self.validate_proposals(&proposal, prefix)?;

        let outgoings = self.consensus.lock().unwrap().propose(proposal)?;
        self.outgoings.append(&mut outgoings.clone());

        Ok(outgoings)
    }

    pub(crate) fn anti_entropy(
        &self,
        _from_gen: Generation,
    ) -> Option<Outgoing<MembershipProposal>> {
        if self.outgoings.is_empty() {
            return None;
        }
        let index: usize = rand::random();
        let outgoing = self.outgoings[index].clone();

        Some(outgoing)
    }

    #[allow(dead_code)]
    pub(crate) fn id(&self) -> NodeId {
        self.consensus.lock().unwrap().self_id()
    }

    pub(crate) fn handle_signed_vote(
        &mut self,
        bundle: Bundle<MembershipProposal>,
        _prefix: &Prefix,
    ) -> Result<(
        Vec<Outgoing<MembershipProposal>>,
        Option<Decision<MembershipProposal>>,
    )> {
        let outgoings = self.consensus.lock().unwrap().process_bundle(&bundle)?;
        self.outgoings.append(&mut outgoings.clone());

        let decision_opt = self.consensus.lock().unwrap().decided_proposal();
        if let Some(decision) = &decision_opt {
            info!(
                "Membership - updated generation from {:?} to {:?}",
                self.gen,
                decision.proposal.gen()
            );

            self.gen = decision.proposal.gen()
        }

        Ok((outgoings, decision_opt))
    }

    /// Returns true if the proposal is valid
    fn validate_proposals(&self, proposal: &MembershipProposal, prefix: &Prefix) -> Result<()> {
        if proposal.gen() != self.gen + 1 {
            return Err(Error::InvalidGeneration(proposal.gen()));
        }
        let members = BTreeMap::from_iter(self.section_members(proposal.gen() - 1)?.into_iter());
        let archived_members = self.archived_members();

        if let Err(err) =
            proposal
                .node_state()
                .validate_node_state(prefix, &members, &archived_members)
        {
            warn!("Failed to validate {proposal:?} with error {:?}", err);
            // TODO: certain errors need AE?
            warn!(
                "Members at generation {} are: {:?}",
                proposal.gen() - 1,
                members
            );
            warn!("Archived members are {:?}", archived_members);
            return Err(Error::NetworkKnowledge(err));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // use super::Error;
    // use crate::node::flow_ctrl::tests::network_builder::TestNetworkBuilder;
    // use sn_interface::{
    //     network_knowledge::NodeState,
    //     test_utils::{gen_node_id, TestSapBuilder},
    // };

    // use assert_matches::assert_matches;
    // use eyre::Result;
    // use rand::thread_rng;
    // use xor_name::Prefix;

    // #[tokio::test]
    // async fn multiple_proposals_in_a_single_generation_should_not_be_possible() -> Result<()> {
    //     let prefix = Prefix::default();
    //     let env = TestNetworkBuilder::new(thread_rng())
    //         .sap(TestSapBuilder::new(prefix))
    //         .build()?;

    //     let mut membership = env
    //         .get_nodes(prefix, 1, 0, None)?
    //         .remove(0)
    //         .membership
    //         .expect("Membership for the elder should've been initialized");

    //     let state1 = NodeState::joined(gen_node_id(5), None);
    //     let state2 = NodeState::joined(gen_node_id(5), None);

    //     let _ = membership.propose(state1, &prefix)?;
    //     assert_matches!(
    //         membership.propose(state2, &prefix),
    //         Err(Error::InvalidProposal)
    //     );

    //     Ok(())
    // }
}
