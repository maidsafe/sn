// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::super::Core;
use crate::messaging::{
    node::{RoutingMsg, Variant},
    section_info::{GetSectionResponse, SectionInfoMsg},
    DstInfo, DstLocation, MessageType, SectionAuthorityProvider,
};
use crate::routing::{
    error::Result,
    messages::RoutingMsgUtils,
    network::NetworkUtils,
    peer::PeerUtils,
    routing_api::command::Command,
    section::{SectionAuthorityProviderUtils, SectionUtils},
};
use std::net::SocketAddr;
use xor_name::XorName;

// Message handling
impl Core {
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
        }
    }

    pub(crate) fn handle_section_knowledge_query(
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
}
