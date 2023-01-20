// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::MsgFromPeer;

use sn_interface::{
    messaging::{MsgKind, WireMsg},
    types::{log_markers::LogMarker, Peer},
};

use qp2p::ConnectionIncoming;
use std::sync::Arc;
use tokio::{sync::mpsc, task};
use tracing::Instrument;

#[derive(Debug)]
pub(crate) enum ConnectionEvent {
    Connected {
        peer: Peer,
        connection: Arc<qp2p::Connection>,
    },
    ConnectionClosed {
        peer: Peer,
        connection: Arc<qp2p::Connection>,
    },
}

#[derive(Clone)]
pub(crate) struct MsgListener {
    connection_events: mpsc::Sender<ConnectionEvent>,
    receive_msg: mpsc::Sender<MsgFromPeer>,
}

impl MsgListener {
    pub(crate) fn new(
        connection_events: mpsc::Sender<ConnectionEvent>,
        receive_msg: mpsc::Sender<MsgFromPeer>,
    ) -> Self {
        Self {
            connection_events,
            receive_msg,
        }
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn listen(&self, conn: Arc<qp2p::Connection>, incoming_msgs: ConnectionIncoming) {
        let clone = self.clone();
        let _ = task::spawn(clone.listen_internal(conn, incoming_msgs).in_current_span());
    }

    #[tracing::instrument(skip_all)]
    async fn listen_internal(
        self,
        conn: Arc<qp2p::Connection>,
        mut incoming_msgs: ConnectionIncoming,
    ) {
        let conn_id = conn.id();
        let remote_address = conn.remote_address();
        let mut node_conn_cached = None;

        while let Some(result) = incoming_msgs.next_with_stream().await.transpose() {
            match result {
                Ok((msg_bytes, send_stream)) => {
                    let stream_info = if let Some(stream) = &send_stream {
                        format!(" on {}", stream.id())
                    } else {
                        "".to_string()
                    };
                    debug!(
                        "New msg arrived over conn_id={conn_id} from {remote_address:?}{stream_info}"
                    );

                    let wire_msg = match WireMsg::from(msg_bytes) {
                        Ok(wire_msg) => wire_msg,
                        Err(error) => {
                            // TODO: should perhaps rather drop this connection.. as it is a spam vector
                            debug!("Failed to deserialize message received from {remote_address:?}{stream_info}: {error:?}");
                            continue;
                        }
                    };
                    let mut is_from_client = false;
                    let mut is_node_join_msg = false;
                    let src_name = match wire_msg.kind() {
                        MsgKind::Client(auth) => {
                            is_from_client = true;
                            auth.public_key.into()
                        }
                        MsgKind::Node { name, is_join } => {
                            is_node_join_msg = *is_join;
                            *name
                        }
                        MsgKind::ClientDataResponse(name) | MsgKind::NodeDataResponse(name) => {
                            *name
                        }
                    };

                    let peer = Peer::new(src_name, remote_address);

                    let msg_id = wire_msg.msg_id();
                    debug!(
                        "Msg {msg_id:?} received, over conn_id={conn_id}, from: {peer:?}{stream_info} was: {wire_msg:?}"
                    );

                    let msg_sender = self.receive_msg.clone();
                    let connection_event_sender = self.connection_events.clone();
                    let connection = conn.clone();

                    if node_conn_cached.is_none() && !is_from_client && !is_node_join_msg {
                        node_conn_cached = Some(peer);
                    }

                    // move this channel sending off thread so we dont hold up incoming msgs at all.
                    let _handle = tokio::spawn(async move {
                        // handle the message first
                        if let Err(error) = msg_sender
                            .send(MsgFromPeer {
                                sender: peer,
                                wire_msg,
                                send_stream,
                            })
                            .await
                        {
                            error!("Error pushing msg {msg_id:?} onto internal msg handling channel: {error:?}");
                        }

                        // we don't want to store PeerSessions from clients
                        if node_conn_cached.is_none() && !is_from_client && !is_node_join_msg {
                            let _ = connection_event_sender
                                .send(ConnectionEvent::Connected { peer, connection })
                                .await;
                        }
                    });
                }
                Err(error) => {
                    warn!("Error on connection {conn_id} with {remote_address}: {error:?}");
                }
            }
        }

        trace!(%conn_id, %remote_address, "{}", LogMarker::ConnectionClosed);

        // if the connection was from a (non client) peer was cached, we shall remove it
        if let Some(peer) = node_conn_cached {
            trace!("Removing connection {conn_id} with node {peer} from cache");
            let _ = self
                .connection_events
                .send(ConnectionEvent::ConnectionClosed {
                    peer,
                    connection: conn,
                })
                .await;
        }
    }
}
