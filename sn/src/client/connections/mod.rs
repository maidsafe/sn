// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod listeners;
mod messaging;
use crate::messaging::{
    data::{CmdError, OperationId, QueryResponse},
    MessageId,
};
use crate::peer::Peer;
use crate::prefix_map::NetworkPrefixMap;
use bls::PublicKey as BlsPublicKey;
use bytes::Bytes;
pub(crate) use messaging::SAFE_CLIENT_DIR;
use qp2p::Endpoint;
use std::path::PathBuf;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc::Sender, RwLock};
use tokio::time::Duration;
use uluru::LRUCache;
type QueryResponseSender = Sender<QueryResponse>;
type PendingQueryResponses = Arc<RwLock<HashMap<OperationId, QueryResponseSender>>>;

#[derive(Debug)]
pub struct QueryResult {
    pub response: QueryResponse,
    pub operation_id: OperationId,
}

pub(crate) type AeCache = LRUCache<(Vec<Peer>, BlsPublicKey, Bytes), 100>;

#[derive(Clone, Debug)]
pub(super) struct Session {
    // Session endpoint.
    endpoint: Endpoint,
    // Channels for sending responses to upper layers
    pending_queries: PendingQueryResponses,
    // Channels for sending errors to upper layer
    incoming_err_sender: Arc<Sender<CmdError>>,
    /// All elders we know about from AE messages
    network: Arc<NetworkPrefixMap>,
    /// AE redirect cache
    ae_redirect_cache: Arc<RwLock<AeCache>>,
    // AE retry cache
    ae_retry_cache: Arc<RwLock<AeCache>>,
    /// Network's genesis key
    genesis_key: bls::PublicKey,
    /// Initial network comms messageId
    initial_connection_check_msg_id: Arc<RwLock<Option<MessageId>>>,
    /// Standard time to await potential AE messages:
    standard_wait: Duration,
    /// Root storage dir for this session
    root_dir: PathBuf,
}
