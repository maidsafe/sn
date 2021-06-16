// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use std::cmp;

use crate::messaging::{
    node::{
        KeyedSig, MembershipState, NodeState, PlainMessage, Proposal, RoutingMsg, SectionSigned,
        Variant,
    },
    DstInfo, DstLocation, SectionAuthorityProvider,
};
use crate::routing::{
    dkg::SectionSignedUtils,
    error::Result,
    messages::RoutingMsgUtils,
    network::NetworkUtils,
    peer::PeerUtils,
    routing_api::command::Command,
    section::{
        ElderCandidatesUtils, SectionAuthorityProviderUtils, SectionPeersUtils, SectionUtils,
    },
    Error, Event, MIN_AGE,
};
use secured_linked_list::SecuredLinkedList;
use xor_name::XorName;

use super::Core;

// Agreement
impl Core {
    pub(crate) async fn handle_agreement(
        &mut self,
        proposal: Proposal,
        sig: KeyedSig,
    ) -> Result<Vec<Command>> {
        debug!("handle agreement on {:?}", proposal);

        match proposal {
            Proposal::Online {
                node_state,
                previous_name,
                ..
            } => {
                self.handle_online_agreement(node_state, previous_name, sig)
                    .await
            }
            Proposal::Offline(node_state) => self.handle_offline_agreement(node_state, sig).await,
            Proposal::SectionInfo(section_auth) => {
                self.handle_section_info_agreement(section_auth, sig)
            }
            Proposal::OurElders(section_auth) => {
                self.handle_our_elders_agreement(section_auth, sig).await
            }
            Proposal::AccumulateAtSrc { message, .. } => {
                let dst_name = if let Some(name) = message.dst.name() {
                    name
                } else {
                    error!(
                        "Not handling AccumulateAtSrc {:?}: No dst_name found",
                        *message
                    );
                    return Err(Error::InvalidDstLocation);
                };
                let dst_section_pk = message.dst_key;
                Ok(vec![self.handle_accumulate_at_src_agreement(
                    *message,
                    self.section.chain().clone(),
                    sig,
                    DstInfo {
                        dst: dst_name,
                        dst_section_pk,
                    },
                )?])
            }
            Proposal::JoinsAllowed(joins_allowed) => {
                self.joins_allowed = joins_allowed.1;
                Ok(vec![])
            }
        }
    }

    async fn handle_online_agreement(
        &mut self,
        new_info: NodeState,
        previous_name: Option<XorName>,
        sig: KeyedSig,
    ) -> Result<Vec<Command>> {
        let mut commands = vec![];

        if let Some(old_info) = self
            .section
            .members()
            .get_section_signed(new_info.peer.name())
        {
            // This node is rejoin with same name.

            if old_info.value.state != MembershipState::Left {
                debug!(
                    "Ignoring Online node {} - {:?} not Left.",
                    new_info.peer.name(),
                    old_info.value.state,
                );

                return Ok(commands);
            }

            let new_age = cmp::max(MIN_AGE, old_info.value.peer.age() / 2);

            if new_age > MIN_AGE {
                // TODO: consider handling the relocation inside the bootstrap phase, to avoid
                // having to send this `NodeApproval`.
                commands.push(self.send_node_approval(old_info.clone())?);
                commands.extend(self.relocate_rejoining_peer(&old_info.value.peer, new_age)?);

                return Ok(commands);
            }
        }

        let new_info = SectionSigned {
            value: new_info,
            sig,
        };

        if !self.section.update_member(new_info.clone()) {
            info!("ignore Online: {:?}", new_info.value.peer);
            return Ok(vec![]);
        }

        info!("handle Online: {:?}", new_info.value.peer);

        self.send_event(Event::MemberJoined {
            name: *new_info.value.peer.name(),
            previous_name,
            age: new_info.value.peer.age(),
        })
        .await;

        commands.extend(self.relocate_peers(new_info.value.peer.name(), &new_info.sig.signature)?);

        let result = self.promote_and_demote_elders()?;
        if result.is_empty() {
            commands.extend(self.send_sync_to_adults()?);
        }

        commands.extend(result);
        commands.push(self.send_node_approval(new_info)?);

        self.print_network_stats();

        Ok(commands)
    }

    async fn handle_offline_agreement(
        &mut self,
        node_state: NodeState,
        sig: KeyedSig,
    ) -> Result<Vec<Command>> {
        let mut commands = vec![];

        let peer = node_state.peer;
        let age = peer.age();
        let signature = sig.signature.clone();

        if !self.section.update_member(SectionSigned {
            value: node_state,
            sig,
        }) {
            info!("ignore Offline: {:?}", peer);
            return Ok(commands);
        }

        info!("handle Offline: {:?}", peer);

        commands.extend(self.relocate_peers(peer.name(), &signature)?);

        let result = self.promote_and_demote_elders()?;
        if result.is_empty() {
            commands.extend(self.send_sync_to_adults()?);
        }

        commands.extend(result);

        self.send_event(Event::MemberLeft {
            name: *peer.name(),
            age,
        })
        .await;

        Ok(commands)
    }

    fn handle_section_info_agreement(
        &mut self,
        section_auth: SectionAuthorityProvider,
        sig: KeyedSig,
    ) -> Result<Vec<Command>> {
        let mut commands = vec![];

        let equal_or_extension = section_auth.prefix() == *self.section.prefix()
            || section_auth.prefix().is_extension_of(self.section.prefix());
        let section_auth = SectionSigned::new(section_auth, sig.clone());

        if equal_or_extension {
            // Our section of sub-section

            let infos = self.section.promote_and_demote_elders(&self.node.name());
            if !infos.contains(&section_auth.value.elder_candidates()) {
                // SectionInfo out of date, ignore.
                return Ok(commands);
            }

            // Send a `Sync` message to all the to-be-promoted members so they have the full
            // section and network data.
            let sync_recipients: Vec<_> = infos
                .iter()
                .flat_map(|info| info.peers())
                .filter(|peer| !self.section.is_elder(peer.name()))
                .map(|peer| (*peer.name(), *peer.addr()))
                .collect();
            if !sync_recipients.is_empty() {
                let sync_message = RoutingMsg::single_src(
                    &self.node,
                    DstLocation::DirectAndUnrouted,
                    Variant::Sync {
                        section: self.section.clone(),
                        network: self.network.clone(),
                    },
                    self.section.authority_provider().section_key(),
                )?;
                let len = sync_recipients.len();
                commands.push(Command::send_message_to_nodes(
                    sync_recipients,
                    len,
                    sync_message,
                    DstInfo {
                        dst: XorName::random(),
                        dst_section_pk: sig.public_key,
                    },
                ));
            }

            // Send the `OurElder` proposal to all of the to-be-elders so it's aggregated by them.
            let our_elders_recipients: Vec<_> =
                infos.iter().flat_map(|info| info.peers()).collect();
            commands.extend(
                self.send_proposal(&our_elders_recipients, Proposal::OurElders(section_auth))?,
            );
        } else {
            // Other section

            let _ = self
                .network
                .update_section(section_auth, None, self.section.chain());
        }

        Ok(commands)
    }

    async fn handle_our_elders_agreement(
        &mut self,
        section_auth: SectionSigned<SectionAuthorityProvider>,
        key_sig: KeyedSig,
    ) -> Result<Vec<Command>> {
        let updates = self
            .split_barrier
            .process(self.section.prefix(), section_auth, key_sig);
        if updates.is_empty() {
            return Ok(vec![]);
        }

        let snapshot = self.state_snapshot();

        for (section_auth, key_sig) in updates {
            if section_auth.value.prefix.matches(&self.node.name()) {
                let _ = self.section.update_elders(section_auth, key_sig);
            } else {
                let _ =
                    self.network
                        .update_section(section_auth, Some(key_sig), self.section.chain());
            }
        }

        self.update_state(snapshot).await
    }

    fn handle_accumulate_at_src_agreement(
        &self,
        message: PlainMessage,
        section_chain: SecuredLinkedList,
        sig: KeyedSig,
        dst_info: DstInfo,
    ) -> Result<Command> {
        let message = RoutingMsg::section_src(message, sig, section_chain)?;

        Ok(Command::HandleMessage {
            message,
            sender: None,
            dst_info,
        })
    }
}
