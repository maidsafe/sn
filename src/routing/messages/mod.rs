// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod plain_message;
mod src_authority;

pub use self::{plain_message::PlainMessageUtils, src_authority::SrcAuthorityUtils};
use crate::messaging::node::{KeyedSig, SigShare};
use crate::messaging::{
    node::{JoinResponse, PlainMessage, RoutingMsg, SrcAuthority, Variant},
    Aggregation, DstLocation, MessageId,
};
use crate::routing::{
    dkg::SectionSignedUtils,
    ed25519::{self, Verifier},
    error::{Error, Result},
    node::Node,
    section::{SectionKeyShare, SectionUtils},
};
use secured_linked_list::{error::Error as SecuredLinkedListError, SecuredLinkedList};
use serde::Serialize;
use std::fmt::Debug;
use thiserror::Error;
use xor_name::XorName;

/// Message sent over the network.
pub trait RoutingMsgUtils {
    /// Check the signature is valid. Only called on message receipt.
    fn check_signature(msg: &RoutingMsg) -> Result<()>;

    /// Creates a signed message where signature is assumed valid.
    fn new_signed(
        src: SrcAuthority,
        dst: DstLocation,
        variant: Variant,
        section_key: bls::PublicKey,
    ) -> Result<RoutingMsg, Error>;

    /// Creates a message signed using a BLS KeyShare for destination accumulation
    fn for_dst_accumulation(
        key_share: &SectionKeyShare,
        src_name: XorName,
        dst: DstLocation,
        variant: Variant,
        proof_chain: SecuredLinkedList,
    ) -> Result<RoutingMsg, Error>;

    /// Converts the message src authority from `BlsShare` to `Section` on successful accumulation.
    /// Returns errors if src is not `BlsShare` or if the signed is invalid.
    fn into_dst_accumulated(self, sig: KeyedSig) -> Result<RoutingMsg>;

    fn signable_view(&self) -> SignableView;

    /// Creates a signed message from single node.
    fn single_src(
        node: &Node,
        dst: DstLocation,
        variant: Variant,
        section_key: bls::PublicKey,
    ) -> Result<RoutingMsg>;

    /// Creates a signed message from a section.
    /// Note: `signed` isn't verified and is assumed valid.
    fn section_src(
        plain: PlainMessage,
        sig: KeyedSig,
        section_chain: SecuredLinkedList,
    ) -> Result<RoutingMsg>;

    /// Verify this message is properly signed and trusted.
    fn verify<'a, I: IntoIterator<Item = &'a bls::PublicKey>>(
        &self,
        trusted_keys: I,
    ) -> Result<VerifyStatus>;

    /// Getter
    fn keyed_sig(&self) -> Option<KeyedSig>;

    fn verify_variant<'a, I: IntoIterator<Item = &'a bls::PublicKey>>(
        &self,
        trusted_keys: I,
    ) -> Result<VerifyStatus>;

    /// Returns an updated message with the provided Section key i.e. known to be latest.
    fn updated_with_latest_key(&mut self, section_pk: bls::PublicKey);
}

impl RoutingMsgUtils for RoutingMsg {
    /// Check the signature is valid. Only called on message receipt.
    fn check_signature(msg: &RoutingMsg) -> Result<()> {
        let signed_bytes = bincode::serialize(&SignableView {
            dst: &msg.dst,
            variant: &msg.variant,
        })
        .map_err(|_| Error::InvalidMessage)?;

        match &msg.src {
            SrcAuthority::Node {
                public_key,
                signature,
                ..
            } => {
                if public_key.verify(&signed_bytes, signature).is_err() {
                    error!("Failed signature: {:?}", msg);
                    return Err(Error::CreateError(CreateError::FailedSignature));
                }
            }
            SrcAuthority::BlsShare { sig_share, .. } => {
                if !sig_share.verify(&signed_bytes) {
                    error!("Failed signature: {:?}", msg);
                    return Err(Error::CreateError(CreateError::FailedSignature));
                }

                if sig_share.public_key_set.public_key() != msg.section_pk {
                    error!(
                        "Signed share public key doesn't match signed chain last key: {:?}",
                        msg
                    );
                    return Err(Error::CreateError(CreateError::FailedSignature));
                }
            }
            SrcAuthority::Section { sig, .. } => {
                if !msg.section_pk.verify(&sig.signature, &signed_bytes) {
                    error!(
                        "Failed signature: {:?} (Section PK: {:?})",
                        msg, msg.section_pk
                    );
                    return Err(Error::CreateError(CreateError::FailedSignature));
                }
            }
        }

        Ok(())
    }

    /// Creates a signed message where signature is assumed valid.
    fn new_signed(
        src: SrcAuthority,
        dst: DstLocation,
        variant: Variant,
        section_pk: bls::PublicKey,
    ) -> Result<RoutingMsg, Error> {
        // Create message id from src authority signature
        let id = match &src {
            SrcAuthority::Node { signature, .. } => MessageId::from_content(signature),
            SrcAuthority::BlsShare { sig_share, .. } => {
                MessageId::from_content(&sig_share.signature_share.0)
            }
            SrcAuthority::Section { sig, .. } => MessageId::from_content(&sig.signature),
        }
        .unwrap_or_default();

        let msg = RoutingMsg {
            id,
            src,
            dst,
            aggregation: Aggregation::None,
            variant,
            section_pk,
        };

        Ok(msg)
    }

    /// Creates a message signed using a BLS KeyShare for destination accumulation
    fn for_dst_accumulation(
        key_share: &SectionKeyShare,
        src_name: XorName,
        dst: DstLocation,
        variant: Variant,
        section_chain: SecuredLinkedList,
    ) -> Result<Self, Error> {
        let serialized = bincode::serialize(&SignableView {
            dst: &dst,
            variant: &variant,
        })
        .map_err(|_| Error::InvalidMessage)?;

        let signature_share = key_share.secret_key_share.sign(&serialized);
        let sig_share = SigShare {
            public_key_set: key_share.public_key_set.clone(),
            index: key_share.index,
            signature_share,
        };

        let src = SrcAuthority::BlsShare {
            src_name,
            sig_share,
            section_chain: section_chain.clone(),
        };

        Self::new_signed(src, dst, variant, *section_chain.last_key())
    }

    /// Converts the message src authority from `BlsShare` to `Section` on successful accumulation.
    /// Returns errors if src is not `BlsShare` or if the signed is invalid.
    fn into_dst_accumulated(mut self, sig: KeyedSig) -> Result<Self> {
        let (sig_share, src_name, section_chain) = if let SrcAuthority::BlsShare {
            sig_share,
            src_name,
            section_chain,
        } = &self.src
        {
            (sig_share.clone(), *src_name, section_chain)
        } else {
            error!("not a message for dst accumulation");
            return Err(Error::InvalidMessage);
        };

        if sig_share.public_key_set.public_key() != sig.public_key {
            error!("signed public key doesn't match signed share public key");
            return Err(Error::InvalidMessage);
        }

        if sig.public_key != self.section_pk {
            error!("signed public key doesn't match the attached section PK");
            return Err(Error::InvalidMessage);
        }

        let bytes = bincode::serialize(&self.signable_view()).map_err(|_| Error::InvalidMessage)?;

        if !sig.verify(&bytes) {
            return Err(Error::FailedSignature);
        }

        self.src = SrcAuthority::Section {
            sig,
            src_name,
            section_chain: section_chain.clone(),
        };

        Ok(self)
    }

    fn signable_view(&self) -> SignableView {
        SignableView {
            dst: &self.dst,
            variant: &self.variant,
        }
    }

    /// Creates a signed message from single node.
    fn single_src(
        node: &Node,
        dst: DstLocation,
        variant: Variant,
        section_pk: bls::PublicKey,
    ) -> Result<Self> {
        let serialized = bincode::serialize(&SignableView {
            dst: &dst,
            variant: &variant,
        })
        .map_err(|_| Error::InvalidMessage)?;

        let signature = ed25519::sign(&serialized, &node.keypair);
        let src = SrcAuthority::Node {
            public_key: node.keypair.public,
            signature,
        };

        RoutingMsg::new_signed(src, dst, variant, section_pk)
    }

    /// Creates a signed message from a section.
    /// Note: `signed` isn't verified and is assumed valid.
    fn section_src(
        plain: PlainMessage,
        sig: KeyedSig,
        section_chain: SecuredLinkedList,
    ) -> Result<Self> {
        Self::new_signed(
            SrcAuthority::Section {
                src_name: plain.src,
                sig,
                section_chain: section_chain.clone(),
            },
            plain.dst,
            plain.variant,
            *section_chain.last_key(),
        )
    }

    /// Verify this message is properly signed and trusted.
    fn verify<'a, I>(&self, trusted_keys: I) -> Result<VerifyStatus>
    where
        I: IntoIterator<Item = &'a bls::PublicKey>,
    {
        let bytes = bincode::serialize(&SignableView {
            dst: &self.dst,
            variant: &self.variant,
        })
        .map_err(|_| Error::InvalidMessage)?;

        match &self.src {
            SrcAuthority::Node {
                public_key,
                signature,
                ..
            } => {
                if public_key.verify(&bytes, signature).is_err() {
                    return Err(Error::FailedSignature);
                }

                // Variant-specific verification.
                self.verify_variant(trusted_keys)
            }
            SrcAuthority::BlsShare {
                sig_share,
                section_chain,
                ..
            } => {
                // Signed chain is required for accumulation at destination.
                if sig_share.public_key_set.public_key() != self.section_pk {
                    return Err(Error::InvalidMessage);
                }

                if !sig_share.verify(&bytes) {
                    return Err(Error::FailedSignature);
                }

                if section_chain.check_trust(trusted_keys) {
                    Ok(VerifyStatus::Full)
                } else {
                    Ok(VerifyStatus::Unknown)
                }
            }
            SrcAuthority::Section {
                sig, section_chain, ..
            } => {
                // Signed chain is required for section-src messages.
                if !self.section_pk.verify(&sig.signature, &bytes) {
                    return Err(Error::FailedSignature);
                }

                if section_chain.check_trust(trusted_keys) {
                    Ok(VerifyStatus::Full)
                } else {
                    Ok(VerifyStatus::Unknown)
                }
            }
        }
    }

    /// Getter
    fn keyed_sig(&self) -> Option<KeyedSig> {
        if let SrcAuthority::Section { sig, .. } = &self.src {
            Some(sig.clone())
        } else {
            None
        }
    }

    fn verify_variant<'a, I>(&self, trusted_keys: I) -> Result<VerifyStatus>
    where
        I: IntoIterator<Item = &'a bls::PublicKey>,
    {
        let proof_chain = match &self.variant {
            Variant::JoinResponse(resp) => {
                if let JoinResponse::Approval {
                    ref section_auth,
                    ref node_state,
                    ref section_chain,
                    ..
                } = **resp
                {
                    if !section_auth.verify(section_chain) {
                        return Err(Error::InvalidMessage);
                    }

                    if !node_state.verify(section_chain) {
                        return Err(Error::InvalidMessage);
                    }

                    section_chain
                } else {
                    return Ok(VerifyStatus::Full);
                }
            }
            Variant::SectionKnowledge {
                src_info: (_, ref chain),
                ..
            } => chain,
            Variant::Sync { section, .. } => section.chain(),
            _ => return Ok(VerifyStatus::Full),
        };

        if proof_chain.check_trust(trusted_keys) {
            Ok(VerifyStatus::Full)
        } else {
            Ok(VerifyStatus::Unknown)
        }
    }

    fn updated_with_latest_key(&mut self, section_pk: bls::PublicKey) {
        self.section_pk = section_pk
    }
}

#[derive(Eq, PartialEq, Debug)]
pub enum VerifyStatus {
    // The message has been fully verified.
    Full,
    // The message trust and integrity cannot be verified because it's signed is not trusted by us,
    // even though it is valid. The message should be relayed to other nodes who might be able to
    // verify it.
    Unknown,
}

/// Status of an incomming message.
#[derive(Eq, PartialEq)]
pub enum MessageStatus {
    /// Message is useful and should be handled.
    Useful,
    /// Message is useless and should be discarded.
    Useless,
    /// Message trust can't be established.
    Untrusted,
}

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("signature check failed")]
    FailedSignature,
    #[error("public key mismatch")]
    PublicKeyMismatch,
}

/// Error returned from `RoutingMsg::extend_proof_chain`.
#[derive(Debug, Error)]
pub enum ExtendSignedChainError {
    #[error("message has no signed chain")]
    NoSignedChain,
    #[error("failed to extend signed chain: {}", .0)]
    Extend(#[from] SecuredLinkedListError),
    #[error("failed to re-create message: {}", .0)]
    Create(#[from] CreateError),
}

// View of a message that can be serialized for the purpose of signing.
#[derive(Serialize)]
pub struct SignableView<'a> {
    // TODO: why don't we include also `src`?
    pub dst: &'a DstLocation,
    pub variant: &'a Variant,
}
