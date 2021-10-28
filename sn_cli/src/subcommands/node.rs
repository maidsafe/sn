// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use crate::operations::{config::Config, node::*};
use color_eyre::{eyre::eyre, Result};
use sn_api::PublicKey;
use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf};
use structopt::StructOpt;
use tracing::debug;

const NODES_DATA_FOLDER: &str = "baby-fleming-nodes";

const LOCAL_NODE_DIR: &str = "local-node";

#[derive(StructOpt, Debug)]
pub enum NodeSubCommands {
    /// Gets the version of `sn_node` binary
    BinVersion {
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
    },
    #[structopt(name = "install")]
    /// Install latest sn_node released version in the system
    Install {
        #[structopt(long = "node-path")]
        /// Path where to install sn_node executable (default ~/.safe/node/). The SN_NODE_PATH env var can also be used to set the path
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
        /// Specify the version of sn_node to install. If not supplied, the latest version will be
        /// installed. Note: just the version number should be supplied, with no 'v' prefix.
        #[structopt(short = "v", long)]
        version: Option<String>,
    },
    #[structopt(name = "join")]
    /// Join an already running network
    Join {
        /// Network to have the node to join to
        network_name: Option<String>,
        #[structopt(long = "node-path")]
        /// Path where to run sn_node executable from (default ~/.safe/node/). The SN_NODE_PATH env var can also be used to set the path
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
        /// Vebosity level for nodes logs
        #[structopt(short = "y", parse(from_occurrences))]
        verbosity: u8,
        /// Hardcoded contacts (endpoints) to be used to bootstrap to an already running network (this overrides any value passed as 'network_name').
        #[structopt(short = "h", long = "hcc")]
        hard_coded_contacts: Vec<SocketAddr>,
        /// Internal address provided for the node
        #[structopt(short = "l", long)]
        local_addr: Option<SocketAddr>,
        #[structopt(short = "p", long)]
        /// External address provided for the node
        public_addr: Option<SocketAddr>,
        /// Delete all data from a previous node running on the same PC
        #[structopt(long = "clear-data")]
        clear_data: bool,
    },
    #[structopt(name = "run-baby-fleming")]
    /// Run nodes to form a local single-section Safe network
    Run {
        /// Path where to run sn_node executable from (default ~/.safe/node/). The SN_NODE_PATH env var can also be used to set the path
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
        /// Vebosity level for nodes logs (default = INFO, -y = DEBUG, -yy = TRACE)
        #[structopt(short = "y", parse(from_occurrences))]
        verbosity: u8,
        /// Interval in seconds between launching each of the nodes
        #[structopt(short = "i", long, default_value = "1")]
        interval: u64,
        /// Number of nodes to be launched
        #[structopt(long = "nodes", default_value = "11")]
        num_of_nodes: u8,
        /// IP to be used to launch the local nodes.
        #[structopt(long = "ip")]
        ip: Option<String>,
        /// Start authd and log in with
        #[structopt(short = "t", long = "testing")]
        test: bool,
    },
    /// Shutdown all running nodes processes
    #[structopt(name = "killall")]
    Killall {
        /// Path of the sn_node executable used to launch the processes with (default ~/.safe/node/sn_node). The SN_NODE_PATH env var can be also used to set this path
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
    },
    #[structopt(name = "update")]
    /// Update to latest sn_node released version
    Update {
        #[structopt(long = "node-path")]
        /// Path of the sn_node executable to update (default ~/.safe/node/). The SN_NODE_PATH env var can be also used to set the path
        #[structopt(long = "node-path", env = "SN_NODE_PATH")]
        node_path: Option<PathBuf>,
    },
}

pub async fn node_commander(cmd: Option<NodeSubCommands>, config: &mut Config) -> Result<()> {
    match cmd {
        Some(NodeSubCommands::BinVersion { node_path }) => node_version(node_path),
        Some(NodeSubCommands::Install { node_path, version }) => {
            // We run this command in a separate thread to overcome a conflict with
            // the self_update crate as it seems to be creating its own runtime.
            let handler = std::thread::spawn(|| node_install(node_path, version));
            handler
                .join()
                .map_err(|err| eyre!("Failed to run self update: {:?}", err))?
        }
        Some(NodeSubCommands::Join {
            network_name,
            node_path,
            verbosity,
            hard_coded_contacts,
            local_addr,
            public_addr,
            clear_data,
        }) => {
            let network_contacts = if hard_coded_contacts.is_empty() {
                if let Some(name) = network_name {
                    let msg = format!("Joining the '{}' network...", name);
                    debug!("{}", msg);
                    println!("{}", msg);
                    config.get_network_info(&name).await?
                } else {
                    let (_, contacts) = config.read_current_node_config()?;
                    contacts
                }
            } else {
                let genesis_key = PublicKey::bls_from_hex("8640e62cc44e75cf4fadc8ee91b74b4cf0fd2c0984fb0e3ab40f026806857d8c41f01d3725223c55b1ef87d669f5e2cc")?
                    .bls()
                    .ok_or_else(|| eyre!("Unexpectedly failed to obtain (BLS) genesis key."))?;
                let mut set: BTreeSet<SocketAddr> = BTreeSet::new();
                for contact in hard_coded_contacts {
                    set.insert(contact);
                }
                (genesis_key, set)
            };

            let msg = format!("Joining network with contacts {:?} ...", network_contacts);
            debug!("{}", msg);
            println!("{}", msg);

            node_join(
                node_path,
                LOCAL_NODE_DIR,
                verbosity,
                &network_contacts.1,
                local_addr,
                public_addr,
                clear_data,
            )
        }
        Some(NodeSubCommands::Run {
            node_path,
            verbosity,
            interval,
            num_of_nodes,
            ip,
            test,
        }) => node_run(
            node_path,
            NODES_DATA_FOLDER,
            verbosity,
            &interval.to_string(),
            &num_of_nodes.to_string(),
            ip,
            test,
        ),
        Some(NodeSubCommands::Killall { node_path }) => node_shutdown(node_path),
        Some(NodeSubCommands::Update { node_path }) => node_update(node_path),
        None => Err(eyre!("Missing node subcommand")),
    }
}
