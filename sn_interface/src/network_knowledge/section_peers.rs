// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::system::{MembershipState, SectionAuth};
use crate::network_knowledge::{errors::Result, NodeState, SectionsDAG};

use dashmap::{mapref::entry::Entry, DashMap};
use std::{collections::BTreeSet, sync::Arc};
use xor_name::{Prefix, XorName};

// Number of Elder churn events before a Left/Relocated member
// can be removed from the section members archive.
const ELDER_CHURN_EVENTS_TO_PRUNE_ARCHIVE: usize = 5;

/// Container for storing information about (current and archived) members of our section.
#[derive(Clone, Default, Debug)]
pub(super) struct SectionPeers {
    members: Arc<DashMap<XorName, SectionAuth<NodeState>>>,
    archive: Arc<DashMap<XorName, SectionAuth<NodeState>>>,
}

impl SectionPeers {
    /// Returns set of current members, i.e. those with state == `Joined`.
    pub(super) fn members(&self) -> BTreeSet<SectionAuth<NodeState>> {
        self.members
            .iter()
            .map(|entry| {
                let (_, state) = entry.pair();
                state.clone()
            })
            .collect()
    }

    /// Returns the number of current members.
    pub(super) fn num_of_members(&self) -> usize {
        self.members.len()
    }

    /// Get the `NodeState` for the member with the given name.
    pub(super) fn get(&self, name: &XorName) -> Option<NodeState> {
        self.members.get(name).map(|state| state.value.clone())
    }

    /// Returns whether the given peer is currently a member of our section.
    pub(super) fn is_member(&self, name: &XorName) -> bool {
        self.members.get(name).is_some()
    }

    /// Get section signed `NodeState` for the member with the given name.
    pub(super) fn is_either_member_or_archived(
        &self,
        name: &XorName,
    ) -> Option<SectionAuth<NodeState>> {
        if let Some(member) = self.members.get(name).map(|state| state.value().clone()) {
            Some(member)
        } else {
            self.archive.get(name).map(|state| state.value().clone())
        }
    }

    /// Update a member of our section.
    /// Returns whether anything actually changed.
    /// To maintain commutativity, the only allowed transitions are:
    /// - Joined -> Joined if the new age is greater than the old age
    /// - Joined -> Left
    /// - Joined -> Relocated
    /// - Relocated <--> Left (should not happen, but needed for consistency)
    pub(super) fn update(&self, new_state: SectionAuth<NodeState>) -> bool {
        let node_name = new_state.name();
        // do ops on the dashmap _after_ matching, so we can drop any refs to prevent deadlocking
        let mut should_insert = false;
        let mut should_remove = false;

        let updating_something = match (self.members.entry(node_name), new_state.state()) {
            (Entry::Vacant(_entry), MembershipState::Joined) => {
                // unless it was already archived, insert it as current member
                if self.archive.get(&node_name).is_none() {
                    should_insert = true;
                    true
                } else {
                    false
                }
            }
            (Entry::Vacant(_), MembershipState::Left | MembershipState::Relocated(_)) => {
                // insert it in our archive regardless it was there with another state
                let _prev = self.archive.insert(node_name, new_state.clone());
                true
            }
            (Entry::Occupied(entry), MembershipState::Joined)
                if new_state.age() > entry.get().age() =>
            {
                should_insert = true;
                true
            }
            (Entry::Occupied(_), MembershipState::Joined) => false,
            (Entry::Occupied(_entry), MembershipState::Left | MembershipState::Relocated(_)) => {
                //  remove it from our current members, and insert it into our archive
                should_remove = true;
                let _prev = self.archive.insert(node_name, new_state.clone());
                true
            }
        };

        // now we have dropped the entry ref
        if should_insert {
            let _prev = self.members.insert(node_name, new_state);
        }
        if should_remove {
            let _prev = self.members.remove(&node_name);
        }

        updating_something
    }

    /// Remove all members whose name does not match `prefix`.
    pub(super) fn retain(&self, prefix: &Prefix) {
        self.members.retain(|name, _| prefix.matches(name))
    }

    // Remove any member which Left, or was Relocated, more
    // than ELDER_CHURN_EVENTS_TO_PRUNE_ARCHIVE section keys ago from `last_key`
    pub(super) fn prune_members_archive(
        &self,
        proof_chain: &SectionsDAG,
        last_key: &bls::PublicKey,
    ) -> Result<()> {
        let mut last_section_keys = proof_chain.get_ancestors(last_key)?;
        last_section_keys.truncate(ELDER_CHURN_EVENTS_TO_PRUNE_ARCHIVE - 1);
        last_section_keys.push(*last_key);
        self.archive
            .retain(|_, node_state| last_section_keys.contains(&node_state.sig.public_key));
        Ok(())
    }
}
