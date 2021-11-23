// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use std::{
    cmp::Ordering,
    fmt::{self, Display, Formatter},
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::Arc,
};
use tokio::sync::RwLock;
use xor_name::{XorName, XOR_NAME_LEN};

/// Network peer identity.
///
/// When a node knows another node as a `Peer` it's logically connected to it. This is separate from
/// being physically connected at the network layer, which is indicated by the optional `connection`
/// field.
#[derive(Clone)]
pub struct Peer {
    name: XorName,
    addr: SocketAddr,

    // An existing connection to the peer. There are no guarantees about the state of the connection
    // except that it once connected to this peer's `addr` (e.g. it may already be closed or
    // otherwise unusable).
    connection: Arc<RwLock<Option<qp2p::Connection>>>,
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let guard = self.connection.try_read();
        let connection; // needed to avoid issues with temporary bindings
        f.debug_struct("Peer")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field(
                "connection",
                // It's likely that the lock will be free, so attempt to read without blocking in
                // order to show more useful info.
                match &guard {
                    Ok(guard) => {
                        connection = guard.as_ref();
                        &connection
                    }
                    Err(_) => &"<locked>",
                },
            )
            .finish()
    }
}

impl Display for Peer {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{} at {} ({})",
            self.name,
            self.addr,
            match self.connection.try_read() {
                Ok(guard) => guard
                    .as_ref()
                    .map(|_| "connected")
                    .unwrap_or("not connected"),
                Err(_) => "<locked>",
            }
        )
    }
}

impl Eq for Peer {}

impl Hash for Peer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.addr.hash(state);
    }
}

impl Ord for Peer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name
            .cmp(&other.name)
            .then_with(|| self.addr.cmp(&other.addr))
    }
}

impl PartialEq for Peer {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.addr == other.addr
    }
}

impl PartialEq<&Self> for Peer {
    fn eq(&self, other: &&Self) -> bool {
        self.name == other.name && self.addr == other.addr
    }
}

impl PartialOrd for Peer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Peer {
    /// Creates a new `Peer` given `Name`, `SocketAddr`.
    pub fn new(name: XorName, addr: SocketAddr) -> Self {
        Self {
            name,
            addr,
            connection: Arc::new(RwLock::new(None)),
        }
    }

    /// Returns the `XorName` of the peer.
    pub fn name(&self) -> XorName {
        self.name
    }

    /// Returns the `SocketAddr`.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns the age.
    pub fn age(&self) -> u8 {
        self.name[XOR_NAME_LEN - 1]
    }

    /// Get the existing connection to the peer, if any.
    pub(crate) async fn connection(&self) -> Option<qp2p::Connection> {
        self.connection.read().await.as_ref().cloned()
    }

    /// Set the connection to the peer.
    ///
    /// If there's an existing connection, it is overwritten and dropped, so ideally this should
    /// only be performed if the existing connection is known to be lost (we cannot test that
    /// without using the connection).
    pub(crate) async fn set_connection(&self, connection: qp2p::Connection) {
        let _existing = self.connection.write().await.replace(connection);
    }
}

/// A peer whose name we don't yet know.
///
/// An `UnnamedPeer` represents a connected peer when we don't yet know the peer's name. For
/// example, when we receive a message we don't know the identity of the sender.
///
/// One rough edge to this is that `UnnamedPeer` is also used to represent messages from "ourself".
/// in this case there is no physical connection, and we technically would know our own identity.
/// It's possible that we're self-sending at the wrong level of the API (e.g. we currently serialise
/// and self-deliver a `WireMsg`, when we could instead directly generate the appropriate
/// `Command`). One benefit of this is it also works with tests, where we also often don't have an
/// actual connection.
#[derive(Clone, Debug)]
pub(crate) struct UnnamedPeer {
    addr: SocketAddr,
    connection: Option<qp2p::Connection>,
}

impl UnnamedPeer {
    pub(crate) fn addressed(addr: SocketAddr) -> Self {
        Self {
            addr,
            connection: None,
        }
    }

    pub(crate) fn connected(connection: qp2p::Connection) -> Self {
        Self {
            addr: connection.remote_address(),
            connection: Some(connection),
        }
    }

    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn named(self, name: XorName) -> Peer {
        Peer {
            name,
            addr: self.addr,
            connection: Arc::new(RwLock::new(self.connection)),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;
    use proptest::{collection::SizeRange, prelude::*};
    use xor_name::XOR_NAME_LEN;

    pub(crate) fn arbitrary_bytes() -> impl Strategy<Value = [u8; XOR_NAME_LEN]> {
        any::<[u8; XOR_NAME_LEN]>()
    }

    // Generate Vec<Peer> where no two peers have the same name.
    pub(crate) fn arbitrary_unique_peers(
        count: impl Into<SizeRange>,
        age: impl Strategy<Value = u8>,
    ) -> impl Strategy<Value = Vec<Peer>> {
        proptest::collection::btree_map(arbitrary_bytes(), (any::<SocketAddr>(), age), count)
            .prop_map(|peers| {
                peers
                    .into_iter()
                    .map(|(mut bytes, (addr, age))| {
                        bytes[XOR_NAME_LEN - 1] = age;
                        let name = XorName(bytes);
                        Peer::new(name, addr)
                    })
                    .collect()
            })
    }
}
