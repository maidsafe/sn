// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use std::time::Duration;

use super::Link;

use crate::node::{Error, Result};

use qp2p::RetryConfig;
use qp2p::UsrMsgBytes;
use sn_interface::messaging::MsgId;

use custom_debug::Debug;

use tokio::sync::mpsc;

// TODO: temporarily disable priority while we transition to channels
// type Priority = i32;

/// These retries are how may _new_ connection attempts do we make.
/// If we fail all of these, HandlePeerFailedSend will be triggered
/// for section nodes, which in turn kicks off Dysfunction tracking
const MAX_SENDJOB_RETRIES: usize = 3;

#[derive(Debug)]
enum SessionCmd {
    Send(SendJob),
    RemoveExpired,
    AddConnection(qp2p::Connection),
    Terminate,
}

#[derive(Clone)]
pub(crate) struct PeerSession {
    channel: mpsc::Sender<SessionCmd>,
}

impl PeerSession {
    pub(crate) fn new(link: Link) -> PeerSession {
        let (sender, receiver) = mpsc::channel(1000);

        let _ = tokio::task::spawn(PeerSessionWorker::new(link, sender.clone()).run(receiver));

        PeerSession { channel: sender }
    }

    /// can we cleanup this session?
    pub(crate) async fn can_cleanup(&self) -> bool {
        if let Err(e) = self.channel.send(SessionCmd::RemoveExpired).await {
            warn!("Error while sending RemoveExpired cmd {e:?}");
            return true;
        }

        self.channel.is_closed()
    }

    // this must be restricted somehow, we can't allow an unbounded inflow
    // of connections from a peer...
    pub(crate) async fn add(&self, conn: qp2p::Connection) {
        let cmd = SessionCmd::AddConnection(conn.clone());
        if let Err(e) = self.channel.send(cmd).await {
            error!("Error while sending AddConnection {e:?}");

            // if we have disconnected from a peer, will we allow it to connect to us again anyway..??
            conn.close(Some(
                "We have disconnected from the peer and do not allow incoming connections."
                    .to_string(),
            ));
        }
    }

    #[instrument(skip(self, bytes))]
    pub(crate) async fn send_using_session(
        &self,
        msg_id: MsgId,
        bytes: UsrMsgBytes,
        is_msg_for_client: bool,
    ) -> Result<SendWatcher> {
        let (watcher, reporter) = status_watching();

        let job = SendJob {
            msg_id,
            bytes,
            retries: 0,
            reporter,
            is_msg_for_client,
        };

        self.channel
            .send(SessionCmd::Send(job))
            .await
            .map_err(|_| Error::PeerSessionChannel)?;

        trace!("Send job sent: {msg_id:?}");
        Ok(watcher)
    }

    pub(crate) async fn disconnect(self) {
        if let Err(e) = self.channel.send(SessionCmd::Terminate).await {
            error!("Error while sending Terminate command: {e:?}");
        }
    }
}

/// After processing each `SessionCmd`, we decide whether to keep going
#[must_use]
enum SessionStatus {
    Ok,
    Terminating,
}

struct PeerSessionWorker {
    queue: mpsc::Sender<SessionCmd>,
    pub(crate) link: Link,
}

impl PeerSessionWorker {
    fn new(link: Link, queue: mpsc::Sender<SessionCmd>) -> Self {
        Self { queue, link }
    }

    async fn run(#[allow(unused_mut)] mut self, mut channel: mpsc::Receiver<SessionCmd>) {
        while let Some(session_cmd) = channel.recv().await {
            trace!(
                "Processing session {:?} cmd: {:?}",
                self.link.peer(),
                session_cmd
            );

            let status = match session_cmd {
                SessionCmd::Send(job) => {
                    match self.send(job).await {
                        Ok(s) => s,
                        Err(error) => {
                            error!("session error {error:?}");
                            // don't breakout here?
                            // TODO: is this correct?
                            continue;
                        }
                    }
                }
                SessionCmd::RemoveExpired => {
                    if self.link.is_connected_after_cleanup().await {
                        SessionStatus::Ok
                    } else {
                        // close down the session
                        SessionStatus::Terminating
                    }
                }
                SessionCmd::AddConnection(conn) => {
                    self.link.add(conn).await;
                    SessionStatus::Ok
                }
                SessionCmd::Terminate => SessionStatus::Terminating,
            };

            match status {
                SessionStatus::Terminating => {
                    info!("Terminating connection to {:?}", self.link.peer());
                    break;
                }
                SessionStatus::Ok => (),
            }
        }

        // close the channel to prevent senders adding more messages.
        channel.close();

        // drain channel to avoid memory leaks.
        while let Some(msg) = channel.recv().await {
            info!("Draining channel: dropping {:?}", msg);
        }

        // disconnect the link.
        self.link.disconnect().await;

        info!("Finished peer session shutdown");
    }

    async fn send(&mut self, mut job: SendJob) -> Result<SessionStatus> {
        let id = job.msg_id;
        let should_establish_new_connection = !job.is_msg_for_client;
        trace!("Performing sendjob: {id:?} , should_establish_new_connection? {should_establish_new_connection}");

        if job.retries > MAX_SENDJOB_RETRIES {
            job.reporter.send(SendStatus::MaxRetriesReached);
            return Ok(SessionStatus::Ok);
        }

        // we can't spawn this atm as it edits/updates the link
        // if we can separate out those parts, we could hopefully get this all properly
        // spawnable.
        // But right now we have to wait to ensure the link has connection
        // before spawning the send

        // TODO: this link makes sense?
        let conn = self
            .link
            .get_or_connect(should_establish_new_connection)
            .await
            .map_err(|_| Error::PeerLinkDropped)?;
        let queue = self.queue.clone();
        let link_connections = self.link.connections.clone();
        let link_queue = self.link.queue.clone();
        let _handle = tokio::spawn(async move {
            let send_resp = Link::send_with(
                job.bytes.clone(),
                0,
                Some(&RetryConfig::default()),
                should_establish_new_connection,
                conn,
                link_connections,
                link_queue,
            )
            .await;

            match send_resp {
                Ok(_) => {
                    job.reporter.send(SendStatus::Sent);
                }
                Err(err) if err.is_local_close() => {
                    info!("Peer linked dropped when trying to send {:?}", id);
                    job.reporter.send(SendStatus::PeerLinkDropped);
                    // return SessionStatus::Terminating;
                }
                Err(err) => {
                    warn!(
                        "Transient error while attempting to send, re-enqueing job {id:?} {err:?}"
                    );

                    // sleep so connection removal.cleanup in the link can occur before we try again.
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    job.reporter
                        .send(SendStatus::TransientError(format!("{err:?}")));

                    job.retries += 1;

                    if let Err(e) = queue.send(SessionCmd::Send(job)).await {
                        warn!("Failed to re-enqueue job {id:?} after transient error {e:?}");
                        // return SessionStatus::Terminating;
                    }
                }
            }
        });

        Ok(SessionStatus::Ok)
    }
}

#[derive(Debug)]
pub(crate) struct SendJob {
    msg_id: MsgId,
    #[debug(skip)]
    bytes: UsrMsgBytes,
    retries: usize, // TAI: Do we need this if we are using QP2P's retry
    reporter: StatusReporting,
    pub(crate) is_msg_for_client: bool,
}

impl PartialEq for SendJob {
    fn eq(&self, other: &Self) -> bool {
        self.msg_id == other.msg_id && self.bytes == other.bytes && self.retries == other.retries
    }
}

impl Eq for SendJob {}

impl std::hash::Hash for SendJob {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.msg_id.hash(state);
        self.bytes.hash(state);
        self.retries.hash(state);
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SendStatus {
    Enqueued,
    Sent,
    PeerLinkDropped,
    TransientError(String),
    WatcherDropped,
    MaxRetriesReached,
}

pub(crate) struct SendWatcher {
    receiver: tokio::sync::watch::Receiver<SendStatus>,
}

impl SendWatcher {
    /// Reads current status
    #[allow(unused)]
    pub(crate) fn status(&self) -> SendStatus {
        self.receiver.borrow().clone()
    }

    /// Waits until a new status arrives.
    pub(crate) async fn await_change(&mut self) -> SendStatus {
        if self.receiver.changed().await.is_ok() {
            self.receiver.borrow_and_update().clone()
        } else {
            SendStatus::WatcherDropped
        }
    }
}

#[derive(Debug)]
struct StatusReporting {
    sender: tokio::sync::watch::Sender<SendStatus>,
}

impl StatusReporting {
    fn send(&self, status: SendStatus) {
        // todo: ok to drop error here?
        let _ = self.sender.send(status);
    }
}

fn status_watching() -> (SendWatcher, StatusReporting) {
    let (sender, receiver) = tokio::sync::watch::channel(SendStatus::Enqueued);
    (SendWatcher { receiver }, StatusReporting { sender })
}
