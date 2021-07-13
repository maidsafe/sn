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
use hex_fmt::HexFmt;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug};
use xor_name::XorName;

/// Source authority of a message.
/// Src of message and authority to send it. Authority is validated by the signature.
/// Messages do not need to sign this field as it is all verifiable (i.e. if the sig validates
/// agains the pub key and we know the pub key then we are good. If the proof is not recognised we
/// ask for a longer chain that can be recognised).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum MsgKind {
    /// SectionInfoMsg wich doesn't contain any msg authority
    SectionInfoMsg,
    /// ClietnMsg with authority provided by a client
    ClientMsg(ClientSigned),
    /// NodeMsg with authority of a single peer.
    NodeSignedMsg(NodeSigned),
    /// NodeMsg with authority of a single peer that uses it's BLS Keyshare to sign the message.
    NodeBlsShareSignedMsg(BlsShareSigned),
    /// NodeMsg with authority of a whole section.
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
    /// ed-25519 signature of the message corresponding to the public key of the source peer.
    pub signature: EdSignature,
}

/// Authority of a single peer that uses it's BLS Keyshare to sign the message.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlsShareSigned {
    /// Section key of the source.
    pub section_pk: BlsPublicKey,
    /// Name in the source section
    pub src_name: XorName,
    /// Proof Share signed by the peer's BLS KeyShare
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

impl Debug for MsgKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SectionInfoMsg => write!(f, "SectionInfoMsg"),
            Self::ClientMsg(ClientSigned {
                public_key,
                signature,
            }) => write!(
                f,
                "ClientMsg {{ public_key: {:?}, signature: {:?} }}",
                public_key, signature
            ),
            Self::NodeSignedMsg(NodeSigned {
                section_pk,
                public_key,
                signature,
            }) => write!(
                f,
                "NodeSignedMsg {{ section_pk: {:?}, public_key: {:10?}, signature: {:10?} }}",
                section_pk,
                HexFmt(public_key),
                HexFmt(signature)
            ),
            Self::NodeBlsShareSignedMsg(BlsShareSigned {
                section_pk,
                src_name,
                sig_share,
            }) => write!(
                f,
                "NodeBlsShareSignedMsg {{ section_pk: {:?}, src_name: {:?}, sig_share: {:?} ",
                section_pk, src_name, sig_share
            ),
            Self::SectionSignedMsg(SectionSigned {
                section_pk,
                src_name,
                sig,
            }) => write!(
                f,
                "Section {{ section_pk: {:?}, src_name: {:?}, sig: {:?}",
                section_pk, src_name, sig
            ),
        }
    }
}
