// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Cmd;

use crate::messaging::{system::SystemMsg, MsgKind, WireMsg};
use crate::node::{
    core::{Core, Proposal, SendStatus},
    messages::WireMsgUtils,
    Error, Result,
};
use crate::types::{log_markers::LogMarker, Peer};

use std::{sync::Arc, time::Duration};
use tokio::time::MissedTickBehavior;
use tokio::{sync::watch, time};
use tracing::Instrument;

const PROBE_INTERVAL: Duration = Duration::from_secs(30);

// A command/subcommand id e.g. "963111461", "963111461.0"
type CmdId = String;

// Cmd Dispatcher.
pub(crate) struct Dispatcher {
    pub(crate) core: Core,

    cancel_timer_tx: watch::Sender<bool>,
    cancel_timer_rx: watch::Receiver<bool>,
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Cancel all scheduled timers including any future ones.
        let _res = self.cancel_timer_tx.send(true);
    }
}

impl Dispatcher {
    pub(super) fn new(core: Core) -> Self {
        let (cancel_timer_tx, cancel_timer_rx) = watch::channel(false);
        Self {
            core,
            cancel_timer_tx,
            cancel_timer_rx,
        }
    }

    /// Enqueues the given cmd and handles whatever cmd is in the next priority queue and triggers handling after any required waits for higher priority tasks
    pub(super) async fn enqueue_and_handle_next_cmd_and_offshoots(
        self: Arc<Self>,
        cmd: Cmd,
        cmd_id: Option<CmdId>,
    ) -> Result<()> {
        let _ = tokio::spawn(async {
            let cmd_id: CmdId = cmd_id.unwrap_or_else(|| rand::random::<u32>().to_string());

            self.handle_cmd_and_offshoots(cmd, Some(cmd_id)).await
        });
        Ok(())
    }

    /// Handles cmd and transitively queues any new cmds that are
    /// produced during its handling. Trace logs will include the provided cmd id,
    /// and any sub-cmds produced will have it as a common root cmd id.
    /// If a cmd id string is not provided a random one will be generated.
    pub(super) async fn handle_cmd_and_offshoots(
        self: Arc<Self>,
        cmd: Cmd,
        cmd_id: Option<CmdId>,
    ) -> Result<()> {
        let cmd_id = cmd_id.unwrap_or_else(|| rand::random::<u32>().to_string());
        let cmd_id_clone = cmd_id.clone();
        let cmd_display = cmd.to_string();
        let _task = tokio::spawn(async move {
            match self.process_cmd(cmd, &cmd_id).await {
                Ok(cmds) => {
                    for (sub_cmd_count, cmd) in cmds.into_iter().enumerate() {
                        let sub_cmd_id = format!("{}.{}", &cmd_id, sub_cmd_count);
                        // Error here is only related to queueing, and so a dropped cmd will be logged
                        let _result = self.clone().spawn_cmd_handling(cmd, sub_cmd_id);
                    }
                }
                Err(err) => {
                    error!("Failed to handle cmd {:?} with error {:?}", cmd_id, err);
                }
            }
        });

        trace!(
            "{:?} {} cmd_id={}",
            LogMarker::CmdHandlingSpawned,
            cmd_display,
            &cmd_id_clone
        );
        Ok(())
    }

    // Note: this indirecton is needed. Trying to call `spawn(self.handle_cmds(...))` directly
    // inside `handle_cmds` causes compile error about type check cycle.
    fn spawn_cmd_handling(self: Arc<Self>, cmd: Cmd, cmd_id: String) -> Result<()> {
        let _task = tokio::spawn(self.enqueue_and_handle_next_cmd_and_offshoots(cmd, Some(cmd_id)));
        Ok(())
    }

    pub(super) async fn start_network_probing(self: Arc<Self>) {
        info!("Starting to probe network");
        let _handle = tokio::spawn(async move {
            let dispatcher = self.clone();
            let mut interval = tokio::time::interval(PROBE_INTERVAL);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                let _instant = interval.tick().await;

                // Send a probe message if we are an elder
                let core = &dispatcher.core;
                if core.is_elder().await && !core.network_knowledge().prefix().await.is_empty() {
                    match core.generate_probe_msg().await {
                        Ok(cmd) => {
                            info!("Sending probe msg");
                            if let Err(e) = dispatcher
                                .clone()
                                .enqueue_and_handle_next_cmd_and_offshoots(cmd, None)
                                .await
                            {
                                error!("Error sending a probe msg to the network: {:?}", e);
                            }
                        }
                        Err(error) => error!("Problem generating probe msg: {:?}", error),
                    }
                }
            }
        });
    }

    pub(super) async fn write_prefixmap_to_disk(self: Arc<Self>) {
        info!("Writing our PrefixMap to disk");
        self.clone().core.write_prefix_map().await
    }

    /// Handles a single cmd.
    pub(super) async fn process_cmd(&self, cmd: Cmd, cmd_id: &str) -> Result<Vec<Cmd>> {
        // Create a tracing span containing info about the current node. This is very useful when
        // analyzing logs produced by running multiple nodes within the same process, for example
        // from integration tests.
        let span = {
            let core = &self.core;

            let prefix = core.network_knowledge().prefix().await;
            let is_elder = core.is_elder().await;
            let section_key = core.network_knowledge().section_key().await;
            let age = core.node.read().await.age();
            trace_span!(
                "process_cmd",
                name = %core.node.read().await.name(),
                prefix = format_args!("({:b})", prefix),
                age,
                elder = is_elder,
                cmd_id = %cmd_id,
                section_key = ?section_key,
                %cmd,
            )
        };

        async {
            let cmd_display = cmd.to_string();
            trace!(
                "{:?} {:?} - {}",
                LogMarker::CmdProcessStart,
                cmd_id,
                cmd_display
            );

            let res = match self.try_processing_cmd(cmd).await {
                Ok(outcome) => {
                    trace!(
                        "{:?} {:?} - {}",
                        LogMarker::CmdProcessEnd,
                        cmd_id,
                        cmd_display
                    );
                    Ok(outcome)
                }
                Err(error) => {
                    error!(
                        "Error encountered when processing cmd (cmd_id {}): {:?}",
                        cmd_id, error
                    );
                    trace!(
                        "{:?} {}: {:?}",
                        LogMarker::CmdProcessingError,
                        cmd_display,
                        error
                    );
                    Err(error)
                }
            };
            res
        }
        .instrument(span)
        .await
    }

    /// Actually process the cmd
    async fn try_processing_cmd(&self, cmd: Cmd) -> Result<Vec<Cmd>> {
        match cmd {
            Cmd::HandleSystemMsg {
                sender,
                msg_id,
                msg_authority,
                dst_location,
                msg,
                payload,
                known_keys,
            } => {
                self.core
                    .handle_system_msg(
                        sender,
                        msg_id,
                        msg_authority,
                        dst_location,
                        msg,
                        payload,
                        known_keys,
                    )
                    .await
            }
            Cmd::SignOutgoingSystemMsg { msg, dst } => {
                let src_section_pk = self.core.network_knowledge().section_key().await;
                let wire_msg =
                    WireMsg::single_src(&*self.core.node.read().await, dst, msg, src_section_pk)?;

                let mut cmds = vec![];
                cmds.extend(self.core.send_msg_to_nodes(wire_msg).await?);

                Ok(cmds)
            }
            Cmd::HandleMsg {
                sender,
                wire_msg,
                original_bytes,
            } => self.core.handle_msg(sender, wire_msg, original_bytes).await,
            Cmd::HandleTimeout(token) => self.core.handle_timeout(token).await,
            Cmd::HandleAgreement { proposal, sig } => {
                self.core.handle_general_agreements(proposal, sig).await
            }
            Cmd::HandleNewNodeOnline(auth) => {
                self.core
                    .handle_online_agreement(auth.value.into_state(), auth.sig)
                    .await
            }
            Cmd::HandleNewEldersAgreement { proposal, sig } => match proposal {
                Proposal::NewElders(section_auth) => {
                    self.core
                        .handle_new_elders_agreement(section_auth, sig)
                        .await
                }
                _ => {
                    error!("Other agreement messages should be handled in `HandleAgreement`, which is non-blocking ");
                    Ok(vec![])
                }
            },
            Cmd::HandlePeerLost(peer) => self.core.handle_peer_lost(&peer.addr()).await,
            Cmd::HandleDkgOutcome {
                section_auth,
                outcome,
            } => self.core.handle_dkg_outcome(section_auth, outcome).await,
            Cmd::HandleDkgFailure(signeds) => self
                .core
                .handle_dkg_failure(signeds)
                .await
                .map(|cmd| vec![cmd]),
            Cmd::SendMsg {
                recipients,
                wire_msg,
            } => self.send_msg(&recipients, recipients.len(), wire_msg).await,
            Cmd::SendMsgDeliveryGroup {
                recipients,
                delivery_group_size,
                wire_msg,
            } => {
                self.send_msg(&recipients, delivery_group_size, wire_msg)
                    .await
            }
            Cmd::ScheduleTimeout { duration, token } => Ok(self
                .handle_schedule_timeout(duration, token)
                .await
                .into_iter()
                .collect()),
            Cmd::SendAcceptedOnlineShare {
                peer,
                previous_name,
            } => {
                self.core
                    .send_accepted_online_share(peer, previous_name)
                    .await
            }
            Cmd::ProposeOffline(name) => self.core.propose_offline(name).await,
            Cmd::StartConnectivityTest(name) => Ok(vec![
                self.core
                    .send_msg_to_our_elders(SystemMsg::StartConnectivityTest(name))
                    .await?,
            ]),
            Cmd::TestConnectivity(name) => {
                let mut cmds = vec![];
                if let Some(member_info) = self
                    .core
                    .network_knowledge()
                    .get_section_member(&name)
                    .await
                {
                    if self
                        .core
                        .comm
                        .is_reachable(&member_info.addr())
                        .await
                        .is_err()
                    {
                        cmds.push(Cmd::ProposeOffline(member_info.name()));
                    }
                }
                Ok(cmds)
            }
        }
    }

    async fn send_msg(
        &self,
        recipients: &[Peer],
        delivery_group_size: usize,
        wire_msg: WireMsg,
    ) -> Result<Vec<Cmd>> {
        let cmds = match wire_msg.msg_kind() {
            MsgKind::NodeAuthMsg(_) | MsgKind::NodeBlsShareAuthMsg(_) => {
                self.deliver_msgs(recipients, delivery_group_size, wire_msg)
                    .await?
            }
            MsgKind::ServiceMsg(_) => {
                if let Err(err) = self
                    .core
                    .comm
                    .send_on_existing_connection_to_client(recipients, wire_msg.clone())
                    .await
                {
                    error!("Failed sending message {:?} to recipients {:?} on existing connection with error {:?}",
                            wire_msg, recipients, err);
                }

                vec![]
            }
        };

        Ok(cmds)
    }

    async fn deliver_msgs(
        &self,
        recipients: &[Peer],
        delivery_group_size: usize,
        wire_msg: WireMsg,
    ) -> Result<Vec<Cmd>> {
        let status = self
            .core
            .comm
            .send(recipients, delivery_group_size, wire_msg)
            .await?;

        match status {
            SendStatus::MinDeliveryGroupSizeReached(failed_recipients)
            | SendStatus::MinDeliveryGroupSizeFailed(failed_recipients) => Ok(failed_recipients
                .into_iter()
                .map(Cmd::HandlePeerLost)
                .collect()),
            _ => Ok(vec![]),
        }
        .map_err(|e: Error| e)
    }

    async fn handle_schedule_timeout(&self, duration: Duration, token: u64) -> Option<Cmd> {
        let mut cancel_rx = self.cancel_timer_rx.clone();

        if *cancel_rx.borrow() {
            // Timers are already cancelled, do nothing.
            return None;
        }

        tokio::select! {
            _ = time::sleep(duration) => Some(Cmd::HandleTimeout(token)),
            _ = cancel_rx.changed() => None,
        }
    }
}
