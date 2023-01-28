// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{core::NodeContext, Cmd, Error, MyNode, Result};

use sn_interface::{
    messaging::{
        data::ClientDataResponse,
        system::{NodeDataCmd, NodeDataResponse, NodeMsg},
        Dst, MsgId, MsgKind, WireMsg,
    },
    types::Peer,
};

use qp2p::SendStream;
use xor_name::XorName;

use bytes::Bytes;
use lazy_static::lazy_static;
use std::{collections::BTreeSet, env::var, str::FromStr};
use tokio::time::Duration;

/// Environment variable to set timeout value (in seconds) for data queries
/// forwarded to Adults. Default value (`NODE_RESPONSE_DEFAULT_TIMEOUT`) is otherwise used.
const ENV_NODE_RESPONSE_TIMEOUT: &str = "SN_NODE_RESPONSE_TIMEOUT";

// Default timeout period set for data queries forwarded to Adult.
// TODO: how to determine this time properly?
const NODE_RESPONSE_DEFAULT_TIMEOUT: Duration = Duration::from_secs(70);

lazy_static! {
    static ref NODE_RESPONSE_TIMEOUT: Duration = match var(ENV_NODE_RESPONSE_TIMEOUT)
        .map(|v| u64::from_str(&v))
    {
        Ok(Ok(secs)) => {
            let timeout = Duration::from_secs(secs);
            info!("{ENV_NODE_RESPONSE_TIMEOUT} env var set, Node data query response timeout set to {timeout:?}");
            timeout
        }
        Ok(Err(err)) => {
            warn!(
                "Failed to parse {ENV_NODE_RESPONSE_TIMEOUT} value, using \
                default value ({NODE_RESPONSE_DEFAULT_TIMEOUT:?}): {err:?}"
            );
            NODE_RESPONSE_DEFAULT_TIMEOUT
        }
        Err(_) => NODE_RESPONSE_DEFAULT_TIMEOUT,
    };
}

// Message handling over streams
impl MyNode {
    pub(crate) async fn send_node_msg_response(
        msg: NodeMsg,
        msg_id: MsgId,
        recipient: Peer,
        context: NodeContext,
        send_stream: SendStream,
    ) -> Result<Option<Cmd>> {
        let stream_id = send_stream.id();
        trace!("Sending response msg {msg_id:?} over {stream_id}");
        let (kind, payload) = MyNode::serialize_node_msg(context.name, &msg)?;
        send_msg_on_stream(
            context.network_knowledge.section_key(),
            payload,
            kind,
            send_stream,
            recipient,
            msg_id,
        )
        .await
    }

    pub(crate) async fn send_client_response(
        msg: ClientDataResponse,
        correlation_id: MsgId,
        send_stream: SendStream,
        context: NodeContext,
        source_client: Peer,
    ) -> Result<Option<Cmd>> {
        trace!("Sending client response msg for {correlation_id:?}");
        let (kind, payload) = MyNode::serialize_client_msg_response(context.name, &msg)?;
        send_msg_on_stream(
            context.network_knowledge.section_key(),
            payload,
            kind,
            send_stream,
            source_client,
            correlation_id,
        )
        .await
    }

    pub(crate) async fn send_node_data_response(
        msg: NodeDataResponse,
        send_stream: SendStream,
        context: NodeContext,
        requesting_peer: Peer,
    ) -> Result<Option<Cmd>> {
        trace!("Sending data response to msg {:?}..", msg.correlation_id());
        let (kind, payload) = MyNode::serialize_node_data_response(context.name, &msg)?;
        send_msg_on_stream(
            context.network_knowledge.section_key(),
            payload,
            kind,
            send_stream,
            requesting_peer,
            *msg.correlation_id(),
        )
        .await
    }

    /// Sends a msg via comms, and listens for any response
    /// The response is returned to be handled via the dispatcher (though a response is not necessarily expected)
    pub(crate) fn send_msg_with_bi_response(
        msg: NodeMsg,
        msg_id: MsgId,
        context: NodeContext,
        recipients: BTreeSet<Peer>,
    ) -> Result<()> {
        let targets_len = recipients.len();
        debug!("Sending out + awaiting response of {msg_id:?} to {targets_len} holder node/s {recipients:?}");

        let (kind, payload) = MyNode::serialize_node_msg(context.name, &msg)?;

        // We create a Dst with random dst name, but we'll update it accordingly for each target
        let mut dst = Dst {
            name: XorName::default(),
            section_key: context.network_knowledge.section_key(),
        };
        let mut wire_msg = WireMsg::new_msg(msg_id, payload, kind, dst);
        let _bytes = wire_msg.serialize_and_cache_bytes()?;

        for target in recipients {
            dst.name = target.name();
            let bytes_to_node = wire_msg.serialize_with_new_dst(&dst)?;
            let comm = context.comm.clone();
            info!("About to send {msg_id:?} to holder node: {target:?}");
            comm.send_with_bi_response(target, msg_id, bytes_to_node);
        }

        Ok(())
    }

    pub(crate) fn send_msg_await_response_and_send_to_client(
        msg_id: MsgId,
        msg: NodeMsg,
        context: NodeContext,
        targets: BTreeSet<Peer>,
        client_stream: SendStream,
        source_client: Peer,
    ) -> Result<()> {
        let targets_len = targets.len();
        debug!("Sending out {msg_id:?} to {targets_len} holder node/s {targets:?}");

        let (kind, payload) = MyNode::serialize_node_msg(context.name, &msg)?;

        // We create a Dst with random dst name, but we'll update it accordingly for each target
        let mut dst = Dst {
            name: XorName::default(),
            section_key: context.network_knowledge.section_key(),
        };
        let mut wire_msg = WireMsg::new_msg(msg_id, payload, kind, dst);

        let _bytes = wire_msg.serialize_and_cache_bytes()?;

        let node_bytes = targets
            .into_iter()
            .filter_map(|target| {
                dst.name = target.name();
                wire_msg
                    .serialize_with_new_dst(&dst)
                    .ok()
                    .map(|bytes_to_node| (target, bytes_to_node))
            })
            .collect();

        use sn_interface::messaging::system::NodeMsgType::*;
        let msg_type = match msg {
            NodeMsg::NodeDataQuery(_) => DataQuery,
            NodeMsg::NodeDataCmd(NodeDataCmd::StoreData(_)) => StoreData,
            _ => return Err(Error::InvalidMessage),
        };
        let dst = Dst {
            name: source_client.name(),
            section_key: context.network_knowledge.section_key(),
        };

        context
            .comm
            .send_and_respond_on_stream(msg_id, msg_type, node_bytes, (dst, client_stream));

        Ok(())
    }
}

// Send a msg on a given stream
async fn send_msg_on_stream(
    section_key: bls::PublicKey,
    payload: Bytes,
    kind: MsgKind,
    mut send_stream: SendStream,
    target_peer: Peer,
    correlation_id: MsgId,
) -> Result<Option<Cmd>> {
    let dst = Dst {
        name: target_peer.name(),
        section_key,
    };
    let msg_id = MsgId::new();
    let wire_msg = WireMsg::new_msg(msg_id, payload, kind, dst);
    let bytes = wire_msg.serialize().map_err(|_| Error::InvalidMessage)?;

    let stream_id = send_stream.id();
    trace!("Sending response {msg_id:?} of msg {correlation_id:?}, to {target_peer:?} over {stream_id}");

    let stream_prio = 10;
    send_stream.set_priority(stream_prio);
    trace!("Prio set on stream {stream_id}, response {msg_id:?} of msg {correlation_id:?}, to {target_peer:?}");

    if let Err(error) = send_stream.send_user_msg(bytes).await {
        error!(
            "Could not send response {msg_id:?} of msg {correlation_id:?}, to {target_peer:?} \
            over response {stream_id}: {error:?}"
        );
        return Ok(Some(Cmd::HandleCommsError {
            peer: target_peer,
            error: sn_comms::Error::from(error),
        }));
    }

    trace!(
        "Sent: Response {msg_id:?} of msg {correlation_id:?} to {target_peer:?}, over {stream_id}."
    );

    // unblock + move finish off thread as it's not strictly related to the sending of the msg.
    let stream_id_clone = stream_id.clone();
    let _handle = tokio::spawn(async move {
        // Attempt to gracefully terminate the stream.
        // If this errors it does _not_ mean our message has not been sent
        let result = send_stream.finish().await;
        trace!("Response {msg_id:?} of msg {correlation_id:?} sent to {target_peer:?} over {stream_id_clone}. Stream finished with result: {result:?}");
    });

    debug!("Sent the response {msg_id:?} of msg {correlation_id:?} to {target_peer:?} over {stream_id}");

    Ok(None)
}
