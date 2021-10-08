// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Core;
use crate::messaging::{
    signature_aggregator::Error as AggregatorError,
    system::{Proposal, SigShare},
};
use crate::routing::{dkg::ProposalError, routing_api::command::Command, Error, Result};

// Decisions
impl Core {
    // Insert the proposal into the proposal aggregator and handle it if aggregated.
    pub(crate) async fn handle_proposal(
        &self,
        proposal: Proposal,
        sig_share: SigShare,
    ) -> Result<Vec<Command>> {
        match self.proposal_aggregator.add(proposal, sig_share).await {
            Ok((proposal, sig)) => Ok(vec![Command::HandleAgreement { proposal, sig }]),
            Err(ProposalError::Aggregation(AggregatorError::NotEnoughShares)) => {
                trace!("Proposal inserted in aggregator, not enough sig shares yet",);
                Ok(vec![])
            }
            Err(error) => {
                error!("Failed to add proposal: {:?}", error);
                Err(Error::InvalidSignatureShare)
            }
        }
    }
}
