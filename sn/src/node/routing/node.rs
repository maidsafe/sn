// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::peer::Peer;
use crate::types::PublicKey;
use ed25519_dalek::Keypair;
use std::{
    fmt::{self, Display, Formatter},
    net::SocketAddr,
    sync::Arc,
};
use xor_name::{XorName, XOR_NAME_LEN};

/// Information and state of our node
#[derive(Clone, custom_debug::Debug)]
pub(crate) struct Node {
    // Keep the secret key in Arc to allow Clone while also preventing multiple copies to exist in
    // memory which might be insecure.
    // TODO: find a way to not require `Clone`.
    #[debug(skip)]
    pub(crate) keypair: Arc<Keypair>,
    pub(crate) addr: SocketAddr,
}

impl Node {
    pub(crate) fn new(keypair: Keypair, addr: SocketAddr) -> Self {
        Self {
            keypair: Arc::new(keypair),
            addr,
        }
    }

    pub(crate) fn peer(&self) -> Peer {
        Peer::new(self.name(), self.addr)
    }

    pub(crate) fn name(&self) -> XorName {
        XorName::from(PublicKey::from(self.keypair.public))
    }

    // Last byte of the name represents the age.
    pub(crate) fn age(&self) -> u8 {
        self.name()[XOR_NAME_LEN - 1]
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;
    use crate::node::routing::ed25519;
    use itertools::Itertools;
    use proptest::{collection::SizeRange, prelude::*};

    pub(crate) fn arbitrary_node() -> impl Strategy<Value = Node> {
        (
            ed25519::test_utils::arbitrary_keypair(),
            any::<SocketAddr>(),
        )
            .prop_map(|(keypair, addr)| Node::new(keypair, addr))
    }

    // Generate Vec<Node> where no two nodes have the same name.
    pub(crate) fn arbitrary_unique_nodes(
        count: impl Into<SizeRange>,
    ) -> impl Strategy<Value = Vec<Node>> {
        proptest::collection::vec(arbitrary_node(), count).prop_filter("non-unique keys", |nodes| {
            nodes
                .iter()
                .unique_by(|node| node.keypair.secret.as_bytes())
                .unique_by(|node| node.addr)
                .count()
                == nodes.len()
        })
    }
}
