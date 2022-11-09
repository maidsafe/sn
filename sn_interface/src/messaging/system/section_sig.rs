// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::{
    signature_aggregator::{AggregatorError, SignatureAggregator},
    AuthorityProof,
};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug, Formatter};

/// Signature created when a quorum of the section elders has agreed on something.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SectionSig {
    /// The BLS public key.
    pub public_key: bls::PublicKey,
    /// The BLS signature corresponding to the public key.
    pub signature: bls::Signature,
}

impl Debug for SectionSig {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_tuple("SectionSig").field(&self.public_key).finish()
    }
}

impl SectionSig {
    /// Verifies this signature against the payload.
    pub fn verify(&self, payload: &[u8]) -> bool {
        self.public_key.verify(&self.signature, payload)
    }

    /// Try to construct verified section authority by aggregating a new share.
    pub fn try_authorize(
        aggregator: &mut SignatureAggregator,
        share: SectionSigShare,
        payload: impl AsRef<[u8]>,
    ) -> Result<Option<AuthorityProof<Self>>, AggregatorError> {
        match aggregator.try_aggregate(payload.as_ref(), share)? {
            Some(sig) => Ok(Some(AuthorityProof(Self {
                public_key: sig.public_key,
                signature: sig.signature,
            }))),
            None => Ok(None),
        }
    }
}

/// Single share of `SectionSig`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SectionSigShare {
    /// BLS public key set.
    pub public_key_set: bls::PublicKeySet,
    /// Index of the node that created this signature share.
    pub index: usize,
    /// BLS signature share corresponding to the `index`-th public key share of the public key set.
    pub signature_share: bls::SignatureShare,
}

impl SectionSigShare {
    /// Creates new signature share.
    pub fn new(
        public_key_set: bls::PublicKeySet,
        index: usize,
        secret_key_share: &bls::SecretKeyShare,
        payload: &[u8],
    ) -> Self {
        Self {
            public_key_set,
            index,
            signature_share: secret_key_share.sign(payload),
        }
    }

    /// Verifies this signature share against the payload.
    pub fn verify(&self, payload: &[u8]) -> bool {
        self.public_key_set
            .public_key_share(self.index)
            .verify(&self.signature_share, payload)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::TestKeys;
    use bls::SecretKey;

    #[test]
    fn verify_keyed_sig() {
        let sk = SecretKey::random();
        let data = "hello";
        let sig = TestKeys::get_section_sig_bytes(&sk, data.as_bytes());
        assert!(sig.verify(data.as_bytes()));
    }
}
