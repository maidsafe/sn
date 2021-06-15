// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{SignatureAggregator, Signed, SignedShare};
use crate::routing::{error::Result, messages::PlainMessageUtils};
use serde::{Serialize, Serializer};
use crate::messaging::node::Proposal;
use thiserror::Error;

pub trait ProposalUtils {
    fn prove(
        &self,
        public_key_set: bls::PublicKeySet,
        index: usize,
        secret_key_share: &bls::SecretKeyShare,
    ) -> Result<SignedShare>;

    fn as_signable(&self) -> SignableView;
}

impl ProposalUtils for Proposal {
    /// Create SignedShare for this proposal.
    fn prove(
        &self,
        public_key_set: bls::PublicKeySet,
        index: usize,
        secret_key_share: &bls::SecretKeyShare,
    ) -> Result<SignedShare> {
        Ok(SignedShare::new(
            public_key_set,
            index,
            secret_key_share,
            &bincode::serialize(&self.as_signable()).map_err(|_| ProposalError::Invalid)?,
        ))
    }

    fn as_signable(&self) -> SignableView {
        SignableView(self)
    }
}

// View of a `Proposal` that can be serialized for the purpose of signing.
pub struct SignableView<'a>(pub &'a Proposal);

impl<'a> Serialize for SignableView<'a> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Proposal::Online { node_state, .. } => node_state.serialize(serializer),
            Proposal::Offline(node_state) => node_state.serialize(serializer),
            Proposal::SectionInfo(info) => info.serialize(serializer),
            Proposal::OurElders(info) => info.signed.public_key.serialize(serializer),
            // Proposal::TheirKey { prefix, key } => (prefix, key).serialize(serializer),
            // Proposal::TheirKnowledge { prefix, key } => (prefix, key).serialize(serializer),
            Proposal::AccumulateAtSrc { message, .. } => {
                message.as_signable().serialize(serializer)
            }
            Proposal::JoinsAllowed(joins_allowed) => joins_allowed.serialize(serializer),
        }
    }
}

// Aggregator of `Proposal`s.
#[derive(Default)]
pub(crate) struct ProposalAggregator(SignatureAggregator);

impl ProposalAggregator {
    pub fn add(
        &mut self,
        proposal: Proposal,
        signed_share: SignedShare,
    ) -> Result<(Proposal, Signed), ProposalError> {
        let bytes =
            bincode::serialize(&SignableView(&proposal)).map_err(|_| ProposalError::Invalid)?;
        let signed = self.0.add(&bytes, signed_share)?;
        Ok((proposal, signed))
    }
}

#[derive(Debug, Error)]
pub enum ProposalError {
    #[error("failed to aggregate signature shares: {0}")]
    Aggregation(#[from] crate::messaging::node::Error),
    #[error("invalid proposal")]
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{dkg, section};
    use anyhow::Result;
    use std::fmt::Debug;
    use xor_name::Prefix;

    #[test]
    fn serialize_for_signing() -> Result<()> {
        // Proposal::SectionInfo
        let (section_auth, _, _) =
            section::test_utils::gen_section_authority_provider(Prefix::default(), 4);
        let proposal = Proposal::SectionInfo(section_auth.clone());
        verify_serialize_for_signing(&proposal, &section_auth)?;

        // Proposal::OurElders
        let new_sk = bls::SecretKey::random();
        let new_pk = new_sk.public_key();
        let section_signed_auth = dkg::test_utils::section_signed(&new_sk, section_auth)?;
        let proposal = Proposal::OurElders(section_signed_auth);
        verify_serialize_for_signing(&proposal, &new_pk)?;

        Ok(())
    }

    // Verify that `SignableView(proposal)` serializes the same as `should_serialize_as`.
    fn verify_serialize_for_signing<T>(proposal: &Proposal, should_serialize_as: &T) -> Result<()>
    where
        T: Serialize + Debug,
    {
        let actual = bincode::serialize(&SignableView(proposal))?;
        let expected = bincode::serialize(should_serialize_as)?;

        assert_eq!(
            actual, expected,
            "expected SignableView({:?}) to serialize same as {:?}, but didn't",
            proposal, should_serialize_as
        );

        Ok(())
    }
}
