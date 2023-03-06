// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

pub use super::system::{SectionSig, SectionSigShare};
use super::{Error, Result};
use crate::types::{PublicKey, Signature};
use bls::PublicKey as BlsPublicKey;
use ed25519_dalek::{PublicKey as EdPublicKey, Signature as EdSignature, Verifier as _};
use serde::{Deserialize, Serialize};

/// Authority of a network client.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ClientAuth {
    /// Client's public key.
    pub public_key: PublicKey,
    /// Client's signature.
    pub signature: Signature,
}

/// Authority of a node.
#[derive(Clone, Eq, PartialEq, custom_debug::Debug, serde::Deserialize, serde::Serialize)]
pub struct NodeSig {
    /// Section key of the node.
    pub section_pk: BlsPublicKey,
    /// Public key of the node.
    #[debug(with = "PublicKey::fmt_ed25519")]
    pub node_ed_pk: EdPublicKey,
    /// Ed25519 signature of the message corresponding to the public key of the node.
    #[debug(with = "Signature::fmt_ed25519")]
    #[serde(with = "serde_bytes")]
    pub signature: EdSignature,
}

/// Verified authority.
///
/// Values of this type constitute a proof that the signature is valid for a particular payload.
/// This is made possible by keeping the field private, and performing verification in all possible
/// constructors of the type.
///
/// Validation is defined by the [`VerifyAuthority`] impl for `T`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthorityProof<T>(pub T);

impl<T: VerifyAuthority> AuthorityProof<T> {
    /// Verify the authority of `inner`.
    ///
    /// This is the only way to construct an instance of [`AuthorityProof`] from a `T`. Since it's
    /// implemented to call [`VerifyAuthority::verify_authority`] an instance of [`AuthorityProof<T>`] is
    /// guaranteed to be valid with respect to that trait's impl.
    pub fn verify(inner: T, payload: impl AsRef<[u8]>) -> Result<Self> {
        inner.verify_authority(payload).map(Self)
    }

    /// Drop the proof of validity and return the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> core::ops::Deref for AuthorityProof<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Verify authority.
///
/// This trait drives the verification logic used by [`AuthorityProof`].
///
/// **Note:** this trait is 'sealed', and as such cannot be implemented outside of this crate.
pub trait VerifyAuthority: Sized + sealed::Sealed {
    /// Verify that we represent authority for `payload`.
    fn verify_authority(self, payload: impl AsRef<[u8]>) -> Result<Self>;
}

impl VerifyAuthority for ClientAuth {
    fn verify_authority(self, payload: impl AsRef<[u8]>) -> Result<Self> {
        self.public_key
            .verify(&self.signature, payload)
            .map_err(|_| Error::InvalidSignature)?;
        Ok(self)
    }
}
impl sealed::Sealed for ClientAuth {}

impl VerifyAuthority for NodeSig {
    fn verify_authority(self, payload: impl AsRef<[u8]>) -> Result<Self> {
        self.node_ed_pk
            .verify(payload.as_ref(), &self.signature)
            .map_err(|_| Error::InvalidSignature)?;
        Ok(self)
    }
}
impl sealed::Sealed for NodeSig {}

impl VerifyAuthority for SectionSigShare {
    fn verify_authority(self, payload: impl AsRef<[u8]>) -> Result<Self> {
        if !self.verify(payload.as_ref()) {
            return Err(Error::InvalidSignature);
        }
        Ok(self)
    }
}
impl sealed::Sealed for SectionSigShare {}

impl VerifyAuthority for SectionSig {
    fn verify_authority(self, payload: impl AsRef<[u8]>) -> Result<Self> {
        if !self.public_key.verify(&self.signature, payload) {
            return Err(Error::InvalidSignature);
        }

        Ok(self)
    }
}
impl sealed::Sealed for SectionSig {}

mod sealed {
    pub trait Sealed {}
}
