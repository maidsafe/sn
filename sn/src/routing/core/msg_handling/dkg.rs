// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::super::Core;
use crate::messaging::{
    system::{DkgFailureSig, DkgFailureSigSet, DkgSessionId, ElderCandidates, Proposal, SystemMsg},
    SectionAuthorityProvider,
};
use crate::routing::{
    dkg::DkgFailureSigSetUtils,
    error::{Error, Result},
    log_markers::LogMarker,
    routing_api::command::Command,
    section::SectionKeyShare,
    SectionAuthorityProviderUtils,
};
use bls::PublicKey as BlsPublicKey;
use bls_dkg::key_gen::message::Message as DkgMessage;
use std::{collections::BTreeSet, net::SocketAddr};
use xor_name::XorName;

impl Core {
    pub(crate) async fn handle_dkg_start(
        &self,
        session_id: DkgSessionId,
        elder_candidates: ElderCandidates,
    ) -> Result<Vec<Command>> {
        trace!("Received DkgStart for {:?}", elder_candidates);
        self.dkg_voter
            .start(
                &self.node.read().await.clone(),
                session_id,
                elder_candidates,
                *self.section_chain().await.last_key(),
            )
            .await
    }

    pub(crate) async fn handle_dkg_message(
        &self,
        session_id: DkgSessionId,
        message: DkgMessage,
        sender: XorName,
    ) -> Result<Vec<Command>> {
        trace!(
            "{} {:?} from {}",
            LogMarker::DkgMessageHandling,
            message,
            sender
        );

        self.dkg_voter
            .process_message(
                &self.node.read().await.clone(),
                &session_id,
                message,
                *self.section_chain().await.last_key(),
            )
            .await
    }

    pub(crate) fn handle_dkg_failure_observation(
        &self,
        session_id: DkgSessionId,
        failed_participants: &BTreeSet<XorName>,
        signed: DkgFailureSig,
    ) -> Result<Vec<Command>> {
        match self
            .dkg_voter
            .process_failure(&session_id, failed_participants, signed)
        {
            None => Ok(vec![]),
            Some(cmd) => Ok(vec![cmd]),
        }
    }

    pub(crate) async fn handle_dkg_failure_agreement(
        &self,
        sender: &XorName,
        failure_set: &DkgFailureSigSet,
    ) -> Result<Vec<Command>> {
        if self.section.members().get(sender).is_none() {
            return Err(Error::InvalidSrcLocation);
        }

        let generation = self.section.main_chain_branch_len().await;

        let elder_candidates = if let Some(elder_candidates) = self
            .section
            .promote_and_demote_elders(&self.node.read().await.name(), &BTreeSet::new())
            .await
            .into_iter()
            .find(|elder_candidates| failure_set.verify(elder_candidates, generation))
        {
            elder_candidates
        } else {
            trace!("Ignore DKG failure agreement with invalid signeds or outdated participants",);
            return Ok(vec![]);
        };

        let mut commands = vec![];

        if !failure_set.failed_participants.is_empty() {
            // The DKG failure is regarding failed_participants, i.e. potential unresponsive node.
            trace!(
                "Received DKG failure agreement, propose offline for failed participants: {:?} , DKG generation({}), candidates: {:?}",
                failure_set.failed_participants,
                generation, elder_candidates
            );
            commands.extend(
                self.cast_offline_proposals(&failure_set.failed_participants)
                    .await?,
            );
        }

        trace!(
            "Received DKG failure agreement, we will restart with candidates: {:?} except failed participants: {:?}",
            elder_candidates, failure_set.failed_participants
        );

        commands.extend(
            self.promote_and_demote_elders_except(&failure_set.failed_participants)
                .await?,
        );
        Ok(commands)
    }

    pub(crate) async fn handle_dkg_outcome(
        &self,
        section_auth: SectionAuthorityProvider,
        key_share: SectionKeyShare,
    ) -> Result<Vec<Command>> {
        trace!(
            "{} public_key={:?}",
            LogMarker::HandlingDkgSuccessfulOutcome,
            key_share.public_key_set.public_key()
        );

        // Add our new keyshare to our cache, we will then use
        // it to sign any msg that needs section agreement.
        self.section_keys_provider.insert(key_share.clone()).await;

        let proposal = Proposal::SectionInfo(section_auth);
        let recipients: Vec<_> = self.section.authority_provider().await.peers();
        let result = self
            .send_proposal_with(recipients, proposal, &key_share)
            .await;

        result
    }

    pub(crate) async fn handle_dkg_failure(
        &self,
        failure_set: DkgFailureSigSet,
    ) -> Result<Command> {
        let node_msg = SystemMsg::DkgFailureAgreement(failure_set);
        self.send_message_to_our_elders(node_msg).await
    }

    pub(crate) async fn check_lagging(
        &self,
        peer: (XorName, SocketAddr),
        public_key: &BlsPublicKey,
    ) -> Result<Option<Command>> {
        if self.section.has_chain_key(public_key).await
            && public_key != &self.section.section_key().await
        {
            let msg = self.generate_ae_update(*public_key, true).await?;
            trace!("{}", LogMarker::SendingAeUpdateAfterLagCheck);

            let cmd = self.send_direct_message(peer, msg, *public_key).await?;
            Ok(Some(cmd))
        } else {
            Ok(None)
        }
    }
}
