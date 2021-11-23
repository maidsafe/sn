// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use xor_name::{XorName, XOR_NAME_LEN};

/// Information about a member of our section.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Debug)]
pub struct NodeState {
    /// Peer's name.
    pub name: XorName,
    /// Peer's address.
    pub addr: SocketAddr,
    /// Current state of the peer
    pub state: MembershipState,
    /// To avoid sybil attack via relocation, a relocated node's original name will be recorded.
    pub previous_name: Option<XorName>,
}

impl NodeState {
    /// Returns the age.
    pub fn age(&self) -> u8 {
        self.name[XOR_NAME_LEN - 1]
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, Debug)]
/// Node's current section membership state
pub enum MembershipState {
    /// Node is active member of the section.
    Joined,
    /// Node went offline.
    Left,
    /// Node was relocated to a different section.
    Relocated(XorName),
}
