// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod candidates;
mod node_state;
mod peer;

pub use candidates::ElderCandidates;
pub use node_state::MembershipState;
pub use node_state::NodeState;
pub use peer::Peer;

use crate::messaging::{system::agreement::SectionAuth, SectionAuthorityProvider};
use bls::PublicKey as BlsPublicKey;
use secured_linked_list::SecuredLinkedList;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use dashmap::DashMap;
use std::sync::Arc;
use xor_name::XorName;

/// Container for storing information about a section.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// All information about a section
pub struct Section {
    /// Network genesis key
    pub genesis_key: BlsPublicKey,
    /// The secured linked list of previous section keys
    pub chain: SecuredLinkedList,
    /// Signed section authority
    pub section_auth: SectionAuth<SectionAuthorityProvider>,
    /// Members of the section
    pub section_peers: SectionPeers,
}

/// Container for storing information about members of our section.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SectionPeers {
    /// Members of the section
    pub members: Arc<DashMap<XorName, SectionAuth<NodeState>>>,
}

impl Eq for SectionPeers {}

impl PartialEq for SectionPeers {
    fn eq(&self, _other: &Self) -> bool {
        // TODO: there must be a better way of doing this...
        let mut us: BTreeMap<XorName, SectionAuth<NodeState>> = BTreeMap::default();
        let mut them: BTreeMap<XorName, SectionAuth<NodeState>> = BTreeMap::default();

        for refmulti in self.members.iter() {
            let (key, value) = refmulti.pair();
            let _ = us.insert(*key, value.clone());
        }

        for refmulti in self.members.iter() {
            let (key, value) = refmulti.pair();
            let _ = them.insert(*key, value.clone());
        }

        us == them
    }
}
