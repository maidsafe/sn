// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::node::{KeyedSig, SigShare};
use crate::types::{PublicKey, Signature};
use bls::PublicKey as BlsPublicKey;
use ed25519_dalek::{PublicKey as EdPublicKey, Signature as EdSignature};
use serde::{Deserialize, Serialize};
use xor_name::XorName;

/// Source authority of a message.
///
/// Source of message and authority to send it. Authority is validated by the signature.
/// Messages do not need to sign this field as it is all verifiable (i.e. if the signature validates
/// against the public key and we know the public key then we are good. If the proof is not
/// recognised we can ask for a longer chain that can be recognised).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MsgKind {
    /// A section information message, which doesn't have any message authority.
    ///
    /// Section information messages can be sent by any network peer (client or node), without any
    /// need for them to prove their authority. This is because section information messages are
    /// read-only, and section information is public across the network.
    SectionInfoMsg,

    /// A data message, with their authority.
    ///
    /// Client authority is needed to access private data, such as reading or writing a private
    /// file.
    DataMsg(ClientSigned),

    /// A message from a Node with its own independent authority.
    ///
    /// Node authority is needed when nodes send messages directly to other nodes.
    // FIXME: is the above true? What does is the recieving node validating against?
    NodeSignedMsg(NodeSigned),

    /// A message from an Elder node with its share of the section authority.
    ///
    /// Section share authority is needed for messages related to section administration, such as
    /// DKG and relocation.
    NodeBlsShareSignedMsg(BlsShareSigned),

    /// A message from an Elder node with authority of its whole section.
    // FIXME: find an example.
    SectionSignedMsg(SectionSigned),
}

/// Authority of a client
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientSigned {
    /// Client public key.
    pub public_key: PublicKey,
    /// Client signature.
    pub signature: Signature,
}

/// Authority of a single peer.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeSigned {
    /// Section key of the source.
    pub section_pk: BlsPublicKey,
    /// Public key of the source peer.
    pub public_key: EdPublicKey,
    /// Ed25519 signature of the message corresponding to the public key of the source peer.
    pub signature: EdSignature,
}

/// Authority of a single peer that uses it's BLS Keyshare to sign the message.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlsShareSigned {
    /// Section key of the source.
    pub section_pk: BlsPublicKey,
    /// Name in the source section.
    pub src_name: XorName,
    /// Proof Share signed by the peer's BLS KeyShare.
    pub sig_share: SigShare,
}

/// Authority of a whole section.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SectionSigned {
    /// Section key of the source.
    pub section_pk: BlsPublicKey,
    /// Name in the source section.
    pub src_name: XorName,
    /// BLS proof of the message corresponding to the source section.
    pub sig: KeyedSig,
}
