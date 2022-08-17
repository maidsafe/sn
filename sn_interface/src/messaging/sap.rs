// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use bls::PublicKeySet;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use sn_consensus::Generation;
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Display, Formatter},
    net::SocketAddr,
};
use xor_name::{Prefix, XorName};

use super::system::NodeState;

// TODO: we need to maintain a list of nodes who have previosly been members of this section (archived nodes)
//       currently, only the final members of the section are preserved on the SAP.

/// Details of section authority.
///
/// A new `SectionAuthorityProvider` is created whenever the elders change, due to an elder being
/// added or removed, or the section splitting or merging.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize)]
pub struct SectionAuthorityProvider {
    /// The section prefix. It matches all the members' names.
    pub prefix: Prefix,
    /// Public key set of the section.
    pub public_key_set: PublicKeySet,
    /// The section's complete set of elders as a map from their name to their socket address.
    pub elders: BTreeMap<XorName, SocketAddr>,
    /// The section members at the time of this elder churn.
    pub members: BTreeSet<NodeState>,
    /// The membership generation this SAP was instantiated on
    pub membership_gen: Generation,
}

impl Borrow<Prefix> for SectionAuthorityProvider {
    fn borrow(&self) -> &Prefix {
        &self.prefix
    }
}

impl Display for SectionAuthorityProvider {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "sap len:{} generation:{} contains: {{{}}}/({:b})",
            self.elders.len(),
            self.membership_gen,
            self.elders.keys().format(", "),
            self.prefix,
        )
    }
}
