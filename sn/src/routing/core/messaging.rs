// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Core;
use super::ProposalUtils;
use crate::messaging::{
    system::{
        DkgSessionId, JoinResponse, Proposal, RelocateDetails, RelocatePromise, SectionAuth,
        SystemMsg,
    },
    DstLocation, WireMsg,
};
use crate::routing::{
    core::StateSnapshot,
    dkg::DkgSessionIdUtils,
    error::Result,
    log_markers::LogMarker,
    messages::WireMsgUtils,
    network_knowledge::{ElderCandidates, NodeState, SectionKeyShare},
    relocation::RelocateState,
    routing_api::command::Command,
    Peer,
};
use crate::types::PublicKey;
use bls::PublicKey as BlsPublicKey;
use xor_name::XorName;

impl Core {
    // Send proposal to all our elders.
    pub(crate) async fn propose(&self, proposal: Proposal) -> Result<Vec<Command>> {
        let elders: Vec<_> = self.network_knowledge.authority_provider().await.peers();
        self.send_proposal(elders, proposal).await
    }

    // Send `proposal` to `recipients`.
    pub(crate) async fn send_proposal(
        &self,
        recipients: Vec<Peer>,
        proposal: Proposal,
    ) -> Result<Vec<Command>> {
        let section_key = self.network_knowledge.section_key().await;

        let key_share = self
            .section_keys_provider
            .key_share(&section_key)
            .await
            .map_err(|err| {
                trace!("Can't propose {:?}: {:?}", proposal, err);
                err
            })?;

        self.send_proposal_with(recipients, proposal, &key_share)
            .await
    }

    pub(crate) async fn send_proposal_with(
        &self,
        recipients: Vec<Peer>,
        proposal: Proposal,
        key_share: &SectionKeyShare,
    ) -> Result<Vec<Command>> {
        trace!(
            "Propose {:?}, key_share: {:?}, aggregators: {:?}",
            proposal,
            key_share,
            recipients,
        );

        let sig_share = proposal.prove(
            key_share.public_key_set.clone(),
            key_share.index,
            &key_share.secret_key_share,
        )?;

        // Broadcast the proposal to the rest of the section elders.
        let node_msg = SystemMsg::Propose {
            proposal,
            sig_share,
        };
        // Name of the section_pk may not matches the section prefix.
        // Carry out a substitution to prevent the dst_location becomes other section.
        let section_key = self.network_knowledge.section_key().await;
        let wire_msg = WireMsg::single_src(
            &self.node.read().await.clone(),
            DstLocation::Section {
                name: self.network_knowledge.prefix().await.name(),
                section_pk: section_key,
            },
            node_msg,
            section_key,
        )?;

        Ok(self.send_or_handle(wire_msg, recipients).await)
    }

    // ------------------------------------------------------------------------------------------------------------
    // ------------------------------------------------------------------------------------------------------------

    /// Generate AntiEntropyUpdate message to update a peer with proof_chain,
    /// and members_info if required.
    pub(crate) async fn generate_ae_update(
        &self,
        _dst_section_key: BlsPublicKey,
        add_peer_info_to_update: bool,
    ) -> Result<SystemMsg> {
        let section_signed_auth = self
            .network_knowledge
            .section_signed_authority_provider()
            .await
            .clone();
        let section_auth = section_signed_auth.value;
        let section_signed = section_signed_auth.sig;

        let proof_chain = self.network_knowledge.chain().await;
        /* TODO: get a proof chain rather than whole chain once we have a DAG for our chain.
        let proof_chain = match self
            .network_knowledge
            .chain()
            .await
            .get_proof_chain_to_current(&dst_section_key)
        {
            Ok(chain) => chain,
            Err(_) => {
                // error getting chain from key, so lets send the whole thing
                self.network_knowledge.chain().await
            }
        };
        */

        let members = if add_peer_info_to_update {
            Some(
                self.network_knowledge
                    .members()
                    .iter()
                    .map(|state| state.clone().into_authed_msg())
                    .collect(),
            )
        } else {
            None
        };

        Ok(SystemMsg::AntiEntropyUpdate {
            section_auth: section_auth.into_msg(),
            section_signed,
            proof_chain,
            members,
        })
    }

    // Send NodeApproval to a joining node which makes them a section member
    pub(crate) async fn send_node_approval(
        &self,
        node_state: SectionAuth<NodeState>,
    ) -> Vec<Command> {
        let peer = node_state.to_peer();
        info!(
            "Our section with {:?} has approved peer {}.",
            self.network_knowledge.prefix().await,
            peer,
        );

        let node_msg = SystemMsg::JoinResponse(Box::new(JoinResponse::Approval {
            genesis_key: *self.network_knowledge.genesis_key(),
            section_auth: self
                .network_knowledge
                .section_signed_authority_provider()
                .await
                .into_authed_msg(),
            node_state: node_state.into_authed_msg(),
            section_chain: self.network_knowledge.chain().await,
        }));

        let dst_section_pk = self.network_knowledge.section_key().await;
        trace!("{}", LogMarker::SendNodeApproval);
        match self
            .send_direct_message(peer.clone(), node_msg, dst_section_pk)
            .await
        {
            Ok(cmd) => vec![cmd],
            Err(err) => {
                error!("Failed to send join approval to node {}: {:?}", peer, err);
                vec![]
            }
        }
    }

    pub(crate) async fn send_ae_update_to_our_section(&self) -> Vec<Command> {
        let our_name = self.node.read().await.name();
        let nodes: Vec<_> = self
            .network_knowledge
            .active_members()
            .await
            .into_iter()
            .filter(|peer| peer.name() != our_name)
            .collect();

        if nodes.is_empty() {
            warn!("No peers of our section found in our network knowledge to send AE-Update");
            return vec![];
        }

        // the PK is that of our section (as we know it; and we're ahead of our adults here)
        let dst_section_pk = self.network_knowledge.section_key().await;
        // the previous PK which is likely what adults know
        let previous_pk = *self.section_chain().await.prev_key();
        let node_msg = match self.generate_ae_update(previous_pk, true).await {
            Ok(node_msg) => node_msg,
            Err(err) => {
                warn!(
                    "Failed to generate AE-Update msg to send to our section's peers: {:?}",
                    err
                );
                return vec![];
            }
        };

        match self
            .send_direct_message_to_nodes(
                nodes,
                node_msg,
                self.network_knowledge.prefix().await.name(),
                dst_section_pk,
            )
            .await
        {
            Ok(cmd) => vec![cmd],
            Err(err) => {
                error!("Failed to send AE update to our section peers: {:?}", err);
                vec![]
            }
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn send_ae_update_to_sibling_section(
        &self,
        old: &StateSnapshot,
    ) -> Vec<Command> {
        debug!("{}", LogMarker::AeSendUpdateToSiblings);
        if let Some(sibling_sec_auth) = self
            .network_knowledge
            .prefix_map()
            .get_signed(&self.network_knowledge.prefix().await.sibling())
        {
            let promoted_sibling_elders: Vec<_> = sibling_sec_auth
                .peers()
                .into_iter()
                .filter(|peer| !old.elders.contains(&peer.name()))
                .collect();

            if promoted_sibling_elders.is_empty() {
                debug!("No promoted siblings found in our network knowledge to send AE-Update");
                return vec![];
            }

            // Using previous_key as dst_section_key as newly promoted sibling elders shall still
            // in the state of pre-split.
            let previous_pk = sibling_sec_auth.sig.public_key;

            // Compose a min sibling proof_chain.

            /* TODO: get a proof chain rather than whole chain once we have a DAG for our chain.
            let mut proof_chain = SecuredLinkedList::new(previous_pk);
            */
            let mut proof_chain = self.network_knowledge.chain().await;

            let _ = proof_chain.insert(
                &previous_pk,
                sibling_sec_auth.section_key(),
                sibling_sec_auth.sig.signature.clone(),
            );

            let dst_name = sibling_sec_auth.prefix().name();

            // Those promoted elders shall already know about other adult members.
            // TODO: confirm no need to populate the members.
            let node_msg = SystemMsg::AntiEntropyUpdate {
                section_signed: sibling_sec_auth.sig,
                section_auth: sibling_sec_auth.value.into_msg(),
                proof_chain,
                members: None,
            };

            match self
                .send_direct_message_to_nodes(
                    promoted_sibling_elders,
                    node_msg,
                    dst_name,
                    previous_pk,
                )
                .await
            {
                Ok(cmd) => vec![cmd],
                Err(err) => {
                    error!(
                        "Failed to send AE update to our promoted sibling elders: {:?}",
                        err
                    );
                    vec![]
                }
            }
        } else {
            error!("Failed to get sibling SAP during split.");
            vec![]
        }
    }

    pub(crate) async fn send_ae_update_to_adults(&self) -> Vec<Command> {
        let adults = self.network_knowledge.live_adults().await;

        let dst_section_pk = self.network_knowledge.section_key().await;
        let node_msg = match self.generate_ae_update(dst_section_pk, true).await {
            Ok(node_msg) => node_msg,
            Err(err) => {
                warn!(
                    "Failed to generate AE-Update msg to send to our section's Adults: {:?}",
                    err
                );
                return vec![];
            }
        };

        match self
            .send_direct_message_to_nodes(
                adults,
                node_msg,
                self.network_knowledge.prefix().await.name(),
                dst_section_pk,
            )
            .await
        {
            Ok(cmd) => vec![cmd],
            Err(err) => {
                error!(
                    "Failed to send AE update to our promoted sibling elders: {:?}",
                    err
                );
                vec![]
            }
        }
    }

    pub(crate) async fn send_relocate(
        &self,
        recipient: Peer,
        details: RelocateDetails,
    ) -> Result<Vec<Command>> {
        let src = details.pub_id;
        let dst = DstLocation::Node {
            name: details.pub_id,
            section_pk: self.network_knowledge.section_key().await,
        };
        let node_msg = SystemMsg::Relocate(details);

        self.send_message_for_dst_accumulation(src, dst, node_msg, vec![recipient])
            .await
    }

    pub(crate) async fn send_relocate_promise(
        &self,
        recipient: Peer,
        promise: RelocatePromise,
    ) -> Result<Vec<Command>> {
        // Note: this message is first sent to a single node who then sends it back to the section
        // where it needs to be handled by all the elders. This is why the destination is
        // `Section`, not `Node`.
        let src = promise.name;
        let dst = DstLocation::Section {
            name: promise.name,
            section_pk: self.network_knowledge.section_key().await,
        };
        let node_msg = SystemMsg::RelocatePromise(promise);

        self.send_message_for_dst_accumulation(src, dst, node_msg, vec![recipient])
            .await
    }

    pub(crate) async fn return_relocate_promise(&self) -> Option<Command> {
        // TODO: keep sending this periodically until we get relocated.
        if let Some(RelocateState::Delayed(msg)) = &*self.relocate_state.read().await {
            self.send_message_to_our_elders(msg.clone()).await.ok()
        } else {
            None
        }
    }

    pub(crate) async fn send_dkg_start(
        &self,
        elder_candidates: ElderCandidates,
    ) -> Result<Vec<Command>> {
        let src_prefix = elder_candidates.prefix();
        let generation = self.network_knowledge.chain_len().await;
        let session_id = DkgSessionId::new(&elder_candidates, generation);

        // Send DKG start to all candidates
        let recipients: Vec<_> = elder_candidates.elders().cloned().collect();

        trace!(
            "Send DkgStart for {:?} with {:?} to {:?}",
            elder_candidates,
            session_id,
            recipients
        );

        let node_msg = SystemMsg::DkgStart {
            session_id,
            prefix: elder_candidates.prefix(),
            elders: elder_candidates
                .elders()
                .map(|peer| (peer.name(), peer.addr()))
                .collect(),
        };
        let section_pk = self.network_knowledge.section_key().await;
        self.send_message_for_dst_accumulation(
            src_prefix.name(),
            DstLocation::Section {
                name: XorName::from(PublicKey::Bls(section_pk)),
                section_pk,
            },
            node_msg,
            recipients,
        )
        .await
    }

    pub(crate) async fn send_message_for_dst_accumulation(
        &self,
        src: XorName,
        dst: DstLocation,
        node_msg: SystemMsg,
        recipients: Vec<Peer>,
    ) -> Result<Vec<Command>> {
        let section_key = self.network_knowledge.section_key().await;

        let key_share = self
            .section_keys_provider
            .key_share(&section_key)
            .await
            .map_err(|err| {
                trace!(
                    "Can't create message {:?} for accumulation at dst {:?}: {:?}",
                    node_msg,
                    dst,
                    err
                );
                err
            })?;

        let wire_msg = WireMsg::for_dst_accumulation(&key_share, src, dst, node_msg, section_key)?;

        trace!(
            "Send {:?} for accumulation at dst to {:?}",
            wire_msg,
            recipients
        );

        Ok(self.send_or_handle(wire_msg, recipients).await)
    }

    // Send the message to all `recipients`. If one of the recipients is us, don't send it over the
    // network but handle it directly.
    pub(crate) async fn send_or_handle(
        &self,
        mut wire_msg: WireMsg,
        recipients: Vec<Peer>,
    ) -> Vec<Command> {
        let mut commands = vec![];
        let mut others = Vec::new();
        let mut handle = false;

        trace!("Send {:?} to {:?}", wire_msg, recipients);

        for recipient in recipients {
            if recipient.name() == self.node.read().await.name() {
                handle = true;
            } else {
                others.push(recipient);
            }
        }

        if !others.is_empty() {
            let dst_section_pk = self.section_key_by_name(&others[0].name()).await;
            wire_msg.set_dst_section_pk(dst_section_pk);

            trace!("{}", LogMarker::SendOrHandle);
            commands.push(Command::SendMessage {
                recipients: others,
                wire_msg: wire_msg.clone(),
            });
        }

        if handle {
            wire_msg.set_dst_section_pk(self.network_knowledge.section_key().await);
            wire_msg.set_dst_xorname(self.node.read().await.name());

            commands.push(Command::HandleMessage {
                sender_addr: self.node.read().await.addr,
                wire_msg,
                original_bytes: None,
            });
        }

        commands
    }

    pub(crate) async fn send_direct_message(
        &self,
        recipient: Peer,
        node_msg: SystemMsg,
        dst_section_pk: BlsPublicKey,
    ) -> Result<Command> {
        let wire_msg = WireMsg::single_src(
            &self.node.read().await.clone(),
            DstLocation::Section {
                name: recipient.name(),
                section_pk: dst_section_pk,
            },
            node_msg,
            self.network_knowledge
                .authority_provider()
                .await
                .section_key(),
        )?;

        trace!("{}", LogMarker::SendDirect);

        Ok(Command::SendMessage {
            recipients: vec![recipient],
            wire_msg,
        })
    }

    pub(crate) async fn send_direct_message_to_nodes(
        &self,
        recipients: Vec<Peer>,
        node_msg: SystemMsg,
        dst_name: XorName,
        dst_section_pk: BlsPublicKey,
    ) -> Result<Command> {
        let wire_msg = WireMsg::single_src(
            &self.node.read().await.clone(),
            DstLocation::Section {
                name: dst_name,
                section_pk: dst_section_pk,
            },
            node_msg,
            self.network_knowledge
                .authority_provider()
                .await
                .section_key(),
        )?;

        trace!("{}", LogMarker::SendDirectToNodes);

        Ok(Command::SendMessage {
            recipients,
            wire_msg,
        })
    }

    // TODO: consider changing this so it sends only to a subset of the elders
    // (say 1/3 of the ones closest to our name or so)
    pub(crate) async fn send_message_to_our_elders(&self, node_msg: SystemMsg) -> Result<Command> {
        let targets: Vec<_> = self.network_knowledge.authority_provider().await.peers();

        let dst_section_pk = self.network_knowledge.section_key().await;
        let cmd = self
            .send_direct_message_to_nodes(
                targets,
                node_msg,
                self.network_knowledge
                    .authority_provider()
                    .await
                    .prefix()
                    .name(),
                dst_section_pk,
            )
            .await?;

        Ok(cmd)
    }
}
