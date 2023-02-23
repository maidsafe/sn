// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

pub(crate) mod cmd_ctrl;
pub(crate) mod cmds;
pub(super) mod dispatcher;
pub(super) mod fault_detection;
mod periodic_checks;

#[cfg(test)]
pub(crate) mod tests;
pub(crate) use cmd_ctrl::CmdCtrl;

use super::DataStorage;
use periodic_checks::PeriodicChecksTimestamps;

use crate::node::{
    flow_ctrl::{
        cmds::Cmd,
        fault_detection::{FaultChannels, FaultsCmd},
    },
    messaging::Peers,
    MyNode, STANDARD_CHANNEL_SIZE,
};

use sn_comms::{CommEvent, MsgFromPeer};
use sn_fault_detection::FaultDetection;
use sn_interface::{
    messaging::system::{JoinRejectReason, NodeDataCmd, NodeMsg},
    types::{log_markers::LogMarker, DataAddress, Peer},
};

use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    RwLock,
};
use xor_name::XorName;

/// Keep this as 1 so we properly feedback if we're not popping things out of the channel fast enough
const CMD_CHANNEL_SIZE: usize = 1;

/// Sent via the rejoin_network_tx to restart the join process.
/// This would only occur when joins are not allowed, or non-recoverable states.
#[derive(Debug)]
pub enum RejoinReason {
    /// Happens when trying to join; we will wait a moment and then try again.
    /// NB: Relocated nodes that try to join, are accepted even if joins are disallowed.
    JoinsDisallowed,
    /// Happens when already part of the network; we need to start from scratch.
    RemovedFromSection,
    /// Unrecoverable error, requires node operator network config.
    NodeNotReachable(SocketAddr),
}

impl RejoinReason {
    pub(crate) fn from_reject_reason(reason: JoinRejectReason) -> RejoinReason {
        use JoinRejectReason::*;
        match reason {
            JoinsDisallowed => RejoinReason::JoinsDisallowed,
            NodeNotReachable(add) => RejoinReason::NodeNotReachable(add),
        }
    }
}

/// Listens for incoming msgs and forms Cmds for each,
/// Periodically triggers other Cmd Processes (eg health checks, fault detection etc)
pub(crate) struct FlowCtrl {
    node: Arc<RwLock<MyNode>>,
    cmd_sender_channel: Sender<(Cmd, Vec<usize>)>,
    fault_channels: FaultChannels,
    timestamps: PeriodicChecksTimestamps,
}

impl FlowCtrl {
    /// Constructs a FlowCtrl instance, spawnning a task which starts processing messages,
    /// returning the channel where it can receive commands on
    pub(crate) async fn start(
        cmd_ctrl: CmdCtrl,
        incoming_msg_events: Receiver<CommEvent>,
        data_replication_receiver: Receiver<(Vec<DataAddress>, Peer)>,
        fault_cmds_channels: (Sender<FaultsCmd>, Receiver<FaultsCmd>),
    ) -> (Sender<(Cmd, Vec<usize>)>, Receiver<RejoinReason>) {
        trace!("[NODE READ]: flowctrl node context lock got");
        let node_context = cmd_ctrl.node().read().await.context();
        let (cmd_sender_channel, mut incoming_cmds_from_apis) = mpsc::channel(CMD_CHANNEL_SIZE);
        let (rejoin_network_tx, rejoin_network_rx) = mpsc::channel(STANDARD_CHANNEL_SIZE);

        let node_identifier = node_context.info.name();

        let all_members = node_context
            .network_knowledge
            .adults()
            .iter()
            .map(|peer| peer.name())
            .collect::<BTreeSet<XorName>>();
        let elders = node_context
            .network_knowledge
            .elders()
            .iter()
            .map(|peer| peer.name())
            .collect::<BTreeSet<XorName>>();
        let fault_channels = {
            let tracker = FaultDetection::new(all_members, elders);
            // start FaultDetection in a new thread
            let faulty_nodes_receiver = Self::start_fault_detection(tracker, fault_cmds_channels.1);
            FaultChannels {
                cmds_sender: fault_cmds_channels.0,
                faulty_nodes_receiver,
            }
        };

        let flow_ctrl = Self {
            node: cmd_ctrl.node(),
            cmd_sender_channel: cmd_sender_channel.clone(),
            fault_channels,
            timestamps: PeriodicChecksTimestamps::now(),
        };

        let _handle = tokio::task::spawn(flow_ctrl.process_messages_and_periodic_checks());

        let cmd_channel = cmd_sender_channel.clone();
        let cmd_channel_for_msgs = cmd_sender_channel.clone();

        let node_arc_for_replication = cmd_ctrl.node();
        // start a new thread to kick off incoming cmds
        let _handle = tokio::task::spawn(async move {
            // Get a stable identifier for statemap naming. This is NOT the node's current name.
            // It's the initial name... but will not change for the entire statemap
            while let Some((cmd, cmd_id)) = incoming_cmds_from_apis.recv().await {
                trace!("Taking cmd off stack: {cmd:?}");
                cmd_ctrl
                    .process_cmd_job(
                        cmd,
                        cmd_id,
                        node_identifier,
                        cmd_channel.clone(),
                        rejoin_network_tx.clone(),
                    )
                    .await
            }
        });

        Self::send_out_data_for_replication(
            node_arc_for_replication,
            node_context.data_storage,
            data_replication_receiver,
            cmd_sender_channel.clone(),
        )
        .await;

        Self::listen_for_comm_events(incoming_msg_events, cmd_channel_for_msgs);

        (cmd_sender_channel, rejoin_network_rx)
    }

    /// Listens on data_replication_receiver on a new thread, sorts and batches data, generating SendMsg Cmds
    async fn send_out_data_for_replication(
        node_arc: Arc<RwLock<MyNode>>,
        node_data_storage: DataStorage,
        mut data_replication_receiver: Receiver<(Vec<DataAddress>, Peer)>,
        cmd_channel: Sender<(Cmd, Vec<usize>)>,
    ) {
        // start a new thread to kick off data replication
        let _handle = tokio::task::spawn(async move {
            // is there a simple way to dedupe common data going to many peers?
            // is any overhead reduction worth the increased complexity?
            while let Some((data_addresses, peer)) = data_replication_receiver.recv().await {
                let send_cmd_channel = cmd_channel.clone();
                let the_node = node_arc.clone();
                let data_storage = node_data_storage.clone();
                // move replication off thread so we don't block the receiver
                let _handle = tokio::task::spawn(async move {
                    debug!(
                        "{:?} Data {:?} to: {:?}",
                        LogMarker::SendingMissingReplicatedData,
                        data_addresses,
                        peer,
                    );

                    let mut data_bundle = vec![];

                    for address in data_addresses.iter() {
                        match data_storage.get_from_local_store(address).await {
                            Ok(data) => {
                                data_bundle.push(data);
                            }
                            Err(error) => {
                                error!("Error getting {address:?} from local storage during data replication flow: {error:?}");
                            }
                        };
                    }
                    trace!("Sending out data batch to {peer:?}");
                    let msg = NodeMsg::NodeDataCmd(NodeDataCmd::ReplicateDataBatch(data_bundle));

                    let node_context = the_node.read().await.context();

                    let cmd = Cmd::send_msg(msg, Peers::Single(peer), node_context.clone());
                    if let Err(error) = send_cmd_channel.send((cmd, vec![])).await {
                        error!("Failed to enqueue send msg command for replication of data batch to {peer:?}: {error:?}");
                    }
                });
            }
        });
    }

    /// This is a never ending loop as long as the node is live.
    /// This loop drives the periodic events internal to the node.
    async fn process_messages_and_periodic_checks(mut self) {
        // the internal process loop
        loop {
            self.perform_periodic_checks().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    // starts a new thread to convert comm event to cmds
    fn listen_for_comm_events(
        mut incoming_msg_events: Receiver<CommEvent>,
        cmd_channel_for_msgs: Sender<(Cmd, Vec<usize>)>,
    ) {
        let _handle = tokio::task::spawn(async move {
            while let Some(event) = incoming_msg_events.recv().await {
                debug!("CmdEvent received: {event:?}");
                let cmd = match event {
                    CommEvent::Error { peer, error } => Cmd::HandleCommsError { peer, error },
                    CommEvent::Msg(MsgFromPeer {
                        sender,
                        wire_msg,
                        send_stream,
                    }) => {
                        if let Ok((header, dst, payload)) = wire_msg.serialize() {
                            let original_bytes_len = header.len() + dst.len() + payload.len();
                            let span =
                                trace_span!("handle_message", ?sender, msg_id = ?wire_msg.msg_id());
                            let _span_guard = span.enter();
                            trace!(
                                "{:?} from {sender:?} length {original_bytes_len}",
                                LogMarker::MsgReceived,
                            );
                        } else {
                            // this should be unreachable
                            trace!(
                                "{:?} from {sender:?}, unknown length due to serialization issues.",
                                LogMarker::MsgReceived,
                            );
                        }

                        Cmd::HandleMsg {
                            origin: sender,
                            wire_msg,
                            send_stream,
                        }
                    }
                };

                // this await prevents us pulling more msgs than the cmd handler can cope with...
                // feeding back up the channels to qp2p and quinn where congestion control should
                // help prevent more messages incoming for the time being
                if let Err(error) = cmd_channel_for_msgs.send((cmd, vec![])).await {
                    error!("Error sending msg onto cmd channel {error:?}");
                }
            }
        });
    }
}
