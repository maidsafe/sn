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

use qp2p::{ConnectionIncoming, IncomingConnections};
use tokio::{sync::mpsc, task};
use tracing::Instrument;

#[derive(Clone)]
pub(crate) struct MsgListener {
    receive_msg: mpsc::Sender<MsgFromPeer>,
}

impl MsgListener {
    pub(crate) fn new(receive_msg: mpsc::Sender<MsgFromPeer>) -> Self {
        Self { receive_msg }
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn listen_for_incoming_msgs(self, mut incoming_connections: IncomingConnections) {
        let _handle = task::spawn(async move {
            while let Some((connection, incoming_msgs)) = incoming_connections.next().await {
                trace!(
                    "{}: from {:?} with connection_id {}",
                    LogMarker::IncomingConnection,
                    connection.remote_address(),
                    connection.id()
                );

                let clone = self.clone();
                let _handle =
                    task::spawn(clone.listen(connection, incoming_msgs).in_current_span());
            }
        });
    }

    #[tracing::instrument(skip_all)]
    async fn listen(self, conn: qp2p::Connection, mut incoming_msgs: ConnectionIncoming) {
        let conn_id = conn.id();
        let remote_address = conn.remote_address();

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

                    let wire_msg = match WireMsg::from(msg_bytes.0) {
                        Ok(wire_msg) => wire_msg,
                        Err(error) => {
                            // TODO: should perhaps rather drop this connection.. as it is a spam vector
                            debug!("Failed to deserialize message received from {remote_address:?}{stream_info}: {error:?}");
                            continue;
                        }
                    };

                    let src_name = match wire_msg.kind() {
                        MsgKind::Client(auth) => auth.public_key.into(),
                        MsgKind::Node { name, .. }
                        | MsgKind::ClientDataResponse(name)
                        | MsgKind::NodeDataResponse(name) => *name,
                    };

                    let peer = Peer::new(src_name, remote_address);

                    let msg_id = wire_msg.msg_id();
                    debug!(
                        "Msg {msg_id:?} received, over conn_id={conn_id}, from: {peer:?}{stream_info} was: {wire_msg:?}"
                    );

                    let msg_sender = self.receive_msg.clone();
                    let msg = MsgFromPeer {
                        sender: peer,
                        wire_msg,
                        send_stream,
                    };
                    // move this channel sending off thread so we don't hold up incoming msgs at all.
                    let _handle = tokio::spawn(async move {
                        // handle the message first
                        if let Err(error) = msg_sender.send(msg).await {
                            error!("Error pushing msg {msg_id:?} onto internal msg handling channel: {error:?}");
                        }
                    });
                }
                Err(error) => {
                    warn!("Error on connection {conn_id} with {remote_address}: {error:?}");
                }
            }
        }

        trace!(%conn_id, %remote_address, "{}", LogMarker::ConnectionClosed);
    }
}
