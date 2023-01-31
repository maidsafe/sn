// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{
    cfg::keypair_storage::{get_reward_pk, store_network_keypair, store_new_reward_keypair},
    flow_ctrl::{
        cmds::Cmd, dispatcher::Dispatcher, fault_detection::FaultsCmd, CmdCtrl, FlowCtrl,
        RejoinReason,
    },
    logging::log_system_details,
    Config, Error, MyNode, Result, STANDARD_CHANNEL_SIZE,
};
use crate::UsedSpace;

use sn_comms::Comm;
use sn_interface::{
    network_knowledge::{NetworkKnowledge, SectionTree, MIN_ADULT_AGE},
    types::{keys::ed25519, log_markers::LogMarker, PublicKey as TypesPublicKey},
};

use rand_07::rngs::OsRng;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{
    fs,
    sync::{mpsc, RwLock},
};
use xor_name::Prefix;

// Filename for storing the content of the genesis DBC.
// The Genesis DBC is generated and owned by the genesis PK of the network's section chain,
// i.e. the very first section key in the chain.
// The first node mints the genesis DBC (as a bearer DBC) and stores it in a file
// named `genesis_dbc`, located at it's configured root dir.
// In current implementation the amount owned by the Genesis DBC is
// set to GENESIS_DBC_AMOUNT (currently 4,525,524,120 * 10^9) individual units.
const GENESIS_DBC_FILENAME: &str = "genesis_dbc";

pub(crate) type CmdChannel = mpsc::Sender<(Cmd, Vec<usize>)>;

/// Test only
pub async fn new_test_api(config: &Config, join_timeout: Duration) -> Result<super::NodeTestApi> {
    let (node, cmd_channel, _) = new_node(config, join_timeout).await?;
    Ok(super::NodeTestApi::new(node, cmd_channel))
}

/// A reference held to the node to keep it running.
///
/// Meant to be held while looping over the event receiver
/// that transports events from the node.
#[allow(missing_debug_implementations, dead_code)]
pub struct NodeRef {
    node: Arc<RwLock<MyNode>>,
    /// Sender which can be used to add a Cmd to the Node's CmdQueue
    cmd_channel: CmdChannel,
}

/// Start a new node.
pub async fn start_new_node(
    config: &Config,
    join_timeout: Duration,
) -> Result<(NodeRef, mpsc::Receiver<RejoinReason>)> {
    let (node, cmd_channel, rejoin_network_rx) = new_node(config, join_timeout).await?;

    Ok((NodeRef { node, cmd_channel }, rejoin_network_rx))
}

// Private helper to create a new node using the given config and bootstraps it to the network.
async fn new_node(
    config: &Config,
    join_timeout: Duration,
) -> Result<(
    Arc<RwLock<MyNode>>,
    CmdChannel,
    mpsc::Receiver<RejoinReason>,
)> {
    let root_dir_buf = config.root_dir()?;
    let root_dir = root_dir_buf.as_path();
    fs::create_dir_all(root_dir).await?;

    let _reward_key = match get_reward_pk(root_dir).await? {
        Some(public_key) => TypesPublicKey::Ed25519(public_key),
        None => {
            let mut rng = OsRng;
            let keypair = ed25519_dalek::Keypair::generate(&mut rng);
            store_new_reward_keypair(root_dir, &keypair).await?;
            TypesPublicKey::Ed25519(keypair.public)
        }
    };

    let used_space = UsedSpace::new(config.min_capacity(), config.max_capacity());

    let (node, cmd_channel, rejoin_network_rx) =
        bootstrap_node(config, used_space, root_dir, join_timeout).await?;

    {
        debug!("[NODE WRITE]: new node...");
        let context = node.read().await.context();
        debug!("[NODE WRITE]: new node write got");

        // Network keypair may have to be changed due to naming criteria or network requirements.
        let keypair_as_bytes = context.keypair.to_bytes();
        store_network_keypair(root_dir, keypair_as_bytes).await?;

        let our_pid = std::process::id();
        let node_prefix = context.network_knowledge.prefix();
        let node_name = context.name;
        let node_age = context.info.age();
        let our_conn_info = context.info.addr;
        let our_conn_info_json = serde_json::to_string(&our_conn_info)
            .unwrap_or_else(|_| "Failed to serialize connection info".into());
        println!(
            "Node PID: {our_pid:?}, prefix: {node_prefix:?}, \
            name: {node_name:?}, age: {node_age}, connection info:\n{our_conn_info_json}",
        );
        info!(
            "Node PID: {:?}, prefix: {:?}, name: {:?}, age: {}, connection info: {}",
            our_pid, node_prefix, node_name, node_age, our_conn_info_json,
        );

        log_system_details(node_prefix);
    }

    Ok((node, cmd_channel, rejoin_network_rx))
}

// Private helper to create a new node using the given config and bootstraps it to the network.
async fn bootstrap_node(
    config: &Config,
    used_space: UsedSpace,
    root_storage_dir: &Path,
    join_timeout: Duration,
) -> Result<(
    Arc<RwLock<MyNode>>,
    CmdChannel,
    mpsc::Receiver<RejoinReason>,
)> {
    let (incoming_msg_pipe, incoming_msg_receiver) = mpsc::channel(STANDARD_CHANNEL_SIZE);
    let (fault_cmds_sender, fault_cmds_receiver) =
        mpsc::channel::<FaultsCmd>(STANDARD_CHANNEL_SIZE);

    let comm = Comm::new(config.local_addr(), incoming_msg_pipe).await?;

    let node = if config.is_first() {
        start_genesis_node(
            comm,
            used_space,
            root_storage_dir,
            fault_cmds_sender.clone(),
        )
        .await?
    } else {
        start_node(
            config,
            comm,
            used_space,
            root_storage_dir,
            fault_cmds_sender.clone(),
        )
        .await?
    };

    let node = Arc::new(RwLock::new(node));
    let (dispatcher, data_replication_receiver) = Dispatcher::new(node.clone());
    let cmd_ctrl = CmdCtrl::new(dispatcher);
    let (cmd_channel, rejoin_network_rx) = FlowCtrl::start(
        cmd_ctrl,
        incoming_msg_receiver,
        data_replication_receiver,
        (fault_cmds_sender, fault_cmds_receiver),
    )
    .await;

    cmd_channel
        .send((Cmd::TryJoinNetwork, vec![]))
        .await
        .map_err(|e| {
            error!("Failed join: {:?}", e);
            Error::JoinTimeout
        })?;

    tokio::time::timeout(join_timeout, await_join(node.clone()))
        .await
        .map_err(|e| {
            error!("Failed join: {:?}", e);
            Error::JoinTimeout
        })?;

    Ok((node, cmd_channel, rejoin_network_rx))
}

async fn await_join(node: Arc<RwLock<MyNode>>) {
    let mut is_member = false;
    while !is_member {
        let read_only = node.read().await;
        let our_name = read_only.name();
        is_member = read_only.network_knowledge.is_section_member(&our_name);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn start_genesis_node(
    comm: Comm,
    used_space: UsedSpace,
    root_storage_dir: &Path,
    fault_cmds_sender: mpsc::Sender<FaultsCmd>,
) -> Result<MyNode> {
    // Genesis node having a fix age of 255.
    let keypair = ed25519::gen_keypair(&Prefix::default().range_inclusive(), 255);
    let node_name = ed25519::name(&keypair.public);

    info!(
        "{} Starting a new network as the genesis node (PID: {}).",
        node_name,
        std::process::id()
    );

    // Generate the genesis key, this will be the first key in the sections chain,
    // as well as the owner of the genesis DBC minted by this first node of the network.
    let genesis_sk_set = bls::SecretKeySet::random(0, &mut rand::thread_rng());
    let (node, genesis_dbc) = MyNode::first_node(
        comm,
        keypair,
        used_space.clone(),
        root_storage_dir.to_path_buf(),
        genesis_sk_set,
        fault_cmds_sender,
    )
    .await?;

    // Write the genesis DBC to disk
    let path = root_storage_dir.join(GENESIS_DBC_FILENAME);
    fs::write(path, genesis_dbc.to_hex()?).await?;

    info!("{}", LogMarker::PromotedToElder);

    let genesis_key = node.network_knowledge.genesis_key();
    info!(
        "{} Genesis node started!. Genesis key {:?}, hex: {}",
        node_name,
        genesis_key,
        hex::encode(genesis_key.to_bytes())
    );

    Ok(node)
}

#[allow(clippy::too_many_arguments)]
async fn start_node(
    config: &Config,
    comm: Comm,
    used_space: UsedSpace,
    root_storage_dir: &Path,
    fault_cmds_sender: mpsc::Sender<FaultsCmd>,
) -> Result<MyNode> {
    let keypair = ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE);
    let node_name = ed25519::name(&keypair.public);
    info!("{} Bootstrapping as a new node.", node_name);
    let section_tree_path = config.network_contacts_file().ok_or_else(|| {
        Error::Configuration("Could not obtain network contacts file path".to_string())
    })?;
    let section_tree = SectionTree::from_disk(&section_tree_path).await?;
    let sap = section_tree.get_signed_by_name(&node_name)?;
    let network_knowledge = NetworkKnowledge::new(sap.prefix(), section_tree.clone())?;

    info!(
        "{} Starting a new node (PID: {}) with socket: {}, network's genesis key: {:?}",
        node_name,
        std::process::id(),
        comm.socket_addr(),
        section_tree.genesis_key()
    );

    let node = MyNode::new(
        comm,
        Arc::new(keypair),
        network_knowledge,
        None,
        used_space.clone(),
        root_storage_dir.to_path_buf(),
        fault_cmds_sender,
    )
    .await?;

    info!("Node {} started.", node.info().name());

    Ok(node)
}
