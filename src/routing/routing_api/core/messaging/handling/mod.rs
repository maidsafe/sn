// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod agreement;
mod bad_msgs;
mod decisions;
mod relocation;
mod resource_proof;

use super::super::Core;
use crate::messaging::node::Error as AggregatorError;
use crate::messaging::{
    client::ClientMsg,
    node::{
        DkgFailureSigSet, JoinAsRelocatedRequest, JoinAsRelocatedResponse, JoinRejectionReason,
        JoinRequest, JoinResponse, Network, Peer, Proposal, RoutingMsg, Section,
        SignedRelocateDetails, SrcAuthority, Variant,
    },
    section_info::{GetSectionResponse, SectionInfoMsg},
    DstInfo, DstLocation, EndUser, MessageType, SectionAuthorityProvider,
};
use crate::routing::{
    dkg::{commands::DkgCommands, ProposalError, SigShare},
    error::{Error, Result},
    event::Event,
    messages::{MessageStatus, RoutingMsgUtils, SrcAuthorityUtils, VerifyStatus},
    network::NetworkUtils,
    peer::PeerUtils,
    relocation::{RelocatePayloadUtils, RelocateState, SignedRelocateDetailsUtils},
    routing_api::command::Command,
    section::{
        SectionAuthorityProviderUtils, SectionKeyShare, SectionPeersUtils, SectionUtils,
        FIRST_SECTION_MAX_AGE, FIRST_SECTION_MIN_AGE, MIN_ADULT_AGE,
    },
};
use bytes::Bytes;
use std::{collections::BTreeSet, iter, net::SocketAddr};
use xor_name::XorName;

// Message handling
impl Core {
    pub(crate) async fn handle_message(
        &mut self,
        sender: Option<SocketAddr>,
        msg: RoutingMsg,
        dst_info: DstInfo,
    ) -> Result<Vec<Command>> {
        let mut commands = vec![];

        // Check if the message is for us.
        let in_dst_location = msg.dst.contains(&self.node.name(), self.section.prefix());
        // TODO: Broadcast message to our section when src is a Node as nodes might not know
        // all the elders in our section and the msg needs to be propagated.
        if !in_dst_location {
            info!("Relay closer to the destination");
            if let Some(cmds) = self.relay_message(&msg).await? {
                commands.push(cmds);
            }
        }
        if !in_dst_location {
            // RoutingMsg not for us.
            return Ok(commands);
        }

        match self.decide_message_status(&msg)? {
            MessageStatus::Useful => {
                trace!("Useful message from {:?}: {:?}", sender, msg);
                let (entropy_commands, shall_be_handled) =
                    self.check_for_entropy(&msg, dst_info.clone()).await?;
                let no_ae_commands = entropy_commands.is_empty();
                commands.extend(entropy_commands);
                if shall_be_handled {
                    info!("Entropy check passed. Handling useful msg!");
                    commands.extend(self.handle_useful_message(sender, msg, dst_info).await?);
                } else if no_ae_commands {
                    // For the case of receiving a JoinRequest not matching our prefix.
                    let sender_name = msg.src.name();
                    let sender_addr = if let Some(addr) = sender {
                        addr
                    } else {
                        error!("JoinRequest from {:?} without address", sender_name);
                        return Ok(commands);
                    };
                    let section_auth = self
                        .network
                        .closest(&sender_name)
                        .unwrap_or_else(|| self.section.authority_provider());
                    let variant = Variant::JoinResponse(Box::new(JoinResponse::Redirect(
                        section_auth.clone(),
                    )));
                    trace!("Sending {:?} to {}", variant, sender_name);
                    commands.push(self.send_direct_message(
                        (sender_name, sender_addr),
                        variant,
                        section_auth.section_key(),
                    )?);
                }
            }
            MessageStatus::Untrusted => {
                debug!("Untrusted message from {:?}: {:?} ", sender, msg);
                commands.push(self.handle_untrusted_message(sender, msg, dst_info)?);
            }
            MessageStatus::Useless => {
                debug!("Useless message from {:?}: {:?}", sender, msg);
            }
        }

        Ok(commands)
    }

    pub(crate) async fn handle_section_info_msg(
        &mut self,
        sender: SocketAddr,
        message: SectionInfoMsg,
        dst_info: DstInfo, // The DstInfo contains the XorName of the sender and a random PK during the initial SectionQuery,
    ) -> Vec<Command> {
        // Provide our PK as the dst PK, only redundant as the message
        // itself contains details regarding relocation/registration.
        let dst_info = DstInfo {
            dst: dst_info.dst,
            dst_section_pk: *self.section().chain().last_key(),
        };

        match message {
            SectionInfoMsg::GetSectionQuery(public_key) => {
                let name = XorName::from(public_key);

                debug!("Received GetSectionQuery({}) from {}", name, sender);

                let response = if let (true, Ok(pk_set)) =
                    (self.section.prefix().matches(&name), self.public_key_set())
                {
                    GetSectionResponse::Success(SectionAuthorityProvider {
                        prefix: self.section.authority_provider().prefix(),
                        public_key_set: pk_set,
                        elders: self
                            .section
                            .authority_provider()
                            .peers()
                            .map(|peer| (*peer.name(), *peer.addr()))
                            .collect(),
                    })
                } else {
                    // If we are elder, we should know a section that is closer to `name` that us.
                    // Otherwise redirect to our elders.
                    let section_auth = self
                        .network
                        .closest(&name)
                        .unwrap_or_else(|| self.section.authority_provider());
                    GetSectionResponse::Redirect(section_auth.clone())
                };

                let response = SectionInfoMsg::GetSectionResponse(response);
                debug!("Sending {:?} to {}", response, sender);

                vec![Command::SendMessage {
                    recipients: vec![(name, sender)],
                    delivery_group_size: 1,
                    message: MessageType::SectionInfo {
                        msg: response,
                        dst_info,
                    },
                }]
            }
            SectionInfoMsg::GetSectionResponse(response) => {
                error!("GetSectionResponse unexpectedly received: {:?}", response);
                vec![]
            }
            SectionInfoMsg::SectionInfoUpdate(error) => {
                error!("SectionInfoUpdate received: {:?}", error);
                vec![]
            }
        }
    }

    pub(crate) fn handle_timeout(&mut self, token: u64) -> Result<Vec<Command>> {
        self.dkg_voter
            .handle_timeout(&self.node.keypair, token)
            .into_commands(&self.node, *self.section_chain().last_key())
    }

    // Insert the proposal into the proposal aggregator and handle it if aggregated.
    pub(crate) fn handle_proposal(
        &mut self,
        proposal: Proposal,
        sig_share: SigShare,
    ) -> Result<Vec<Command>> {
        match self.proposal_aggregator.add(proposal, sig_share) {
            Ok((proposal, sig)) => Ok(vec![Command::HandleAgreement { proposal, sig }]),
            Err(ProposalError::Aggregation(crate::messaging::node::Error::NotEnoughShares)) => {
                Ok(vec![])
            }
            Err(error) => {
                error!("Failed to add proposal: {}", error);
                Err(Error::InvalidSignatureShare)
            }
        }
    }

    pub(crate) fn handle_dkg_outcome(
        &mut self,
        section_auth: SectionAuthorityProvider,
        key_share: SectionKeyShare,
    ) -> Result<Vec<Command>> {
        let proposal = Proposal::SectionInfo(section_auth);
        let recipients: Vec<_> = self.section.authority_provider().peers().collect();
        let result = self.send_proposal_with(&recipients, proposal, &key_share);

        let public_key = key_share.public_key_set.public_key();

        self.section_keys_provider.insert_dkg_outcome(key_share);

        if self.section.chain().has_key(&public_key) {
            self.section_keys_provider.finalise_dkg(&public_key)
        }

        result
    }

    pub(crate) fn handle_dkg_failure(&mut self, failure_set: DkgFailureSigSet) -> Result<Command> {
        let variant = Variant::DkgFailureAgreement(failure_set);
        let message = RoutingMsg::single_src(
            &self.node,
            DstLocation::DirectAndUnrouted,
            variant,
            self.section.authority_provider().section_key(),
        )?;
        Ok(self.send_message_to_our_elders(message))
    }

    pub(crate) fn aggregate_message(&mut self, msg: RoutingMsg) -> Result<Option<RoutingMsg>> {
        let sig_share = if let SrcAuthority::BlsShare { sig_share, .. } = &msg.src {
            sig_share
        } else {
            // Not an aggregating message, return unchanged.
            return Ok(Some(msg));
        };

        let signed_bytes =
            bincode::serialize(&msg.signable_view()).map_err(|_| Error::InvalidMessage)?;
        match self
            .message_aggregator
            .add(&signed_bytes, sig_share.clone())
        {
            Ok(sig) => {
                trace!("Successfully accumulated signatures for message: {:?}", msg);
                Ok(Some(msg.into_dst_accumulated(sig)?))
            }
            Err(AggregatorError::NotEnoughShares) => Ok(None),
            Err(err) => {
                error!("Error accumulating message at dst: {:?}", err);
                Err(Error::InvalidSignatureShare)
            }
        }
    }

    pub(crate) async fn handle_useful_message(
        &mut self,
        sender: Option<SocketAddr>,
        msg: RoutingMsg,
        dst_info: DstInfo,
    ) -> Result<Vec<Command>> {
        let msg = if let Some(msg) = self.aggregate_message(msg)? {
            msg
        } else {
            return Ok(vec![]);
        };
        let src_name = msg.src.name();

        match msg.variant {
            Variant::SectionKnowledge { src_info, msg } => {
                self.update_section_knowledge(src_info.0, src_info.1);
                if let Some(message) = msg {
                    Ok(vec![Command::HandleMessage {
                        sender,
                        message: *message,
                        dst_info,
                    }])
                } else {
                    Ok(vec![])
                }
            }
            Variant::Sync { section, network } => self.handle_sync(section, network).await,
            Variant::Relocate(_) => {
                if msg.src.is_section() {
                    let signed_relocate = SignedRelocateDetails::new(msg)?;
                    Ok(self
                        .handle_relocate(signed_relocate)
                        .await?
                        .into_iter()
                        .collect())
                } else {
                    Err(Error::InvalidSrcLocation)
                }
            }
            Variant::RelocatePromise(promise) => self.handle_relocate_promise(promise, msg).await,
            Variant::StartConnectivityTest(name) => Ok(vec![Command::TestConnectivity(name)]),
            Variant::JoinRequest(join_request) => {
                let sender = sender.ok_or(Error::InvalidSrcLocation)?;
                self.handle_join_request(msg.src.peer(sender)?, *join_request)
            }
            Variant::JoinAsRelocatedRequest(join_request) => {
                let sender = sender.ok_or(Error::InvalidSrcLocation)?;
                self.handle_join_as_relocated_request(msg.src.peer(sender)?, *join_request)
            }
            Variant::UserMessage(ref content) => {
                let bytes = Bytes::from(content.clone());
                self.handle_user_message(msg, bytes).await
            }
            Variant::BouncedUntrustedMessage {
                msg: bounced_msg,
                dst_info,
            } => {
                let sender = sender.ok_or(Error::InvalidSrcLocation)?;
                Ok(vec![self.handle_bounced_untrusted_message(
                    msg.src.peer(sender)?,
                    dst_info.dst_section_pk,
                    *bounced_msg,
                )?])
            }
            Variant::SectionKnowledgeQuery {
                last_known_key,
                msg: returned_msg,
            } => {
                let sender = sender.ok_or(Error::InvalidSrcLocation)?;
                Ok(vec![self.handle_section_knowledge_query(
                    last_known_key,
                    returned_msg,
                    sender,
                    src_name,
                    msg.src.src_location().to_dst(),
                )?])
            }
            Variant::DkgStart {
                dkg_key,
                elder_candidates,
            } => self.handle_dkg_start(dkg_key, elder_candidates),
            Variant::DkgMessage { dkg_key, message } => {
                self.handle_dkg_message(dkg_key, message, src_name)
            }
            Variant::DkgFailureObservation {
                dkg_key,
                sig,
                failed_participants,
            } => self.handle_dkg_failure_observation(dkg_key, &failed_participants, sig),
            Variant::DkgFailureAgreement(sig_set) => {
                self.handle_dkg_failure_agreement(&src_name, &sig_set)
            }
            Variant::Propose { content, sig_share } => {
                let mut commands = vec![];
                let result = self.handle_proposal(content, sig_share.clone());

                if let Some(addr) = sender {
                    commands.extend(self.check_lagging((src_name, addr), &sig_share)?);
                }

                commands.extend(result?);
                Ok(commands)
            }
            Variant::JoinResponse(join_response) => {
                debug!("Ignoring unexpected message: {:?}", join_response);
                Ok(vec![])
            }
            Variant::JoinAsRelocatedResponse(_) => {
                match (sender, self.relocate_state.as_mut()) {
                    (
                        Some(sender),
                        Some(RelocateState::InProgress(ref mut joining_as_relocated)),
                    ) => {
                        if let Some(cmd) = joining_as_relocated
                            .handle_join_response(msg, sender)
                            .await?
                        {
                            return Ok(vec![cmd]);
                        }
                    }
                    (Some(_), _) => {}
                    (None, _) => error!("Missing sender of {:?}", msg),
                }

                Ok(vec![])
            }
        }
    }

    fn handle_section_knowledge_query(
        &self,
        given_key: Option<bls::PublicKey>,
        msg: Box<RoutingMsg>,
        sender: SocketAddr,
        src_name: XorName,
        dst_location: DstLocation,
    ) -> Result<Command> {
        let chain = self.section.chain();
        let given_key = if let Some(key) = given_key {
            key
        } else {
            *self.section_chain().root_key()
        };
        let truncated_chain = chain.get_proof_chain_to_current(&given_key)?;
        let section_auth = self.section.section_signed_authority_provider();
        let variant = Variant::SectionKnowledge {
            src_info: (section_auth.clone(), truncated_chain),
            msg: Some(msg),
        };

        let msg = RoutingMsg::single_src(
            self.node(),
            dst_location,
            variant,
            self.section.authority_provider().section_key(),
        )?;
        let key = self.section_key_by_name(&src_name);
        Ok(Command::send_message_to_node(
            (src_name, sender),
            msg,
            DstInfo {
                dst: src_name,
                dst_section_pk: key,
            },
        ))
    }

    pub(crate) fn verify_message(&self, msg: &RoutingMsg) -> Result<bool> {
        let known_keys: Vec<bls::PublicKey> = self
            .section
            .chain()
            .keys()
            .copied()
            .chain(self.network.keys().map(|(_, key)| key))
            .chain(iter::once(*self.section.genesis_key()))
            .collect();

        match msg.verify(known_keys.iter()) {
            Ok(VerifyStatus::Full) => Ok(true),
            Ok(VerifyStatus::Unknown) => Ok(false),
            Err(error) => {
                warn!("Verification of {:?} failed: {}", msg, error);
                Err(error)
            }
        }
    }

    async fn handle_user_message(
        &mut self,
        msg: RoutingMsg,
        content: Bytes,
    ) -> Result<Vec<Command>> {
        trace!("handle user message {:?}", msg);
        if let DstLocation::EndUser(EndUser {
            xorname: xor_name,
            socket_id,
        }) = msg.dst
        {
            if let Some(socket_addr) = self.get_socket_addr(socket_id).copied() {
                trace!("sending user message {:?} to client {:?}", msg, socket_addr);
                return Ok(vec![Command::SendMessage {
                    recipients: vec![(xor_name, socket_addr)],
                    delivery_group_size: 1,
                    message: MessageType::Client {
                        msg: ClientMsg::from(content)?,
                        dst_info: DstInfo {
                            dst: xor_name,
                            dst_section_pk: *self.section.chain().last_key(),
                        },
                    },
                }]);
            } else {
                trace!(
                    "Cannot route user message, socket id not found for {:?}",
                    msg
                );
                return Err(Error::EmptyRecipientList);
            }
        }

        self.send_event(Event::MessageReceived {
            content,
            src: msg.src.src_location(),
            dst: msg.dst,
            sig: msg.keyed_sig(),
            section_pk: msg.section_pk,
        })
        .await;

        Ok(vec![])
    }

    pub(crate) async fn handle_sync(
        &mut self,
        section: Section,
        network: Network,
    ) -> Result<Vec<Command>> {
        if !section.prefix().matches(&self.node.name()) {
            trace!("ignore Sync - not our section");
            return Ok(vec![]);
        }

        let old_adults: BTreeSet<_> = self
            .section
            .live_adults()
            .map(|p| p.name())
            .copied()
            .collect();

        let snapshot = self.state_snapshot();
        trace!(
            "Updating knowledge of own section \n    elders: {:?} \n    members: {:?}",
            section.authority_provider(),
            section.members()
        );
        self.section.merge(section)?;
        self.network.merge(network, self.section.chain());

        if !self.is_elder() {
            let current_adults: BTreeSet<_> = self
                .section
                .live_adults()
                .map(|p| p.name())
                .copied()
                .collect();
            let added: BTreeSet<_> = current_adults.difference(&old_adults).copied().collect();
            let removed: BTreeSet<_> = old_adults.difference(&current_adults).copied().collect();

            if !added.is_empty() || !removed.is_empty() {
                self.send_event(Event::AdultsChanged {
                    remaining: old_adults.intersection(&current_adults).copied().collect(),
                    added,
                    removed,
                })
                .await;
            }
        }

        self.update_state(snapshot).await
    }

    pub(crate) fn handle_join_request(
        &mut self,
        peer: Peer,
        join_request: JoinRequest,
    ) -> Result<Vec<Command>> {
        debug!("Received {:?} from {}", join_request, peer);

        if !self.section.prefix().matches(peer.name())
            || join_request.section_key != *self.section.chain().last_key()
        {
            debug!(
                "JoinRequest from {} - name doesn't match our prefix {:?}.",
                peer,
                self.section.prefix()
            );

            let variant = Variant::JoinResponse(Box::new(JoinResponse::Retry(
                self.section.authority_provider().clone(),
            )));
            trace!("Sending {:?} to {}", variant, peer);
            return Ok(vec![self.send_direct_message(
                (*peer.name(), *peer.addr()),
                variant,
                *self.section.chain().last_key(),
            )?]);
        }

        if self.section.members().is_joined(peer.name()) {
            debug!(
                "Ignoring JoinRequest from {} - already member of our section.",
                peer
            );
            return Ok(vec![]);
        }

        if !self.joins_allowed {
            debug!(
                "Rejecting JoinRequest from {} - joins currently not allowed.",
                peer,
            );
            let variant = Variant::JoinResponse(Box::new(JoinResponse::Rejected(
                JoinRejectionReason::JoinsDisallowed,
            )));

            trace!("Sending {:?} to {}", variant, peer);
            return Ok(vec![self.send_direct_message(
                (*peer.name(), *peer.addr()),
                variant,
                *self.section.chain().last_key(),
            )?]);
        }

        // Start as Adult as long as passed resource signed.
        let mut age = MIN_ADULT_AGE;

        // During the first section, node shall use ranged age to avoid too many nodes got
        // relocated at the same time. After the first section got split, later on nodes shall
        // only start with age of MIN_ADULT_AGE
        if self.section.prefix().is_empty() {
            if peer.age() < FIRST_SECTION_MIN_AGE || peer.age() > FIRST_SECTION_MAX_AGE {
                debug!(
                    "Ignoring JoinRequest from {} - first-section node having wrong age {:?}",
                    peer,
                    peer.age(),
                );
                return Ok(vec![]);
            } else {
                age = peer.age();
            }
        } else if peer.age() != MIN_ADULT_AGE {
            // After section split, new node has to join with age of MIN_ADULT_AGE.
            let variant = Variant::JoinResponse(Box::new(JoinResponse::Retry(
                self.section.authority_provider().clone(),
            )));
            trace!("New node after section split must join with age of MIN_ADULT_AGE. Sending {:?} to {}", variant, peer);
            return Ok(vec![self.send_direct_message(
                (*peer.name(), *peer.addr()),
                variant,
                *self.section.chain().last_key(),
            )?]);
        }

        // Requires the node name matches the age.
        if age != peer.age() {
            debug!(
                "Ignoring JoinRequest from {} - required age {:?} not presented.",
                peer, age,
            );
            return Ok(vec![]);
        }

        // Require resource signed if joining as a new node.
        if let Some(response) = join_request.resource_proof_response {
            if !self.validate_resource_proof_response(peer.name(), response) {
                debug!(
                    "Ignoring JoinRequest from {} - invalid resource signed response",
                    peer
                );
                return Ok(vec![]);
            }
        } else {
            return Ok(vec![self.send_resource_proof_challenge(&peer)?]);
        }

        Ok(vec![Command::ProposeOnline {
            peer,
            previous_name: None,
            dst_key: None,
        }])
    }

    pub(crate) fn handle_join_as_relocated_request(
        &mut self,
        peer: Peer,
        join_request: JoinAsRelocatedRequest,
    ) -> Result<Vec<Command>> {
        debug!("Received {:?} from {}", join_request, peer);
        let payload = if let Some(payload) = join_request.relocate_payload {
            payload
        } else {
            let variant = Variant::JoinAsRelocatedResponse(Box::new(
                JoinAsRelocatedResponse::Retry(self.section.authority_provider().clone()),
            ));

            trace!("Sending {:?} to {}", variant, peer);
            return Ok(vec![self.send_direct_message(
                (*peer.name(), *peer.addr()),
                variant,
                *self.section.chain().last_key(),
            )?]);
        };

        if !self.section.prefix().matches(peer.name())
            || join_request.section_key != *self.section.chain().last_key()
        {
            debug!(
                "JoinAsRelocatedRequest from {} - name doesn't match our prefix {:?}.",
                peer,
                self.section.prefix()
            );

            let variant = Variant::JoinAsRelocatedResponse(Box::new(
                JoinAsRelocatedResponse::Retry(self.section.authority_provider().clone()),
            ));
            trace!("Sending {:?} to {}", variant, peer);
            return Ok(vec![self.send_direct_message(
                (*peer.name(), *peer.addr()),
                variant,
                *self.section.chain().last_key(),
            )?]);
        }

        if self.section.members().is_joined(peer.name()) {
            debug!(
                "Ignoring JoinAsRelocatedRequest from {} - already member of our section.",
                peer
            );
            return Ok(vec![]);
        }

        // This joining node is being relocated to us.
        if !payload.verify_identity(peer.name()) {
            debug!(
                "Ignoring JoinAsRelocatedRequest from {} - invalid signature.",
                peer
            );
            return Ok(vec![]);
        }

        let details = payload.relocate_details()?;

        if !self.section.prefix().matches(&details.dst) {
            debug!(
                "Ignoring JoinAsRelocatedRequest from {} - destination {} doesn't match \
                         our prefix {:?}.",
                peer,
                details.dst,
                self.section.prefix()
            );
            return Ok(vec![]);
        }

        if !self
            .verify_message(payload.details.signed_msg())
            .unwrap_or(false)
        {
            debug!("Ignoring JoinAsRelocatedRequest from {} - untrusted.", peer);
            return Ok(vec![]);
        }

        // Requires the node name matches the age.
        let age = details.age;
        if age != peer.age() {
            debug!(
                "Ignoring JoinAsRelocatedRequest from {} - required age {:?} not presented.",
                peer, age,
            );
            return Ok(vec![]);
        }

        let previous_name = Some(details.pub_id);
        let dst_key = Some(details.dst_key);

        Ok(vec![Command::ProposeOnline {
            peer,
            previous_name,
            dst_key,
        }])
    }

    // Generate a new section info based on the current set of members and if it differs from the
    // current elders, trigger a DKG.
    pub(crate) fn promote_and_demote_elders(&mut self) -> Result<Vec<Command>> {
        let mut commands = vec![];

        for info in self.section.promote_and_demote_elders(&self.node.name()) {
            commands.extend(self.send_dkg_start(info)?);
        }

        Ok(commands)
    }
}
