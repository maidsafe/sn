// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{Comm, Command, Core};
use crate::messaging::{
    node::{
        JoinAsRelocatedResponse, JoinRejectionReason, JoinResponse, RoutingMsg, Section,
        SrcAuthority, Variant,
    },
    DstLocation, MessageType,
};
use crate::routing::{
    error::Result, event::Event, messages::RoutingMsgUtils, node::Node, peer::PeerUtils,
    routing_api::comm::SendStatus, section::SectionPeersUtils, section::SectionUtils, Error,
    XorName,
};
use itertools::Itertools;
use sn_data_types::PublicKey;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    sync::{watch, RwLock},
    time,
};
use tracing::Instrument;

// `Command` Dispatcher.
pub(crate) struct Dispatcher {
    pub(super) core: RwLock<Core>,
    pub(super) comm: Comm,

    cancel_timer_tx: watch::Sender<bool>,
    cancel_timer_rx: watch::Receiver<bool>,
}

impl Dispatcher {
    pub fn new(state: Core, comm: Comm) -> Self {
        let (cancel_timer_tx, cancel_timer_rx) = watch::channel(false);
        Self {
            core: RwLock::new(state),
            comm,
            cancel_timer_tx,
            cancel_timer_rx,
        }
    }

    /// Send provided Event to the user which shall receive it through the EventStream
    pub async fn send_event(&self, event: Event) {
        self.core.read().await.send_event(event).await
    }

    /// Handles the given command and transitively any new commands that are produced during its
    /// handling.
    pub async fn handle_commands(self: Arc<Self>, command: Command) -> Result<()> {
        let commands = self.handle_command(command).await?;
        for command in commands {
            self.clone().spawn_handle_commands(command)
        }

        Ok(())
    }

    /// Handles a single command.
    pub async fn handle_command(&self, command: Command) -> Result<Vec<Command>> {
        // Create a tracing span containing info about the current node. This is very useful when
        // analyzing logs produced by running multiple nodes within the same process, for example
        // from integration tests.
        let span = {
            let state = self.core.read().await;
            trace_span!(
                "handle_command",
                name = %state.node().name(),
                prefix = format_args!("({:b})", state.section().prefix()),
                age = state.node().age(),
                elder = state.is_elder(),
            )
        };

        async {
            trace!(?command);

            self.try_handle_command(command).await.map_err(|error| {
                error!("Error encountered when handling command: {}", error);
                error
            })
        }
        .instrument(span)
        .await
    }

    // Terminate this routing instance - cancel all scheduled timers including any future ones,
    // close all network connections and stop accepting new connections.
    pub fn terminate(&self) {
        let _ = self.cancel_timer_tx.send(true);
        self.comm.terminate()
    }

    async fn try_handle_command(&self, command: Command) -> Result<Vec<Command>> {
        match command {
            Command::HandleMessage {
                sender,
                message,
                dst_info,
            } => {
                if let Some(sender) = &sender {
                    // let's then see if we need to do a reachability test
                    let failure = match &message.variant {
                        Variant::JoinRequest(join_request) => {
                            // Do this check only for the initial join request
                            if join_request.resource_proof_response.is_none()
                                && self.comm.is_reachable(sender).await.is_err()
                            {
                                Some(Variant::JoinResponse(Box::new(JoinResponse::Rejected(
                                    JoinRejectionReason::NodeNotReachable(*sender),
                                ))))
                            } else {
                                None
                            }
                        }
                        Variant::JoinAsRelocatedRequest(join_request) => {
                            // Do this check only for the initial join request
                            if join_request.relocate_payload.is_none()
                                && self.comm.is_reachable(sender).await.is_err()
                            {
                                Some(Variant::JoinAsRelocatedResponse(Box::new(
                                    JoinAsRelocatedResponse::NodeNotReachable(*sender),
                                )))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some(variant) = failure {
                        let section_key = *self.core.read().await.section().chain.last_key();
                        if let SrcAuthority::Node { public_key, .. } = message.src {
                            trace!("Sending {:?} to {}", variant, sender);
                            return Ok(vec![self.core.read().await.send_direct_message(
                                (XorName::from(PublicKey::from(public_key)), *sender),
                                variant,
                                section_key,
                            )?]);
                        }
                    }
                }

                self.core
                    .write()
                    .await
                    .handle_message(sender, message, dst_info)
                    .await
            }
            Command::HandleSectionInfoMsg {
                sender,
                message,
                dst_info,
            } => Ok(self
                .core
                .write()
                .await
                .handle_section_info_msg(sender, message, dst_info)
                .await),
            Command::HandleTimeout(token) => self.core.write().await.handle_timeout(token),
            Command::HandleAgreement { proposal, sig } => {
                self.core
                    .write()
                    .await
                    .handle_agreement(proposal, sig)
                    .await
            }
            Command::HandleConnectionLost(addr) => {
                self.core.read().await.handle_connection_lost(addr)
            }
            Command::HandlePeerLost(addr) => self.core.read().await.handle_peer_lost(&addr),
            Command::HandleDkgOutcome {
                section_auth,
                outcome,
            } => self
                .core
                .write()
                .await
                .handle_dkg_outcome(section_auth, outcome),
            Command::HandleDkgFailure(signeds) => self
                .core
                .write()
                .await
                .handle_dkg_failure(signeds)
                .map(|command| vec![command]),
            Command::SendMessage {
                recipients,
                delivery_group_size,
                message,
            } => {
                self.send_message(&recipients, delivery_group_size, message)
                    .await
            }
            Command::SendUserMessage {
                itinerary,
                content,
                additional_proof_chain_key: _,
            } => {
                self.core
                    .write()
                    .await
                    .send_user_message(itinerary, content)
                    .await
            }
            Command::ScheduleTimeout { duration, token } => Ok(self
                .handle_schedule_timeout(duration, token)
                .await
                .into_iter()
                .collect()),
            Command::HandlelocationComplete { node, section } => {
                self.handle_relocation_complete(node, section).await
            }
            Command::SetJoinsAllowed(joins_allowed) => {
                self.core.read().await.set_joins_allowed(joins_allowed)
            }
            Command::ProposeOnline {
                mut peer,
                previous_name,
                dst_key,
            } => {
                // The reachability check was completed during the initial bootstrap phase
                peer.set_reachable(true);
                self.core
                    .read()
                    .await
                    .make_online_proposal(peer, previous_name, dst_key)
                    .await
            }
            Command::ProposeOffline(name) => self.core.read().await.propose_offline(name),
            Command::StartConnectivityTest(name) => {
                let msg = {
                    let core = self.core.read().await;
                    let node = core.node();
                    let section_key = *core.section().chain.last_key();
                    RoutingMsg::single_src(
                        node,
                        DstLocation::Section(core.node().name()),
                        Variant::StartConnectivityTest(name),
                        section_key,
                    )?
                };
                let our_name = self.core.read().await.node().name();
                let peers = self
                    .core
                    .read()
                    .await
                    .section()
                    .active_members()
                    .filter(|peer| peer.name() != &name && peer.name() != &our_name)
                    .cloned()
                    .collect_vec();
                Ok(self.core.read().await.send_or_handle(msg, &peers))
            }
            Command::TestConnectivity(name) => {
                let mut commands = vec![];
                if let Some(peer) = self
                    .core
                    .read()
                    .await
                    .section()
                    .members()
                    .get(&name)
                    .map(|member_info| member_info.peer)
                {
                    if self.comm.is_reachable(peer.addr()).await.is_err() {
                        commands.push(Command::ProposeOffline(*peer.name()));
                    }
                }
                Ok(commands)
            }
        }
    }

    // Note: this indirecton is needed. Trying to call `spawn(self.handle_commands(...))` directly
    // inside `handle_commands` causes compile error about type check cycle.
    fn spawn_handle_commands(self: Arc<Self>, command: Command) {
        let _ = tokio::spawn(self.handle_commands(command));
    }

    async fn send_message(
        &self,
        recipients: &[(XorName, SocketAddr)],
        delivery_group_size: usize,
        message: MessageType,
    ) -> Result<Vec<Command>> {
        let cmds = match message {
            MessageType::Node { .. } | MessageType::Routing { .. } => {
                let status = self
                    .comm
                    .send(recipients, delivery_group_size, message)
                    .await?;
                match status {
                    SendStatus::MinDeliveryGroupSizeReached(failed_recipients)
                    | SendStatus::MinDeliveryGroupSizeFailed(failed_recipients) => {
                        Ok(failed_recipients
                            .into_iter()
                            .map(Command::HandlePeerLost)
                            .collect())
                    }
                    _ => Ok(vec![]),
                }
                .map_err(|e: Error| e)?
            }
            MessageType::Client { .. } => {
                for recipient in recipients {
                    if self
                        .comm
                        .send_on_existing_connection(*recipient, message.clone())
                        .await
                        .is_err()
                    {
                        trace!(
                            "Lost connection to client {:?} when sending message {:?}",
                            recipient,
                            message
                        );
                        self.send_event(Event::ClientLost(recipient.1)).await;
                    }
                }
                vec![]
            }
            MessageType::SectionInfo { .. } => {
                for recipient in recipients {
                    let _ = self
                        .comm
                        .send_on_existing_connection(*recipient, message.clone())
                        .await;
                }
                vec![]
            }
        };

        Ok(cmds)
    }

    async fn handle_schedule_timeout(&self, duration: Duration, token: u64) -> Option<Command> {
        let mut cancel_rx = self.cancel_timer_rx.clone();

        if *cancel_rx.borrow() {
            // Timers are already cancelled, do nothing.
            return None;
        }

        tokio::select! {
            _ = time::sleep(duration) => Some(Command::HandleTimeout(token)),
            _ = cancel_rx.changed() => None,
        }
    }

    async fn handle_relocation_complete(
        &self,
        new_node: Node,
        new_section: Section,
    ) -> Result<Vec<Command>> {
        let previous_name = self.core.read().await.node().name();
        let new_keypair = new_node.keypair.clone();

        let mut state = self.core.write().await;
        let event_tx = state.event_tx.clone();
        *state = Core::new(new_node, new_section, None, event_tx);

        state
            .send_event(Event::Relocated {
                previous_name,
                new_keypair,
            })
            .await;

        Ok(vec![])
    }
}
