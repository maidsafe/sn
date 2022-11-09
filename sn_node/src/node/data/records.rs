// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{Cmd, Error, MyNode, Prefix, Result};

use bytes::Bytes;
use futures::FutureExt;
use itertools::Itertools;
use qp2p::{SendStream, UsrMsgBytes};
use sn_dysfunction::IssueType;
use sn_interface::messaging::{MsgType, WireMsg};
use sn_interface::{
    data_copy_count,
    messaging::{
        data::{ClientMsg, CmdResponse, DataCmd, DataQuery, MetadataExchange, StorageLevel},
        system::{NodeCmd, NodeEvent, NodeMsg, NodeQuery, OperationId},
        AuthorityProof, ClientAuth, Dst, MsgId, MsgKind,
    },
    types::{log_markers::LogMarker, Peer, PublicKey, ReplicatedData},
};
use std::{cmp::Ordering, collections::BTreeSet, sync::Arc};
use tokio::sync::Mutex;
use tokio::time::Duration;
use tracing::info;
use xor_name::XorName;

const REPONSE_TIMEOUT: Duration = Duration::from_secs(7); // w/ 6s qp2p timeout, we give it just a bit more time
impl MyNode {
    // Locate ideal holders for this data, instruct them to store the data
    pub(crate) async fn replicate_data_to_adults(
        &self,
        data: ReplicatedData,
        msg_id: MsgId,
        targets: BTreeSet<Peer>,
    ) -> Result<Vec<(Peer, Result<WireMsg>)>> {
        info!(
            "Replicating data from {msg_id:?} {:?} to holders {:?}",
            data.name(),
            &targets,
        );

        // TODO: general ReplicateData flow could go bidi?
        // Right now we've a new msg for just one datum.
        // Atm that's perhaps more bother than its worth..
        let msg = NodeMsg::NodeCmd(NodeCmd::ReplicateOneData(data));

        let (kind, payload) = self.serialize_node_msg(msg)?;

        let mut send_tasks = vec![];

        for target in targets {
            let bytes_to_adult = self.form_usr_msg_bytes_to_node(
                payload.clone(),
                kind.clone(),
                Some(target),
                msg_id,
            )?;

            info!("About to send {msg_id:?} to holder: {:?}", &target,);

            send_tasks.push(
                async move {
                    (
                        target,
                        self.comm
                            .send_out_bytes_to_peer_and_return_response(
                                target,
                                msg_id,
                                bytes_to_adult.clone(),
                            )
                            .await,
                    )
                }
                .boxed(),
            );
        }

        Ok(futures::future::join_all(send_tasks).await)
    }

    // Locate ideal holders for this data, instruct them to store the data
    pub(crate) async fn replicate_data_to_adults_and_ack_to_client(
        &self,
        cmd: DataCmd,
        data: ReplicatedData,
        msg_id: MsgId,
        targets: BTreeSet<Peer>,
        client_response_stream: Arc<Mutex<SendStream>>,
    ) -> Result<()> {
        info!(
            "Replicating data from client {msg_id:?} {:?} to holders {:?}",
            data.name(),
            &targets,
        );

        let targets_len = targets.len();

        let responses = self.replicate_data_to_adults(data, msg_id, targets).await?;
        let mut success_count = 0;
        let mut ack_response = None;
        let mut last_error = None;
        for (peer, the_response) in responses {
            match the_response {
                Ok(response) => {
                    success_count += 1;
                    debug!("Response in from {peer:?} for {msg_id:?} {response:?}");
                    ack_response = Some(response);
                }
                Err(error) => {
                    error!("{msg_id:?} Error when replicating to adult {peer:?}: {error:?}");
                    last_error = Some(error);
                }
            }
        }

        // everything went fine, tell the client that
        if success_count == targets_len {
            if let Some(response) = ack_response {
                self.send_ack_to_client_on_stream(response, msg_id, client_response_stream.clone())
                    .await?;
            } else {
                // This should not be possible with above checks
                error!("No valid ack response to send from all responses for {msg_id:?}")
            }
        } else {
            error!("Storage was not completely successful for {msg_id:?}");

            if let Some(error) = last_error {
                self.send_cmd_error_response_over_stream(
                    cmd,
                    error,
                    msg_id,
                    client_response_stream,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Parses WireMsg and if DataStored Ack, we send a response to the client
    async fn send_ack_to_client_on_stream(
        &self,
        response: WireMsg,
        msg_id: MsgId,
        send_stream: Arc<Mutex<SendStream>>,
    ) -> Result<()> {
        if let MsgType::Node {
            msg: NodeMsg::NodeEvent(NodeEvent::DataStored(_address)),
            ..
        } = response.into_msg()?
        {
            let client_msg = ClientMsg::CmdResponse {
                response: CmdResponse::StoreChunk(Ok(())),
                correlation_id: msg_id,
            };

            let (kind, payload) = self.serialize_sign_client_msg(client_msg)?;

            debug!("{msg_id:?} sending cmd response ack back to client");
            self.send_msg_on_stream(
                payload,
                kind,
                send_stream,
                None, // we shouldn't need this...
                msg_id,
            )
            .await
        } else {
            error!(
                "Unexpected reponse to query from node. To : {msg_id:?}; response: {response:?}"
            );
            // TODO: handle this bad response
            Ok(())
        }
    }

    /// Find target adult, sends a bidi msg, awaiting response, and then sends this on to the client
    pub(crate) async fn read_data_from_adult_and_respond_to_client(
        &self,
        query: DataQuery,
        msg_id: MsgId,
        auth: AuthorityProof<ClientAuth>,
        source_client: Peer,
        client_response_stream: Arc<Mutex<SendStream>>,
    ) -> Result<Vec<Cmd>> {
        // We generate the operation id to track the response from the Adult
        // by using the query msg id, which shall be unique per query.
        let operation_id = OperationId::from(&Bytes::copy_from_slice(msg_id.as_ref()));
        let address = query.variant.address();
        trace!(
            "{:?} preparing to query adults for data at {:?} with op_id: {:?}",
            LogMarker::DataQueryReceviedAtElder,
            address,
            operation_id
        );

        let targets = self.target_data_holders_including_full(address.name());

        // Query only the nth adult
        let target = if let Some(peer) = targets.iter().nth(query.adult_index) {
            *peer
        } else {
            debug!("No targets found for {msg_id:?}");
            let error = Error::InsufficientAdults {
                prefix: self.network_knowledge().prefix(),
                expected: query.adult_index as u8 + 1,
                found: targets.len() as u8,
            };

            self.query_error_response(
                error,
                &query.variant,
                source_client,
                msg_id,
                client_response_stream,
            )
            .await?;
            // TODO: do error processing
            return Ok(vec![]);
        };

        // Form a msg to our adult
        let msg = NodeMsg::NodeQuery(NodeQuery::Data {
            query: query.variant,
            auth: auth.into_inner(),
            operation_id,
        });

        let (kind, payload) = self.serialize_node_msg(msg)?;

        let bytes_to_adult =
            self.form_usr_msg_bytes_to_node(payload, kind, Some(target), msg_id)?;

        debug!("sending out {msg_id:?}");
        // TODO: how to determine this time?
        // TODO: don't use arbitrary time here. (But 3s is very realistic here under normal load)
        let response = match tokio::time::timeout(REPONSE_TIMEOUT, async {
            self.comm
                .send_out_bytes_to_peer_and_return_response(target, msg_id, bytes_to_adult)
                .await
        })
        .await
        {
            Ok(resp) => resp,
            Err(_elapsed) => {
                error!("No response before arbitrary timeout. Marking adult as dysfunctional");
                return Ok(vec![Cmd::TrackNodeIssueInDysfunction {
                    name: target.name(),
                    // TODO: no need for op id tracking here, this can be a simple counter
                    issue: IssueType::PendingRequestOperation(operation_id),
                }]);
            }
        }?;

        debug!("Response in from peer for query {msg_id:?} {response:?}");

        if let MsgType::Node {
            msg: NodeMsg::NodeQueryResponse { response, .. },
            ..
        } = response.into_msg()?
        {
            let client_msg = ClientMsg::QueryResponse {
                response,
                correlation_id: msg_id,
            };

            let (kind, payload) = self.serialize_sign_client_msg(client_msg)?;

            self.send_msg_on_stream(payload, kind, client_response_stream, Some(target), msg_id)
                .await?;
        } else {
            error!(
                "Unexpected reponse to query from node. To : {msg_id:?}; response: {response:?}"
            );
        }

        // Everything went okay, so no further cmds to handle
        Ok(vec![])
    }

    /// Send an OutgoingMsg on a given stream
    pub(crate) async fn send_msg_on_stream(
        &self,
        payload: Bytes,
        kind: MsgKind,
        send_stream: Arc<Mutex<SendStream>>,
        target_peer: Option<Peer>,
        original_msg_id: MsgId,
    ) -> Result<()> {
        // TODO why do we need dst here?
        let bytes = self.form_usr_msg_bytes_to_node(payload, kind, target_peer, original_msg_id)?;
        trace!("USING BIDI to send to msg {original_msg_id:?}! OH DEAR, FASTEN SEATBELTS");
        let stream_prio = 10;
        let mut send_stream = send_stream.lock().await;

        debug!("stream locked for {original_msg_id:?} to {target_peer:?}");
        send_stream.set_priority(stream_prio);
        debug!("prio set for {original_msg_id:?} to {target_peer:?}");
        if let Err(error) = send_stream.send_user_msg(bytes).await {
            error!(
                "Could not send query response {original_msg_id:?} to peer {target_peer:?} over response stream: {error:?}",

            );
            return Err(Error::from(error));
        }

        debug!("msg away for {original_msg_id:?} to {target_peer:?}");
        if let Err(error) = send_stream.finish().await {
            error!(
                        "Could not close response stream for {original_msg_id:?} to peer {target_peer:?}: {error:?}",
                    );
        }

        debug!("sent the msg over stream {original_msg_id:?} to {target_peer:?}");

        Ok(())
    }

    pub(crate) fn form_usr_msg_bytes_to_node(
        &self,
        payload: Bytes,
        kind: MsgKind,
        target: Option<Peer>,
        msg_id: MsgId,
    ) -> Result<UsrMsgBytes> {
        let dst_name = target.map_or(XorName::default(), |peer| peer.name());
        // we first generate the XorName
        let dst = Dst {
            name: dst_name,
            section_key: self.network_knowledge().section_key(),
        };

        #[allow(unused_mut)]
        let mut wire_msg = WireMsg::new_msg(msg_id, payload, kind, dst);

        #[cfg(feature = "test-utils")]
        let wire_msg = wire_msg.set_payload_debug(msg);

        wire_msg
            .serialize_and_cache_bytes()
            .map_err(|_| Error::InvalidMessage)
    }

    pub(crate) fn get_metadata_of(&self, prefix: &Prefix) -> MetadataExchange {
        // Load tracked adult_levels
        let adult_levels = self.capacity.levels_matching(*prefix);
        MetadataExchange { adult_levels }
    }

    pub(crate) fn set_adult_levels(&mut self, adult_levels: MetadataExchange) {
        let MetadataExchange { adult_levels } = adult_levels;
        self.capacity.set_adult_levels(adult_levels)
    }

    /// Registered holders not present in provided list of members
    /// will be removed from `adult_storage_info` and no longer tracked for liveness.
    pub(crate) fn liveness_retain_only(&mut self, members: BTreeSet<XorName>) -> Result<()> {
        // full adults
        self.capacity.retain_members_only(&members);
        // stop tracking liveness of absent holders
        self.dysfunction_tracking.retain_members_only(members);
        Ok(())
    }

    /// Adds the new adult to the Capacity and Liveness trackers.
    pub(crate) fn add_new_adult_to_trackers(&mut self, adult: XorName) {
        info!("Adding new Adult: {adult} to trackers");
        self.capacity.add_new_adult(adult);
        self.dysfunction_tracking.add_new_node(adult);
    }

    /// Set storage level of a given node.
    /// Returns whether the level changed or not.
    pub(crate) fn set_storage_level(&mut self, node_id: &PublicKey, level: StorageLevel) -> bool {
        info!("Setting new storage level..");
        let changed = self
            .capacity
            .set_adult_level(XorName::from(*node_id), level);
        let avg_usage = self.capacity.avg_usage();
        info!(
            "Avg storage usage among Adults is between {}-{} %",
            avg_usage * 10,
            (avg_usage + 1) * 10
        );
        changed
    }

    pub(crate) fn full_adults(&self) -> BTreeSet<XorName> {
        self.capacity.full_adults()
    }

    /// Construct list of adults that hold target data, including full nodes.
    /// List is sorted by distance from `target`.
    fn target_data_holders_including_full(&self, target: &XorName) -> BTreeSet<Peer> {
        let full_adults = self.full_adults();
        let adults = self.network_knowledge().adults();

        let mut candidates = adults
            .clone()
            .into_iter()
            .sorted_by(|lhs, rhs| target.cmp_distance(&lhs.name(), &rhs.name()))
            .filter(|peer| !full_adults.contains(&peer.name()))
            .take(data_copy_count())
            .collect::<BTreeSet<_>>();

        trace!(
            "Data holders of {:?} are non-full adults: {:?} and full adults: {:?}",
            target,
            candidates,
            full_adults
        );

        // Full adults that are close to the chunk, shall still be considered as candidates
        // to allow chunks stored to non-full adults can be queried when nodes become full.
        let candidates_clone = candidates.clone();
        let close_full_adults = if let Some(closest_not_full) = candidates_clone.iter().next() {
            full_adults
                .iter()
                .filter_map(|name| {
                    if target.cmp_distance(name, &closest_not_full.name()) == Ordering::Less {
                        // get the actual peer if closer
                        let mut the_closer_peer = None;
                        for adult in &adults {
                            if &adult.name() == name {
                                the_closer_peer = Some(adult)
                            }
                        }
                        the_closer_peer
                    } else {
                        None
                    }
                })
                .collect::<BTreeSet<_>>()
        } else {
            // In case there is no empty candidates, query all full_adults
            adults
                .iter()
                .filter(|peer| !full_adults.contains(&peer.name()))
                .collect::<BTreeSet<_>>()
        };

        candidates.extend(close_full_adults);
        candidates
    }

    /// Used to fetch the list of holders for given name of data. Excludes full nodes
    pub(crate) fn target_data_holders(&self, target: XorName) -> BTreeSet<Peer> {
        let full_adults = self.full_adults();
        trace!("full_adults = {}", full_adults.len());
        // TODO: reuse our_adults_sorted_by_distance_to API when core is merged into upper layer
        let adults = self.network_knowledge().adults();

        trace!("Total adults known about: {:?}", adults.len());

        let candidates = adults
            .into_iter()
            .sorted_by(|lhs, rhs| target.cmp_distance(&lhs.name(), &rhs.name()))
            .filter(|peer| !full_adults.contains(&peer.name()))
            .take(data_copy_count())
            .collect::<BTreeSet<_>>();

        trace!(
            "Target holders of {:?} are non-full adults: {:?} and full adults that were ignored: {:?}",
            target,
            candidates,
            full_adults
        );

        candidates
    }
}
