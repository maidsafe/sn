// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod dkg;
mod dkg_msgs_utils;
mod proposal;
mod proven;
#[cfg(test)]
pub mod test_utils;

pub(crate) use self::{
    dkg::{DkgCommands, DkgVoter},
    dkg_msgs_utils::{DkgFailureSigSetUtils, DkgFailureSigUtils, DkgKeyUtils},
    proposal::{ProposalAggregator, ProposalError, ProposalUtils},
};
pub(crate) use crate::messaging::node::{SignatureAggregator, KeyedSig, SigShare};
pub use proven::ProvenUtils;
use serde::Serialize;

// Verify the integrity of `message` against signature.
pub(crate) fn verify_sig<T: Serialize>(sig: &KeyedSig, message: &T) -> bool {
    bincode::serialize(message)
        .map(|bytes| sig.verify(&bytes))
        .unwrap_or(false)
}
