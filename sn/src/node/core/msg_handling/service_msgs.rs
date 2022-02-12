// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::data_copy_count;
use crate::messaging::{
    data::{CmdError, DataCmd, DataQuery, Error as ErrorMsg, ServiceMsg},
    system::{NodeQueryResponse, SystemMsg},
    AuthorityProof, DstLocation, EndUser, MsgId, ServiceAuth, WireMsg,
};
use crate::node::{api::cmds::Cmd, core::Node, Result};
use crate::types::{log_markers::LogMarker, register::User, Peer, PublicKey, ReplicatedData};

use xor_name::XorName;

impl Node {
    /// Handle data query
    pub(crate) async fn handle_data_query_at_adult(
        &self,
        correlation_id: MsgId,
        query: &DataQuery,
        auth: ServiceAuth,
        user: EndUser,
        requesting_elder: XorName,
    ) -> Result<Vec<Cmd>> {
        trace!("Handling data query at adult");
        let mut cmds = vec![];

        let response = self
            .data_storage
            .query(query, User::Key(auth.public_key))
            .await;

        trace!("data query response at adult is:  {:?}", response);

        let msg = SystemMsg::NodeQueryResponse {
            response,
            correlation_id,
            user,
        };

        // Setup node authority on this response and send this back to our elders
        let section_pk = self.network_knowledge().section_key().await;
        let dst = DstLocation::Node {
            name: requesting_elder,
            section_pk,
        };

        cmds.push(Cmd::SignOutgoingSystemMsg { msg, dst });

        Ok(cmds)
    }

    /// Handle data read
    /// Records response in liveness tracking
    /// Forms a response to send to the requester
    pub(crate) async fn handle_data_query_response_at_elder(
        &self,
        // msg_id: MsgId,
        correlation_id: MsgId,
        response: NodeQueryResponse,
        user: EndUser,
        sending_node_pk: PublicKey,
    ) -> Result<Vec<Cmd>> {
        let msg_id = MsgId::new();
        let mut cmds = vec![];
        debug!(
            "Handling data read @ elders, received from {:?} ",
            sending_node_pk
        );

        let node_id = XorName::from(sending_node_pk);
        let op_id = response.operation_id()?;

        let waiting_peers = if let Some(peers) = self.pending_data_queries.remove(&op_id).await {
            peers
        } else {
            warn!(
                "Dropping chunk query response from Adult {}. We might have already forwarded this chunk to the requesting client orthe client connection cache has expired: {}",
                sending_node_pk, user.0
            );
            return Ok(cmds);
        };

        // Clear expired queries from the cache.
        self.pending_data_queries.remove_expired().await;

        let query_response = response.convert();

        let pending_removed = match query_response.operation_id() {
            Ok(op_id) => {
                self.liveness
                    .request_operation_fulfilled(&node_id, op_id)
                    .await
            }
            Err(error) => {
                warn!("Node problems noted when retrieving data: {:?}", error);
                false
            }
        };

        // Check for unresponsive adults here.
        for (name, count) in self.liveness.find_unresponsive_nodes().await {
            warn!(
                "Node {} has {} pending ops. It might be unresponsive",
                name, count
            );
            cmds.push(Cmd::ProposeOffline(name));
        }

        if !pending_removed {
            trace!("Ignoring un-expected response");
            return Ok(cmds);
        }

        // Send response if one is warrented
        if query_response.failed_with_data_not_found()
            || (!query_response.is_success()
                && self
                    .capacity
                    .is_full(&XorName::from(sending_node_pk))
                    .await
                    .unwrap_or(false))
        {
            // we don't return data not found errors.
            trace!("Node {:?}, reported data not found", sending_node_pk);

            return Ok(cmds);
        }

        let msg = ServiceMsg::QueryResponse {
            response: query_response,
            correlation_id,
        };
        let (msg_kind, payload) = self.ed_sign_client_msg(&msg).await?;

        for origin in waiting_peers {
            let dst = DstLocation::EndUser(EndUser(origin.name()));
            let wire_msg = WireMsg::new_msg(msg_id, payload.clone(), msg_kind.clone(), dst)?;

            trace!(
                "Responding with the first chunk query response to {:?}",
                dst
            );

            let command = Cmd::SendMsg {
                recipients: vec![origin],
                wire_msg,
            };
            cmds.push(command);
        }

        Ok(cmds)
    }

    /// Handle ServiceMsgs received from EndUser
    pub(crate) async fn handle_service_msg_received(
        &self,
        msg_id: MsgId,
        msg: ServiceMsg,
        auth: AuthorityProof<ServiceAuth>,
        origin: Peer,
    ) -> Result<Vec<Cmd>> {
        if self.is_not_elder().await {
            error!("Received unexpected message while Adult");
            return Ok(vec![]);
        }
        // extract the data from the request
        let data = match msg {
            // These reads/writes are for adult nodes...
            ServiceMsg::Cmd(DataCmd::Register(cmd)) => ReplicatedData::RegisterWrite(cmd),
            ServiceMsg::Cmd(DataCmd::StoreChunk(chunk)) => ReplicatedData::Chunk(chunk),
            ServiceMsg::Query(query) => {
                return self
                    .read_data_from_adults(query, msg_id, auth, origin)
                    .await
            }
            _ => {
                warn!("!!!! Unexpected ServiceMsg received in routing. Was not sent to node layer: {:?}", msg);
                return Ok(vec![]);
            }
        };
        // build the replication cmds
        let mut cmds = self.replicate_data(data).await?;
        // make sure the expected replication factor is achieved
        if data_copy_count() > cmds.len() {
            let error = CmdError::Data(ErrorMsg::InsufficientAdults {
                prefix: self.network_knowledge().prefix().await,
                expected: data_copy_count() as u8,
                found: cmds.len() as u8,
            });
            return self.send_cmd_error_response(error, origin, msg_id).await;
        }
        cmds.extend(self.send_cmd_ack(origin, msg_id).await?);
        Ok(cmds)
    }

    /// Handle incoming data msgs.
    pub(crate) async fn handle_service_msg(
        &self,
        msg_id: MsgId,
        msg: ServiceMsg,
        dst_location: DstLocation,
        auth: AuthorityProof<ServiceAuth>,
        user: Peer,
    ) -> Result<Vec<Cmd>> {
        trace!("{:?} {:?}", LogMarker::ServiceMsgToBeHandled, msg);
        if let DstLocation::EndUser(_) = dst_location {
            warn!(
                "Service msg has been dropped as its destination location ({:?}) is invalid: {:?}",
                dst_location, msg
            );
            return Ok(vec![]);
        }

        self.handle_service_msg_received(msg_id, msg, auth, user)
            .await
    }
}
