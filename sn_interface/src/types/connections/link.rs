// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Peer;

use crate::types::log_markers::LogMarker;
use qp2p::{Connection, ConnectionIncoming as IncomingMsgs, Endpoint, RecvStream, UsrMsgBytes};

// Required for docs
#[allow(unused_imports)]
use qp2p::RetryConfig;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use xor_name::XorName;

type ConnId = String;

/// A link to a peer in our network.
///
/// The upper layers will add incoming connections to the link,
/// and use the link to send msgs.
/// Using the link will open a connection if there is none there.
/// The link is a way to keep connections to a peer in one place
/// and use them efficiently; converge to a single one regardless of concurrent
/// comms initiation between the peers, and so on.
/// Unused connections will expire, so the Link is cheap to keep around.
/// The Link is kept around as long as the peer is deemed worth to keep contact with.
#[derive(Clone, Debug)]
pub struct Link {
    peer: Peer,
    endpoint: Endpoint,
    connections: Arc<RwLock<BTreeMap<ConnId, Connection>>>,
}

impl Link {
    pub(crate) fn new(peer: Peer, endpoint: Endpoint) -> Self {
        Self {
            peer,
            endpoint,
            connections: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub(crate) async fn new_with(id: Peer, endpoint: Endpoint, conn: Connection) -> Self {
        let instance = Self::new(id, endpoint);
        instance.insert(conn).await;
        instance
    }

    #[allow(unused)]
    pub(crate) fn name(&self) -> XorName {
        self.peer.name()
    }

    pub(crate) async fn add(&self, conn: Connection) {
        self.insert(conn).await;
    }

    /// Send a message to the peer with default retry configuration.
    ///
    /// The message will be sent on a unidirectional QUIC stream, meaning the application is
    /// responsible for correlating any anticipated responses from incoming streams.
    ///
    ///
    /// The priority will be `0` and retry behaviour will be determined by the
    /// [`RetryConfig`] that was used to construct the [`Endpoint`] this connection
    /// belongs to. See [`send_with`](Self::send_with) if you want to send a message with specific
    /// configuration.
    #[allow(unused)]
    pub async fn send<F: Fn(Connection, IncomingMsgs)>(
        &self,
        bytes: UsrMsgBytes,
        listen: F,
    ) -> Result<(), SendToOneError> {
        self.send_with(bytes, listen).await
    }

    /// Send a message to the peer using the given configuration.
    ///
    /// See [`send`](Self::send) if you want to send with the default configuration.
    #[instrument(skip_all)]
    pub async fn send_with<F: Fn(Connection, IncomingMsgs)>(
        &self,
        bytes: UsrMsgBytes,
        listen: F,
    ) -> Result<(), SendToOneError> {
        let default_priority = 10;
        let conn = self.get_or_connect_with_listener(listen).await?;
        trace!(
            "We have {} open connections to peer {}.",
            self.connections.read().await.len(),
            self.peer
        );

        // Simulate failed connections
        #[cfg(feature = "chaos")]
        {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let x: f64 = rng.gen_range(0.0..1.0);

            if x > 0.9 {
                warn!(
                    "\n =========== [Chaos] Connection fail chaos. Conection removed from Link w/ x of: {}. ============== \n",
                    x
                );

                // clean up failing connections at once, no nead to leak it outside of here
                // next send (e.g. when retrying) will use/create a new connection
                let id = &conn.id();

                {
                    let _ = self.connections.write().await.remove(id);
                }
                conn.close(Some(format!("{:?}", error)));
                Err(SendToOneError::ChaosNoConnection)
            }
        }

        match conn.send_with(bytes, default_priority, None).await {
            Ok(()) => Ok(()),
            Err(error) => {
                // clean up failing connections at once, no nead to leak it outside of here
                // next send (e.g. when retrying) will use/create a new connection
                let id = &conn.id();

                {
                    let _ = self.connections.write().await.remove(id);
                }
                Err(SendToOneError::Send(error))
            }
        }
    }

    pub async fn send_bi(&self, bytes: UsrMsgBytes) -> Result<RecvStream, SendToOneError> {
        // TODO: proper retry code setup
        let mut attempts = 1;
        while attempts <= 3 {
            attempts += 1;
            let conn = match self.get_or_connect().await {
                Ok(conn) => conn,
                Err(err) => {
                    error!("Err getting connection to {:?} during bi stream initialisation. Retrying...", self.peer);
                    if attempts > 3 {
                        return Err(err);
                    }
                    continue;
                }
            };

            let (mut send_stream, recv_stream) =
                conn.open_bi().await.map_err(SendToOneError::Connection)?;
            send_stream.set_priority(10);
            send_stream
                .send_user_msg(bytes.clone())
                .await
                .map_err(SendToOneError::Send)?;

            send_stream.finish().await.or_else(|err| match err {
                qp2p::SendError::StreamLost(qp2p::StreamError::Stopped(_)) => Ok(()),
                _ => Err(SendToOneError::Send(err)),
            })?;
            return Ok(recv_stream);
        }

        Err(SendToOneError::SendRepeatedlyFailed)
    }

    // gets a conn or creates one with a listener func
    async fn get_or_connect_with_listener<F: Fn(Connection, IncomingMsgs)>(
        &self,
        listen: F,
    ) -> Result<Connection, SendToOneError> {
        if self.connections.read().await.is_empty() {
            // read again
            // first caller will find none again, but the subsequent callers
            // will access only after the first one finished creating a new connection
            // thus will find a connection here:
            debug!("creating conn with {:?}", self.peer);
            self.create_connection_with_listener(listen).await
        } else {
            // let x = self.connections.read().await.iter().enumerate().filter(|(i, _)| i == 0).map(|(_,conn)|conn);
            if let Some((_id, conn)) = self.connections.read().await.iter().next() {
                return Ok(conn.clone());
            }

            // we should never hit here...
            // but if we do, we'll try making a conn
            self.create_connection_with_listener(listen).await
        }
    }

    // Get a connection or create a fresh one
    async fn get_or_connect(&self) -> Result<Connection, SendToOneError> {
        if self.connections.read().await.is_empty() {
            // read again
            // first caller will find none again, but the subsequent callers
            // will access only after the first one finished creating a new connection
            // thus will find a connection here:
            debug!("creating conn with {:?}", self.peer);
            self.create_connection().await
        } else {
            let mut dead_conns = vec![];
            let mut live_conn = None;

            while let Some((_id, conn)) = self.connections.read().await.iter().next() {
                // TODO: replace this with simple connection check when available.
                let is_connected = conn.open_bi().await.is_ok();

                // set up some cleanup
                if !is_connected {
                    dead_conns.push(conn.id());
                    continue;
                }

                // return the first live conn
                live_conn = Some(conn.clone());
            }

            // cleanup those dead conns
            for dead_conn_id in dead_conns {
                warn!("Dead connection to {:?} being removed from link", self.peer);
                let _conn = self.connections.write().await.remove(&dead_conn_id);
            }

            if let Some(conn) = live_conn {
                trace!("live connection found to {:?}", self.peer);
                Ok(conn)
            } else {
                trace!(
                    "No live connection found to {:?}, creating a new one.",
                    self.peer
                );
                self.create_connection().await
            }
        }
    }

    /// Is this Link currently connected?
    pub(crate) async fn is_connected(&self) -> bool {
        // self.remove_expired().await;
        !self.connections.read().await.is_empty()
    }

    async fn create_connection_with_listener<F: Fn(Connection, IncomingMsgs)>(
        &self,
        listen: F,
    ) -> Result<Connection, SendToOneError> {
        let (conn, incoming_msgs) = self
            .endpoint
            .connect_to(&self.peer.addr())
            .await
            .map_err(SendToOneError::Connection)?;

        trace!(
            "{} to {} (id: {})",
            LogMarker::ConnectionOpened,
            conn.remote_address(),
            conn.id()
        );

        self.insert(conn.clone()).await;

        listen(conn.clone(), incoming_msgs);

        Ok(conn)
    }

    /// Uses qp2p to create a connection and stores it on Self
    async fn create_connection(&self) -> Result<Connection, SendToOneError> {
        let (conn, _incoming_msgs) = self
            .endpoint
            .connect_to(&self.peer.addr())
            .await
            .map_err(SendToOneError::Connection)?;

        trace!(
            "{} to {} (id: {})",
            LogMarker::ConnectionOpened,
            conn.remote_address(),
            conn.id()
        );

        self.insert(conn.clone()).await;

        Ok(conn)
    }

    async fn insert(&self, conn: Connection) {
        let id = conn.id();

        {
            let _ = self.connections.write().await.insert(id.clone(), conn);
        }
    }
}

/// Errors that can be returned from `Comm::send_to_one`.
#[derive(Debug)]
pub enum SendToOneError {
    ///
    Connection(qp2p::ConnectionError),
    ///
    Send(qp2p::SendError),
    #[cfg(feature = "chaos")]
    /// ChaosNoConn
    ChaosNoConnection,
    /// Sending failed repeatedly
    SendRepeatedlyFailed,
}

impl SendToOneError {
    ///
    #[allow(unused)]
    pub(crate) fn is_local_close(&self) -> bool {
        matches!(
            self,
            Self::Connection(qp2p::ConnectionError::Closed(qp2p::Close::Local))
                | Self::Send(qp2p::SendError::ConnectionLost(
                    qp2p::ConnectionError::Closed(qp2p::Close::Local)
                ))
        )
    }
}
