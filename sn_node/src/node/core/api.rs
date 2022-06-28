// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{delivery_group, Comm, Node};

use crate::node::{
    api::{cmds::Cmd, event_channel::EventSender},
    error::{Error, Result},
    Event,
};
use crate::UsedSpace;

use sn_interface::{
    messaging::WireMsg,
    network_knowledge::{NetworkKnowledge, NodeInfo, SectionAuthorityProvider, SectionKeyShare},
    types::log_markers::LogMarker,
};

use ed25519_dalek::Keypair;
use secured_linked_list::SecuredLinkedList;
use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf, sync::Arc};
use xor_name::XorName;

impl Node {
    // Creates `Core` for the first node in the network
    pub(crate) async fn first_node(
        comm: Comm,
        keypair: Arc<Keypair>,
        event_sender: EventSender,
        used_space: UsedSpace,
        root_storage_dir: PathBuf,
        genesis_sk_set: bls::SecretKeySet,
    ) -> Result<Self> {
        let info = NodeInfo {
            keypair: keypair.clone(),
            addr: comm.our_connection_info(),
        };

        let (section, section_key_share) =
            NetworkKnowledge::first_node(info.peer(), genesis_sk_set).await?;
        Self::new(
            comm,
            keypair.clone(),
            section,
            Some(section_key_share),
            event_sender,
            used_space,
            root_storage_dir,
        )
        .await
    }

    pub(crate) async fn relocate(
        &self,
        new_keypair: Arc<Keypair>,
        new_section: NetworkKnowledge,
    ) -> Result<()> {
        // we first try to relocate section info.
        self.network_knowledge.relocated_to(new_section).await?;

        let mut our_keypair = self.keypair.write().await;
        *our_keypair = new_keypair;

        Ok(())
    }

    pub(crate) fn network_knowledge(&self) -> &NetworkKnowledge {
        &self.network_knowledge
    }

    pub(crate) async fn section_chain(&self) -> SecuredLinkedList {
        self.network_knowledge.section_chain().await
    }

    /// Is this node an elder?
    pub(crate) async fn is_elder(&self) -> bool {
        self.network_knowledge.is_elder(&self.info().await.name())
    }

    pub(crate) async fn is_not_elder(&self) -> bool {
        !self.is_elder().await
    }

    /// Returns connection info of this node.
    pub(crate) fn our_connection_info(&self) -> SocketAddr {
        self.comm.our_connection_info()
    }

    /// Returns the current BLS public key set
    pub(crate) async fn public_key_set(&self) -> Result<bls::PublicKeySet> {
        Ok(self.key_share().await?.public_key_set)
    }

    /// Returns the SAP of the section matching the name.
    pub(crate) async fn matching_section(
        &self,
        name: &XorName,
    ) -> Result<SectionAuthorityProvider> {
        self.network_knowledge
            .section_by_name(name)
            .map_err(Error::from)
    }

    /// Returns our key share in the current BLS group if this node is a member of one, or
    /// `Error::MissingSecretKeyShare` otherwise.
    pub(crate) async fn key_share(&self) -> Result<SectionKeyShare> {
        let section_key = self.network_knowledge.section_key();
        self.section_keys_provider
            .key_share(&section_key)
            .await
            .map_err(Error::from)
    }

    pub(crate) async fn send_event(&self, event: Event) {
        self.event_sender.send(event).await
    }

    // ----------------------------------------------------------------------------------------
    //   ---------------------------------- Mut ------------------------------------------
    // ----------------------------------------------------------------------------------------

    pub(crate) async fn handle_dkg_timeout(&self, token: u64) -> Result<Vec<Cmd>> {
        self.dkg_voter.handle_timeout(
            &self.info().await,
            token,
            self.network_knowledge().section_key(),
        )
    }

    // Send message to peers on the network.
    pub(crate) async fn send_msg_to_nodes(&self, mut wire_msg: WireMsg) -> Result<Option<Cmd>> {
        let dst_location = wire_msg.dst_location();
        let (targets, dg_size) = delivery_group::delivery_targets(
            dst_location,
            &self.info().await.name(),
            &self.network_knowledge,
        )?;

        let target_name = dst_location.name();

        // To avoid loop: if destination is to Node, targets are multiple, self is an elder,
        //     self section prefix matches the destination name, then don't carry out a relay.
        if self.is_elder().await
            && targets.len() > 1
            && dst_location.is_to_node()
            && self.network_knowledge.prefix().matches(&target_name)
        {
            // This actually means being an elder, but we don't know the member yet. Which most likely
            // happens during the join process that a node's name is changed.
            // we just drop the message
            return Ok(None);
        }

        trace!(
            "relay {:?} to first {:?} of {:?} (Section PK: {:?})",
            wire_msg,
            dg_size,
            targets,
            wire_msg.src_section_pk(),
        );

        let dst_pk = self.section_key_by_name(&target_name).await;
        wire_msg.set_dst_section_pk(dst_pk);

        let cmd = Cmd::SendMsgDeliveryGroup {
            recipients: targets.into_iter().collect(),
            delivery_group_size: dg_size,
            wire_msg,
        };

        Ok(Some(cmd))
    }

    // Generate a new section info based on the current set of members, but
    // excluding the ones in the provided list. And if the outcome list of candidates
    // differs from the current elders, trigger a DKG.
    pub(crate) async fn promote_and_demote_elders_except(
        &self,
        excluded_names: &BTreeSet<XorName>,
    ) -> Result<Vec<Cmd>> {
        debug!("{}", LogMarker::TriggeringPromotionAndDemotion);
        let mut cmds = vec![];
        // TODO: move `promote_and_demote_elders` to Membership
        for session_id in self.promote_and_demote_elders(excluded_names).await? {
            cmds.extend(self.send_dkg_start(session_id).await?);
        }

        Ok(cmds)
    }
}
