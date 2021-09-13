// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{Error, Result};
use crate::routing::NetworkConfig;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{self},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};
use structopt::StructOpt;
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};
use tracing::{debug, warn, Level};

const CONFIG_FILE: &str = "node.config";
const CONNECTION_INFO_FILE: &str = "node_connection_info.config";
const DEFAULT_ROOT_DIR_NAME: &str = "root_dir";
const DEFAULT_MAX_CAPACITY: u64 = 2 * 1024 * 1024 * 1024;

/// Node configuration
#[derive(Default, Clone, Debug, Serialize, Deserialize, Eq, PartialEq, StructOpt)]
#[structopt(rename_all = "kebab-case", bin_name = "sn_node")]
#[structopt(global_settings = &[structopt::clap::AppSettings::ColoredHelp])]
pub struct Config {
    /// The address to be credited when this node farms SafeCoin.
    /// A hex formatted BLS public key.
    #[structopt(short, long, parse(try_from_str))]
    pub wallet_id: Option<String>,
    /// Upper limit in bytes for allowed network storage on this node.
    #[structopt(short, long)]
    pub max_capacity: Option<u64>,
    /// Root directory for dbs and cached state. If not set, it defaults to "root_dir"
    /// within the sn_node project data directory, located at:
    /// Linux: $HOME/.safe/node/root_dir
    /// Windows: {FOLDERID_Profile}/.safe/node/root_dir
    /// MacOS: $HOME/.safe/node/root_dir
    #[structopt(short, long, parse(from_os_str))]
    pub root_dir: Option<PathBuf>,
    /// Verbose output. `-v` is equivalent to logging with `warn`, `-vv` to `info`, `-vvv` to
    /// `debug`, `-vvvv` to `trace`. This flag overrides RUST_LOG.
    #[structopt(short, long, parse(from_occurrences))]
    pub verbose: u8,
    /// dump shell completions for: [bash, fish, zsh, powershell, elvish]
    #[structopt(long)]
    pub completions: Option<String>,
    /// Send logs to a file within the specified directory
    #[structopt(long)]
    pub log_dir: Option<PathBuf>,
    /// Attempt to self-update?
    #[structopt(long)]
    pub update: bool,
    /// Attempt to self-update without starting the node process
    #[structopt(long)]
    pub update_only: bool,
    /// Outputs logs in json format for easier processing
    #[structopt(short, long)]
    pub json_logs: bool,
    /// print node resourse usage to stdout
    #[structopt(long)]
    pub resource_logs: bool,
    /// Delete all data from a previous node running on the same PC
    #[structopt(long)]
    pub clear_data: bool,
    /// If the node is the first node on the network, the local address to be used should be passed.
    /// To use a random port number, use 0. If this argument is passed `--local-ip` and `--local-port`
    /// is not requried, however if they are passed, they should match the value provided here.
    #[structopt(long)]
    pub first: Option<SocketAddr>,
    /// Local address to be used for the node. This field is mandatory if manual port forwarding is being used.
    /// Otherwise, the value is fetched from `--first` (for genesis) and obtained by connecting to the
    /// bootstrap node otherwise.
    #[structopt(long)]
    pub local_addr: Option<SocketAddr>,
    /// External address of the node. This field can be used to specify the external socket address when
    /// manual port forwarding is used. If this field is provided, either `--first` or `--local-addr` must
    /// be provided
    #[structopt(long)]
    pub public_addr: Option<SocketAddr>,
    /// This flag can be used to skip port forwarding using IGD. This is used when running a network on LAN
    /// or when a node is connected to the internect directly without a router. Eg. Digital Ocean droplets.
    #[structopt(long)]
    pub skip_igd: bool,
    /// Hard Coded contacts
    #[structopt(
        short,
        long,
        default_value = "[]",
        parse(try_from_str = serde_json::from_str)
    )]
    pub hard_coded_contacts: BTreeSet<SocketAddr>,
    /// Genesis key of the network in hex format.
    #[structopt(long)]
    pub genesis_key: Option<String>,
    /// This is the maximum message size we'll allow the peer to send to us. Any bigger message and
    /// we'll error out probably shutting down the connection to the peer. If none supplied we'll
    /// default to the documented constant.
    #[structopt(long)]
    pub max_msg_size_allowed: Option<u32>,
    /// If we hear nothing from the peer in the given interval we declare it offline to us. If none
    /// supplied we'll default to the documented constant.
    ///
    /// The interval is in milliseconds. A value of 0 disables this feature.
    #[structopt(long)]
    pub idle_timeout_msec: Option<u64>,
    /// Interval to send keep-alives if we are idling so that the peer does not disconnect from us
    /// declaring us offline. If none is supplied we'll default to the documented constant.
    ///
    /// The interval is in milliseconds. A value of 0 disables this feature.
    #[structopt(long)]
    pub keep_alive_interval_msec: Option<u32>,
    /// Directory in which the bootstrap cache will be stored. If none is supplied, the platform specific
    /// default cache directory is used.
    #[structopt(long)]
    pub bootstrap_cache_dir: Option<String>,
    /// Duration of a UPnP port mapping.
    #[structopt(long)]
    pub upnp_lease_duration: Option<u32>,
    #[structopt(skip)]
    #[allow(missing_docs)]
    pub network_config: NetworkConfig,
}

impl Config {
    /// Returns a new `Config` instance.  Tries to read from the default node config file location,
    /// and overrides values with any equivalent command line args.
    pub async fn new() -> Result<Self, Error> {
        // FIXME: Re-enable when we have rejoins working
        // let mut config = match Self::read_from_file() {
        //     Ok(Some(config)) => config,
        //     Ok(None) | Err(_) => Default::default(),
        // };

        let mut config = Config::default();

        let mut command_line_args = Config::from_args();
        command_line_args.validate()?;

        if let Some(socket_addr) = command_line_args.first {
            command_line_args.local_addr = Some(socket_addr);
        }

        if command_line_args.hard_coded_contacts.is_empty() {
            debug!("Using node connection config file as no hard coded contacts were passed in");
            if let Ok((_, info)) = read_conn_info_from_file().await {
                command_line_args.hard_coded_contacts = info;
            }
        }

        config.merge(command_line_args);

        config.clear_data_from_disk().await.unwrap_or_else(|_| {
            tracing::error!("Error deleting data file from disk");
        });

        Ok(config)
    }

    fn validate(&mut self) -> Result<(), Error> {
        if self.public_addr.is_some() && self.first.is_none() && self.local_addr.is_none() {
            return Err(Error::Configuration("--public-addr passed without specifing local address using --first or --local-addr".to_string()));
        }

        if self.public_addr.is_none() && self.local_addr.is_some() {
            if self.skip_igd {
                // local_addr duplicated to public_addr so that the specified port is used (and not a random one)
                self.public_addr = self.local_addr;
            } else {
                println!("Warning: Local Address provided is skipped since external address is not provided.");
                self.local_addr = None;
            }
        }
        Ok(())
    }

    /// Overwrites the current config with the provided values from another config
    fn merge(&mut self, config: Config) {
        if let Some(wallet_id) = config.wallet_id() {
            self.wallet_id = Some(wallet_id.clone());
        }

        if let Some(max_capacity) = &config.max_capacity {
            self.max_capacity = Some(*max_capacity);
        }

        if let Some(root_dir) = &config.root_dir {
            self.root_dir = Some(root_dir.clone());
        }

        self.json_logs = config.json_logs;
        self.resource_logs = config.resource_logs;

        if config.verbose > 0 {
            self.verbose = config.verbose;
        }

        if let Some(completions) = &config.completions {
            self.completions = Some(completions.clone());
        }

        if let Some(log_dir) = &config.log_dir {
            self.log_dir = Some(log_dir.clone());
        }

        self.update = config.update || self.update;
        self.update_only = config.update_only || self.update_only;
        self.clear_data = config.clear_data || self.clear_data;

        if let Some(socket_addr) = config.first {
            self.first = Some(socket_addr);
            self.local_addr = Some(socket_addr);
        }

        if let Some(local_addr) = config.local_addr {
            self.local_addr = Some(local_addr);
        }

        if let Some(public_addr) = config.public_addr {
            self.public_addr = config.public_addr;
            self.network_config.external_port = Some(public_addr.port());
            self.network_config.external_ip = Some(public_addr.ip());
        }

        self.network_config.forward_port = !config.skip_igd;

        if !config.hard_coded_contacts.is_empty() {
            self.hard_coded_contacts = config.hard_coded_contacts;
        }

        if config.genesis_key.is_some() {
            self.genesis_key = config.genesis_key;
        }

        if let Some(max_msg_size) = config.max_msg_size_allowed {
            self.max_msg_size_allowed = Some(max_msg_size);
        }

        if let Some(idle_timeout) = config.idle_timeout_msec {
            self.idle_timeout_msec = Some(idle_timeout);
        }

        if let Some(keep_alive) = config.keep_alive_interval_msec {
            self.keep_alive_interval_msec = Some(keep_alive);
        }

        if let Some(bootstrap_cache_dir) = config.bootstrap_cache_dir {
            self.bootstrap_cache_dir = Some(bootstrap_cache_dir);
        }

        if let Some(upnp_lease_duration) = config.upnp_lease_duration {
            self.network_config.upnp_lease_duration =
                Some(Duration::from_millis(upnp_lease_duration as u64));
        }
    }

    /// The address to be credited when this node farms SafeCoin.
    pub fn wallet_id(&self) -> Option<&String> {
        self.wallet_id.as_ref()
    }

    /// Is this the first node in a section?
    pub fn is_first(&self) -> bool {
        self.first.is_some()
    }

    /// Upper limit in bytes for allowed network storage on this node.
    pub fn max_capacity(&self) -> u64 {
        self.max_capacity.unwrap_or(DEFAULT_MAX_CAPACITY)
    }

    /// Root directory for dbs and cached state. If not set, it defaults to
    /// `DEFAULT_ROOT_DIR_NAME` within the project's data directory (see `Config::root_dir` for the
    /// directories on each platform).
    pub fn root_dir(&self) -> Result<PathBuf> {
        Ok(match &self.root_dir {
            Some(root_dir) => root_dir.clone(),
            None => project_dirs()?.join(DEFAULT_ROOT_DIR_NAME),
        })
    }

    /// Set the root directory for dbs and cached state.
    pub fn set_root_dir<P: Into<PathBuf>>(&mut self, path: P) {
        self.root_dir = Some(path.into())
    }

    /// Set the directory to write the logs.
    pub fn set_log_dir<P: Into<PathBuf>>(&mut self, path: P) {
        self.log_dir = Some(path.into())
    }

    /// Get the log level.
    pub fn verbose(&self) -> Level {
        match self.verbose {
            0 => Level::ERROR,
            1 => Level::WARN,
            2 => Level::INFO,
            3 => Level::DEBUG,
            _ => Level::TRACE,
        }
    }

    /// Network configuration options.
    pub fn network_config(&self) -> &NetworkConfig {
        &self.network_config
    }

    /// Set network configuration options.
    pub fn set_network_config(&mut self, config: NetworkConfig) {
        self.network_config = config;
    }

    /// Get the completions option
    pub fn completions(&self) -> &Option<String> {
        &self.completions
    }

    /// Directory where to write log file/s if specified
    pub fn log_dir(&self) -> &Option<PathBuf> {
        &self.log_dir
    }

    /// Attempt to self-update?
    pub fn update(&self) -> bool {
        self.update
    }

    /// Attempt to self-update without starting the node process
    pub fn update_only(&self) -> bool {
        self.update_only
    }

    // Clear data from of a previous node running on the same PC
    async fn clear_data_from_disk(&self) -> Result<()> {
        if self.clear_data {
            let path = project_dirs()?.join(self.root_dir()?);
            if path.exists() {
                fs::remove_dir_all(&path).await?;
            }
        }
        Ok(())
    }

    /// Reads the default node config file.
    #[allow(unused)]
    async fn read_from_file() -> Result<Option<Config>> {
        let path = project_dirs()?.join(CONFIG_FILE);

        match fs::read(path.clone()).await {
            Ok(content) => {
                debug!("Reading settings from {}", path.display());

                serde_json::from_slice(&content).map_err(|err| {
                    warn!(
                        "Could not parse content of config file '{:?}': {:?}",
                        path, err
                    );
                    err.into()
                })
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    debug!("No config file available at {:?}", path);
                    Ok(None)
                } else {
                    Err(error.into())
                }
            }
        }
    }

    /// Writes the config file to disk
    pub async fn write_to_disk(&self) -> Result<()> {
        write_file(CONFIG_FILE, self).await
    }
}

/// Overwrites connection info at file.
///
/// The file is written to the `current_bin_dir()` with the appropriate file name.
pub async fn set_connection_info(genesis_key: bls::PublicKey, contact: SocketAddr) -> Result<()> {
    let genesis_key_hex = hex::encode(genesis_key.to_bytes());
    write_file(CONNECTION_INFO_FILE, &(genesis_key_hex, vec![contact])).await
}

/// Writes connection info to file for use by clients (and joining nodes when local network).
///
/// The file is written to the `current_bin_dir()` with the appropriate file name.
pub async fn add_connection_info(contact: SocketAddr) -> Result<()> {
    let (genesis_key_hex, mut bootstrap_nodes) = read_conn_info_from_file().await?;
    let _ = bootstrap_nodes.insert(contact);
    write_file(CONNECTION_INFO_FILE, &(genesis_key_hex, bootstrap_nodes)).await
}

/// Reads the default node config file.
async fn read_conn_info_from_file() -> Result<(String, BTreeSet<SocketAddr>)> {
    let path = project_dirs()?.join(CONNECTION_INFO_FILE);

    match fs::read(&path).await {
        Ok(content) => {
            debug!("Reading connection info from {}", path.display());
            let config = serde_json::from_slice(&content)?;
            Ok(config)
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::NotFound {
                debug!("No connection info file available at {}", path.display());
            }
            Err(error.into())
        }
    }
}

async fn write_file<T: ?Sized>(file: &str, config: &T) -> Result<()>
where
    T: Serialize,
{
    let project_dirs = project_dirs()?;
    fs::create_dir_all(project_dirs.clone()).await?;

    let path = project_dirs.join(file);
    let mut file = File::create(&path).await?;
    let serialized = serde_json::to_string_pretty(config)?;
    file.write_all(serialized.as_bytes()).await?;
    file.sync_all().await?;
    Ok(())
}

fn project_dirs() -> Result<PathBuf> {
    let mut home_dir = dirs_next::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory not found"))?;

    home_dir.push(".safe");
    home_dir.push("node");

    Ok(home_dir)
}

#[test]
fn smoke() {
    // NOTE: IF this value is being changed due to a change in the config,
    // the change in config also be handled in Config::merge()
    // and in examples/config_handling.rs
    let expected_size = 456;

    assert_eq!(std::mem::size_of::<Config>(), expected_size);
}
