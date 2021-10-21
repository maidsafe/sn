// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{msg_count::MsgCount, BackPressure};
use crate::messaging::{system::LoadReport, WireMsg};
use crate::routing::error::{Error, Result};
use bytes::Bytes;
use futures::{
    future::{FutureExt, TryFutureExt},
    stream::{FuturesUnordered, StreamExt},
};
use qp2p::Endpoint;
use std::net::SocketAddr;
use tokio::{sync::mpsc, task};
use xor_name::XorName;

// Communication component of the node to interact with other nodes.
#[derive(Clone)]
pub(crate) struct Comm {
    endpoint: Endpoint<XorName>,
    event_tx: mpsc::Sender<ConnectionEvent>,
    msg_count: MsgCount,
    back_pressure: BackPressure,
}

impl Drop for Comm {
    fn drop(&mut self) {
        // Close all existing connections and stop accepting new ones.
        self.endpoint.close();
    }
}

impl Comm {
    pub(crate) async fn new(
        local_addr: SocketAddr,
        config: qp2p::Config,
        event_tx: mpsc::Sender<ConnectionEvent>,
    ) -> Result<Self> {
        // Don't bootstrap, just create an endpoint to listen to
        // the incoming messages from other nodes.
        // This also returns the a channel where we can listen for
        // disconnection events.
        let (endpoint, incoming_connections, _) =
            Endpoint::new(local_addr, Default::default(), config).await?;

        let msg_count = MsgCount::new();

        let _handle = task::spawn(handle_incoming_connections(
            incoming_connections,
            event_tx.clone(),
            msg_count.clone(),
        ));

        let back_pressure = BackPressure::new();

        Ok(Self {
            endpoint,
            event_tx,
            msg_count,
            back_pressure,
        })
    }

    pub(crate) async fn bootstrap(
        local_addr: SocketAddr,
        bootstrap_nodes: &[SocketAddr],
        config: qp2p::Config,
        event_tx: mpsc::Sender<ConnectionEvent>,
    ) -> Result<(Self, SocketAddr)> {
        // Bootstrap to the network returning the connection to a node.
        // We can use the returned channels to listen for incoming messages and disconnection events
        let (endpoint, incoming_connections, bootstrap_peer) =
            Endpoint::new(local_addr, bootstrap_nodes, config).await?;
        let (bootstrap_peer, peer_incoming) = bootstrap_peer.ok_or(Error::BootstrapFailed)?;

        let msg_count = MsgCount::new();

        let _handle = task::spawn(handle_incoming_connections(
            incoming_connections,
            event_tx.clone(),
            msg_count.clone(),
        ));

        if let Some(incoming_messages) = peer_incoming {
            let _ = task::spawn(handle_incoming_messages(
                bootstrap_peer.remote_address(),
                incoming_messages,
                event_tx.clone(),
                msg_count.clone(),
            ));
        }

        Ok((
            Self {
                endpoint,
                event_tx,
                msg_count,
                back_pressure: BackPressure::new(),
            },
            bootstrap_peer.remote_address(),
        ))
    }

    pub(crate) fn our_connection_info(&self) -> SocketAddr {
        self.endpoint.public_addr()
    }

    /// Get the connection ID (XorName) of an existing connection with the provided socket address
    pub(crate) async fn get_connection_id(&self, addr: &SocketAddr) -> Option<XorName> {
        self.endpoint
            .get_connection_by_addr(addr)
            .await
            .map(|connection| connection.id())
    }

    /// Get the SocketAddr of a connection using the connection ID (XorName)
    pub(crate) async fn get_socket_addr_by_id(&self, xorname: &XorName) -> Option<SocketAddr> {
        self.endpoint
            .get_connection_by_id(xorname)
            .await
            .map(|connection| connection.remote_address())
    }

    /// Sends a message on an existing connection. If no such connection exists, returns an error.
    pub(crate) async fn send_on_existing_connection(
        &self,
        recipients: &[(XorName, SocketAddr)],
        mut wire_msg: WireMsg,
    ) -> Result<(), Error> {
        trace!("Sending msg on existing connection to {:?}", recipients);
        for (name, addr) in recipients {
            wire_msg.set_dst_xorname(*name);

            let bytes = wire_msg.serialize()?;
            let priority = wire_msg.msg_kind().priority();
            let retries = self.back_pressure.get(addr).await; // TODO: more laid back retries with lower priority, more aggressive with higher

            self.endpoint
                .get_connection_by_addr(addr)
                .map(|res| res.ok_or(None))
                .and_then(|connection| async move {
                    connection
                        .send_with(bytes, priority, Some(&retries))
                        .await
                        .map_err(Some)
                })
                .await
                .map_err(|err| {
                    error!(
                        "Sending message (msg_id: {:?}) to {:?} (name {:?}) failed with {:?}",
                        wire_msg.msg_id(),
                        addr,
                        name,
                        err
                    );
                    Error::FailedSend(*addr, *name)
                })?;

            // count outgoing msgs..
            self.msg_count.increase_outgoing(*addr);
        }

        Ok(())
    }

    /// Tests whether the peer is reachable.
    pub(crate) async fn is_reachable(&self, peer: &SocketAddr) -> Result<(), Error> {
        let qp2p_config = qp2p::Config {
            forward_port: false,
            ..Default::default()
        };

        let connectivity_endpoint =
            Endpoint::<XorName>::new_client((self.endpoint.local_addr().ip(), 0), qp2p_config)?;

        let result = connectivity_endpoint
            .is_reachable(peer)
            .await
            .map_err(|err| {
                info!("Peer {} is NOT externally reachable: {:?}", peer, err);
                err.into()
            })
            .map(|()| {
                info!("Peer {} is externally reachable.", peer);
            });
        connectivity_endpoint.close();
        result
    }

    /// Sends a message to multiple recipients. Attempts to send to `delivery_group_size`
    /// recipients out of the `recipients` list. If a send fails, attempts to send to the next peer
    /// until `delivery_group_size`  successful sends complete or there are no more recipients to
    /// try.
    ///
    /// Returns an `Error::ConnectionClosed` if the connection is closed locally. Else it returns a
    /// `SendStatus::MinDeliveryGroupSizeReached` or `SendStatus::MinDeliveryGroupSizeFailed` depending
    /// on if the minimum delivery group size is met or not. The failed recipients are sent along
    /// with the status. It returns a `SendStatus::AllRecipients` if message is sent to all the recipients.
    pub(crate) async fn send(
        &self,
        recipients: &[(XorName, SocketAddr)],
        delivery_group_size: usize,
        wire_msg: WireMsg,
    ) -> Result<SendStatus> {
        let msg_id = wire_msg.msg_id();
        trace!(
            "Sending message (msg_id: {:?}) to {} of {:?}",
            msg_id,
            delivery_group_size,
            recipients
        );

        if recipients.len() < delivery_group_size {
            warn!(
                "Less than delivery_group_size valid recipients - delivery_group_size: {}, recipients: {:?}",
                delivery_group_size,
                recipients,
            );
        }

        let delivery_group_size = delivery_group_size.min(recipients.len());

        if recipients.is_empty() {
            return Err(Error::EmptyRecipientList);
        }

        let msg_bytes = wire_msg.serialize().map_err(Error::Messaging)?;
        let priority = wire_msg.msg_kind().priority();

        // Run all the sends concurrently (using `FuturesUnordered`). If any of them fails, pick
        // the next recipient and try to send to them. Proceed until the needed number of sends
        // succeeds or if there are no more recipients to pick.
        let send = |recipient: (XorName, SocketAddr), msg_bytes: Bytes| async move {
            trace!(
                "Sending message ({} bytes, msg_id: {:?}) to {} of delivery group size {}",
                msg_bytes.len(),
                msg_id,
                recipient.1,
                delivery_group_size,
            );

            let retries = self.back_pressure.get(&recipient.1).await; // TODO: more laid back retries with lower priority, more aggressive with higher

            let result = self
                .endpoint
                .connect_to(&recipient.1)
                .err_into()
                .and_then(|(connection, connection_incoming)| async move {
                    if let Some(connection_incoming) = connection_incoming {
                        let _ = task::spawn(handle_incoming_messages(
                            connection.remote_address(),
                            connection_incoming,
                            self.event_tx.clone(),
                            self.msg_count.clone(),
                        ));
                    }
                    connection
                        .send_with(msg_bytes, priority, Some(&retries))
                        .await
                })
                .await
                .map_err(|err| match err {
                    qp2p::SendError::ConnectionLost(qp2p::ConnectionError::Closed(
                        qp2p::Close::Local,
                    )) => Error::ConnectionClosed,
                    _ => {
                        trace!("during sending, received error {:?}", err);
                        err.into()
                    }
                });

            (result, recipient.1)
        };

        let mut tasks: FuturesUnordered<_> = recipients[0..delivery_group_size]
            .iter()
            .map(|(name, recipient)| send((*name, *recipient), msg_bytes.clone()))
            .collect();

        let mut next = delivery_group_size;
        let mut successes = 0;
        let mut failed_recipients = vec![];

        while let Some((result, addr)) = tasks.next().await {
            match result {
                Ok(()) => {
                    successes += 1;
                    // count outgoing msgs..
                    self.msg_count.increase_outgoing(addr);
                }
                Err(Error::ConnectionClosed) => {
                    // The connection was closed by us which means
                    // we are terminating so let's cut this short.
                    return Err(Error::ConnectionClosed);
                }
                Err(_) => {
                    failed_recipients.push(addr);

                    if next < recipients.len() {
                        tasks.push(send(recipients[next], msg_bytes.clone()));
                        next += 1;
                    }
                }
            }
        }

        trace!(
            "Finished sending message {:?} to {}/{} recipients (failed: {:?})",
            wire_msg,
            successes,
            delivery_group_size,
            failed_recipients
        );

        if successes == delivery_group_size {
            if failed_recipients.is_empty() {
                Ok(SendStatus::AllRecipients)
            } else {
                Ok(SendStatus::MinDeliveryGroupSizeReached(failed_recipients))
            }
        } else {
            Ok(SendStatus::MinDeliveryGroupSizeFailed(failed_recipients))
        }
    }

    /// Regulates comms with the specified peer
    /// according to the cpu load report provided by it.
    pub(crate) async fn regulate(&self, peer: SocketAddr, load_report: LoadReport) {
        self.back_pressure.set(peer, load_report).await
    }

    /// Returns cpu load report if being strained.
    pub(crate) async fn check_strain(&self, caller: SocketAddr) -> Option<LoadReport> {
        self.back_pressure.load_report(caller).await
    }

    pub(crate) fn print_stats(&self) {
        let incoming = self.msg_count.incoming();
        let outgoing = self.msg_count.outgoing();
        info!("*** Incoming msgs: {:?} ***", incoming);
        info!("*** Outgoing msgs: {:?} ***", outgoing);
    }
}

#[derive(Debug)]
pub(crate) enum ConnectionEvent {
    Received((SocketAddr, Bytes)),
}

async fn handle_incoming_connections(
    mut incoming_connections: qp2p::IncomingConnections<XorName>,
    event_tx: mpsc::Sender<ConnectionEvent>,
    msg_count: MsgCount,
) {
    while let Some((connection, connection_incoming)) = incoming_connections.next().await {
        let _ = task::spawn(handle_incoming_messages(
            connection.remote_address(),
            connection_incoming,
            event_tx.clone(),
            msg_count.clone(),
        ));
    }
}

async fn handle_incoming_messages(
    src: SocketAddr,
    mut incoming_msgs: qp2p::ConnectionIncoming<XorName>,
    event_tx: mpsc::Sender<ConnectionEvent>,
    msg_count: MsgCount,
) {
    while let Some(result) = incoming_msgs.next().await.transpose() {
        match result {
            Ok(msg) => {
                let _send_res = event_tx.send(ConnectionEvent::Received((src, msg))).await;
                // count incoming msgs..
                msg_count.increase_incoming(src);
            }
            Err(error) => {
                // TODO: should we propagate this?
                warn!("error on connection with {}: {:?}", src, error);
            }
        }
    }
}

/// Returns the status of the send operation.
#[derive(Debug, Clone)]
pub(crate) enum SendStatus {
    AllRecipients,
    MinDeliveryGroupSizeReached(Vec<SocketAddr>),
    MinDeliveryGroupSizeFailed(Vec<SocketAddr>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{DstLocation, MessageId, MsgKind, ServiceAuth};
    use crate::types::Keypair;
    use assert_matches::assert_matches;
    use eyre::Result;
    use futures::future;
    use qp2p::Config;
    use rand::rngs::OsRng;
    use std::{net::Ipv4Addr, time::Duration};
    use tokio::{net::UdpSocket, sync::mpsc, time};

    const TIMEOUT: Duration = Duration::from_secs(1);

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_send() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let comm = Comm::new(local_addr(), Config::default(), tx).await?;

        let mut peer0 = Peer::new().await?;
        let mut peer1 = Peer::new().await?;

        let original_message = new_test_message()?;

        let status = comm
            .send(
                &[(peer0.name, peer0.addr), (peer1.name, peer1.addr)],
                2,
                original_message.clone(),
            )
            .await?;

        assert_matches!(status, SendStatus::AllRecipients);

        if let Some(bytes) = peer0.rx.recv().await {
            assert_eq!(WireMsg::from(bytes)?, original_message.clone());
        }

        if let Some(bytes) = peer1.rx.recv().await {
            assert_eq!(WireMsg::from(bytes)?, original_message);
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_send_to_subset() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let comm = Comm::new(local_addr(), Config::default(), tx).await?;

        let mut peer0 = Peer::new().await?;
        let mut peer1 = Peer::new().await?;

        let original_message = new_test_message()?;
        let status = comm
            .send(
                &[(peer0.name, peer0.addr), (peer1.name, peer1.addr)],
                1,
                original_message.clone(),
            )
            .await?;

        assert_matches!(status, SendStatus::AllRecipients);

        if let Some(bytes) = peer0.rx.recv().await {
            assert_eq!(WireMsg::from(bytes)?, original_message);
        }

        assert!(time::timeout(TIMEOUT, peer1.rx.recv())
            .await
            .unwrap_or_default()
            .is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_send() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let comm = Comm::new(
            local_addr(),
            Config {
                // This makes this test faster.
                idle_timeout: Some(Duration::from_millis(1)),
                ..Config::default()
            },
            tx,
        )
        .await?;
        let invalid_addr = get_invalid_addr().await?;

        let status = comm
            .send(&[(XorName::random(), invalid_addr)], 1, new_test_message()?)
            .await?;

        assert_matches!(
            &status,
            &SendStatus::MinDeliveryGroupSizeFailed(_) => vec![invalid_addr]
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_send_after_failed_attempts() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let comm = Comm::new(
            local_addr(),
            Config {
                idle_timeout: Some(Duration::from_millis(1)),
                ..Config::default()
            },
            tx,
        )
        .await?;
        let mut peer = Peer::new().await?;
        let invalid_addr = get_invalid_addr().await?;
        let name = XorName::random();

        let message = new_test_message()?;
        let status = comm
            .send(
                &[(name, invalid_addr), (peer.name, peer.addr)],
                1,
                message.clone(),
            )
            .await?;
        assert_matches!(status, SendStatus::MinDeliveryGroupSizeReached(failed_recipients) => {
            assert_eq!(&failed_recipients, &[invalid_addr])
        });

        if let Some(bytes) = peer.rx.recv().await {
            assert_eq!(WireMsg::from(bytes)?, message);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn partially_successful_send() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let comm = Comm::new(
            local_addr(),
            Config {
                idle_timeout: Some(Duration::from_millis(1)),
                ..Config::default()
            },
            tx,
        )
        .await?;
        let mut peer = Peer::new().await?;
        let invalid_addr = get_invalid_addr().await?;
        let name = XorName::random();

        let message = new_test_message()?;
        let status = comm
            .send(
                &[(name, invalid_addr), (peer.name, peer.addr)],
                2,
                message.clone(),
            )
            .await?;

        assert_matches!(
            status,
            SendStatus::MinDeliveryGroupSizeFailed(_) => vec![invalid_addr]
        );

        if let Some(bytes) = peer.rx.recv().await {
            assert_eq!(WireMsg::from(bytes)?, message);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_after_reconnect() -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let send_comm = Comm::new(local_addr(), Config::default(), tx).await?;

        let (recv_endpoint, mut incoming_connections, _) =
            Endpoint::<XorName>::new(local_addr(), &[], Config::default()).await?;
        let recv_addr = recv_endpoint.public_addr();
        let name = XorName::random();

        let msg0 = new_test_message()?;
        let status = send_comm
            .send(&[(name, recv_addr)], 1, msg0.clone())
            .await?;
        assert_matches!(status, SendStatus::AllRecipients);

        let mut msg0_received = false;

        // Receive one message and disconnect from the peer
        {
            if let Some((connection, mut incoming_msgs)) = incoming_connections.next().await {
                if let Some(msg) = time::timeout(TIMEOUT, incoming_msgs.next()).await?? {
                    assert_eq!(WireMsg::from(msg)?, msg0);
                    msg0_received = true;
                    recv_endpoint
                        .disconnect_from(&connection.remote_address())
                        .await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            assert!(msg0_received);
        }

        let msg1 = new_test_message()?;
        let status = send_comm
            .send(&[(name, recv_addr)], 1, msg1.clone())
            .await?;
        assert_matches!(status, SendStatus::AllRecipients);

        let mut msg1_received = false;

        if let Some((_, mut incoming_msgs)) = incoming_connections.next().await {
            if let Some(msg) = time::timeout(TIMEOUT, incoming_msgs.next()).await?? {
                assert_eq!(WireMsg::from(msg)?, msg1);
                msg1_received = true;
            }
        }

        assert!(msg1_received);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn incoming_connection_lost() -> Result<()> {
        let (tx, mut rx0) = mpsc::channel(1);
        let comm0 = Comm::new(local_addr(), Config::default(), tx).await?;
        let addr0 = comm0.our_connection_info();

        let (tx, _rx) = mpsc::channel(1);
        let comm1 = Comm::new(local_addr(), Config::default(), tx).await?;

        // Send a message to establish the connection
        let status = comm1
            .send(&[(XorName::random(), addr0)], 1, new_test_message()?)
            .await?;
        assert_matches!(status, SendStatus::AllRecipients);

        assert_matches!(rx0.recv().await, Some(ConnectionEvent::Received(_)));
        // Drop `comm1` to cause connection lost.
        drop(comm1);

        assert_matches!(time::timeout(TIMEOUT, rx0.recv()).await, Err(_));

        Ok(())
    }

    fn new_test_message() -> Result<WireMsg> {
        let dst_location = DstLocation::Node {
            name: XorName::random(),
            section_pk: bls::SecretKey::random().public_key(),
        };

        let mut rng = OsRng;
        let src_keypair = Keypair::new_ed25519(&mut rng);

        let payload = WireMsg::serialize_msg_payload(&"test_string".to_string())?;
        let auth = ServiceAuth {
            public_key: src_keypair.public_key(),
            signature: src_keypair.sign(&payload),
        };

        let wire_msg = WireMsg::new_msg(
            MessageId::new(),
            payload,
            MsgKind::ServiceMsg(auth),
            dst_location,
        )?;

        Ok(wire_msg)
    }

    struct Peer {
        addr: SocketAddr,
        name: XorName,
        rx: mpsc::Receiver<Bytes>,
    }

    impl Peer {
        async fn new() -> Result<Self> {
            let (endpoint, mut incoming_connections, _) =
                Endpoint::<XorName>::new(local_addr(), &[], Config::default()).await?;
            let addr = endpoint.public_addr();

            let (tx, rx) = mpsc::channel(1);

            let _handle = tokio::spawn(async move {
                while let Some((_, mut incoming_messages)) = incoming_connections.next().await {
                    while let Ok(Some(msg)) = incoming_messages.next().await {
                        let _ = tx.send(msg).await;
                    }
                }
            });

            Ok(Self {
                addr,
                rx,
                name: XorName::random(),
            })
        }
    }

    async fn get_invalid_addr() -> Result<SocketAddr> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = socket.local_addr()?;

        // Keep the socket alive to keep the address bound, but don't read/write to it so any
        // attempt to connect to it will fail.
        let _handle = tokio::spawn(async move {
            future::pending::<()>().await;
            let _ = socket;
        });

        Ok(addr)
    }

    fn local_addr() -> SocketAddr {
        (Ipv4Addr::LOCALHOST, 0).into()
    }
}
