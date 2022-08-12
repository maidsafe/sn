// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod listeners;
mod messaging;

use crate::Result;
use sn_interface::{
    messaging::{
        data::{CmdError, OperationId, QueryResponse},
        MsgId,
    },
    network_knowledge::prefix_map::NetworkPrefixMap,
    types::PeerLinks,
};

use dashmap::DashMap;
use qp2p::{Config as QuicP2pConfig, Endpoint};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{mpsc::Sender, RwLock};

// Here we dont track the msg_id across the network, but just use it as a local identifier to remove the correct listener
type PendingQueryResponses = Arc<DashMap<OperationId, Vec<(MsgId, QueryResponseSender)>>>;
type QueryResponseSender = Sender<QueryResponse>;

type CmdResponse = (SocketAddr, Option<CmdError>);
type PendingCmdAcks = Arc<DashMap<MsgId, Sender<CmdResponse>>>;

#[derive(Debug)]
pub struct QueryResult {
    pub response: QueryResponse,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug)]
pub(super) struct Session {
    // Session endpoint.
    endpoint: Endpoint,
    // Channels for sending responses to upper layers
    pending_queries: PendingQueryResponses,
    // Channels for sending CmdAck to upper layers
    pending_cmds: PendingCmdAcks,
    /// All elders we know about from AE messages
    pub(super) network: Arc<RwLock<NetworkPrefixMap>>,
    /// Standard time to await potential AE messages:
    cmd_ack_wait: Duration,
    /// Links to nodes
    peer_links: PeerLinks,
}

impl Session {
    /// Acquire a session by bootstrapping to a section, maintaining connections to several nodes.
    #[instrument(level = "debug")]
    pub(crate) fn new(
        qp2p_config: QuicP2pConfig,
        local_addr: SocketAddr,
        cmd_ack_wait: Duration,
        prefix_map: NetworkPrefixMap,
    ) -> Result<Session> {
        let endpoint = Endpoint::new_client(local_addr, qp2p_config)?;
        let peer_links = PeerLinks::new(endpoint.clone());

        let session = Session {
            pending_queries: Arc::new(DashMap::default()),
            pending_cmds: Arc::new(DashMap::default()),
            endpoint,
            network: Arc::new(RwLock::new(prefix_map)),
            cmd_ack_wait,
            peer_links,
        };

        Ok(session)
    }
}
