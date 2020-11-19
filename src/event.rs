// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::location::{DstLocation, SrcLocation};

use bytes::Bytes;
use ed25519_dalek::Keypair;
use hex_fmt::HexFmt;
pub use qp2p::{RecvStream, SendStream};
use std::{
    collections::BTreeSet,
    fmt::{self, Debug, Formatter},
    net::SocketAddr,
    sync::Arc,
};
use xor_name::{Prefix, XorName};

/// An Event raised by a `Node` or `Client` via its event sender.
///
/// These are sent by sn_routing to the library's user. It allows the user to handle requests and
/// responses, and to react to changes in the network.
///
/// `Request` and `Response` events from section locations are only raised once the majority has
/// been reached, i.e. enough members of the section have sent the same message.
// FIXME - See https://maidsafe.atlassian.net/browse/MAID-2026 for info on removing this exclusion.
#[allow(clippy::large_enum_variant)]
pub enum Event {
    /// Received a message.
    MessageReceived {
        /// The content of the message.
        content: Bytes,
        /// The source location that sent the message.
        src: SrcLocation,
        /// The destination location that receives the message.
        dst: DstLocation,
    },
    /// The node has been promoted to elder
    PromotedToElder,
    /// The node has been promoted to adult
    PromotedToAdult,
    /// The node has been demoted from elder
    Demoted,
    /// A new peer joined our section.
    MemberJoined {
        /// Name of the node
        name: XorName,
        /// Previous name before relocation or `None` if it is a new node.
        previous_name: Option<XorName>,
        /// Age of the node
        age: u8,
    },
    /// A node left our section.
    MemberLeft {
        /// Name of the node
        name: XorName,
        /// Age of the node
        age: u8,
    },
    /// The set of elders in our section has changed.
    EldersChanged {
        /// The prefix of our section.
        prefix: Prefix,
        /// The BLS public key of our section.
        key: bls::PublicKey,
        /// The set of elders of our section.
        elders: BTreeSet<XorName>,
    },
    /// This node has started relocating to other section. Will be followed by
    /// `Relocated` when the node finishes joining the destination section.
    RelocationStarted {
        /// Previous name before relocation
        previous_name: XorName,
    },
    /// This node has completed relocation to other section.
    Relocated {
        /// Old name before the relocation.
        previous_name: XorName,
        /// New keypair to be used after relocation.
        new_keypair: Arc<Keypair>,
    },
    /// Disconnected or failed to connect - restart required.
    RestartRequired,
    /// Received a message from a client node.
    ClientMessageReceived {
        /// The content of the message.
        content: Bytes,
        /// The address of the client that sent the message.
        src: SocketAddr,
        /// Stream to send messages back to the client that sent the message
        send: SendStream,
        /// Stream to receive more messages from the client on the same channel
        recv: RecvStream,
    },
}

impl Debug for Event {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        match self {
            Self::MessageReceived { content, src, dst } => write!(
                formatter,
                "MessageReceived {{ content: \"{:<8}\", src: {:?}, dst: {:?} }}",
                HexFmt(content),
                src,
                dst
            ),
            Self::PromotedToElder => write!(formatter, "PromotedToElder"),
            Self::PromotedToAdult => write!(formatter, "PromotedToAdult"),
            Self::Demoted => write!(formatter, "Demoted"),
            Self::MemberJoined {
                name,
                previous_name,
                age,
            } => formatter
                .debug_struct("MemberJoined")
                .field("name", name)
                .field("previous_name", previous_name)
                .field("age", age)
                .finish(),
            Self::MemberLeft { name, age } => formatter
                .debug_struct("MemberLeft")
                .field("name", name)
                .field("age", age)
                .finish(),
            Self::EldersChanged {
                prefix,
                key,
                elders,
            } => formatter
                .debug_struct("EldersChanged")
                .field("prefix", prefix)
                .field("key", key)
                .field("elders", elders)
                .finish(),
            Self::RelocationStarted { previous_name } => formatter
                .debug_struct("RelocationStarted")
                .field("previous_name", previous_name)
                .finish(),
            Self::Relocated {
                previous_name,
                new_keypair,
            } => formatter
                .debug_struct("Relocated")
                .field("previous_name", previous_name)
                .field("new_keypair", new_keypair)
                .finish(),
            Self::RestartRequired => write!(formatter, "RestartRequired"),
            Self::ClientMessageReceived { content, src, .. } => write!(
                formatter,
                "ClientMessageReceived {{ content: \"{:<8}\", src: {:?} }}",
                HexFmt(content),
                src,
            ),
        }
    }
}
