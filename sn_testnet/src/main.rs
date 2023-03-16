// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! sn_node provides the interface to Safe routing.  The resulting executable is the node
//! for the Safe network.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/maidsafe/QA/master/Images/maidsafe_logo.png",
    html_favicon_url = "https://maidsafe.net/img/favicon.ico",
    test(attr(deny(warnings)))
)]
// For explanation of lint checks, run `rustc -W help`.
#![forbid(unsafe_code)]
#![warn(
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]

use clap::Parser;
use color_eyre::{eyre::eyre, Help, Result};
use sn_testnet::{Testnet, DEFAULT_NODE_LAUNCH_INTERVAL, SAFENODE_BIN_NAME};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

const BASE_TRACING_DIRECTIVES: &str = "sn_testnet=debug";
const DEFAULT_NODE_COUNT: u32 = 30;

#[derive(Debug, clap::StructOpt)]
#[clap(name = "testnet", version)]
struct Cmd {
    /// Launched nodes will join an existing testnet.
    #[clap(long = "join", short = 'j', value_parser)]
    join_network: bool,

    /// Optional location for a network contacts file.
    ///
    /// This should only be used in conjunction with the 'join' command. You can supply it if you
    /// have an existing network contacts path and you want to launch nodes perhaps on another
    /// machine in your local network and have them join an existing testnet on the same network.
    ///
    /// If the value is not supplied, we will use the default location for the contacts file, which
    /// will be at node-data-dir/local-test-network/sn-genesis-node/section_tree.
    #[clap(long = "network-contacts-path", short = 'n', value_name = "FILE_PATH")]
    network_contacts_path: Option<PathBuf>,

    /// Interval between node launches in ms. Defaults to 5000.
    #[clap(long = "interval", short = 'i')]
    node_launch_interval: Option<u64>,

    /// Use flamegraph setup.
    ///
    /// Flamegraph will elevate to root, so log output will need to be deleted as root.
    ///
    /// Windows is not supported.
    #[clap(long, short = 'f')]
    flame: bool,

    /// Build the node from source.
    ///
    /// This assumes you're running the process from the `safe_network` repository.
    #[clap(long, short = 'b')]
    build_node: bool,

    /// Optional path to the safenode binary.
    ///
    /// This will take precedence over the --build-node flag and effectively ignore it.
    ///
    /// If not supplied we will assume that safenode is on PATH.
    #[clap(short = 'p', long, value_name = "FILE_PATH")]
    node_path: Option<PathBuf>,

    /// The number of nodes for the testnet. Defaults to 30.
    ///
    /// If you use the 'join' command, you must supply this value.
    #[clap(short = 'c', long, env = "NODE_COUNT")]
    node_count: Option<u32>,

    /// Specify any additional arguments to pass to safenode on launch, e.g., --json-logs.
    ///
    /// Any arguments must be valid safenode arguments.
    #[clap(last = true)]
    node_args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    init_tracing()?;

    let args = Cmd::from_args();
    if args.flame {
        check_flamegraph_prerequisites().await?;
    }

    let mut node_bin_path = PathBuf::new();
    if let Some(node_path) = args.node_path {
        node_bin_path.push(node_path);
    } else if args.build_node {
        build_node().await?;
        node_bin_path.push("target");
        node_bin_path.push("release");
        node_bin_path.push(SAFENODE_BIN_NAME);
    } else {
        node_bin_path.push(SAFENODE_BIN_NAME);
    }

    if args.join_network {
        let node_count = args.node_count.ok_or_else(|| {
            eyre!("A node count must be specified for joining an existing network")
                .suggestion("Please try again using the --node-count argument")
        })?;
        join_network(
            node_bin_path,
            args.node_launch_interval
                .unwrap_or(DEFAULT_NODE_LAUNCH_INTERVAL),
            node_count,
            args.network_contacts_path,
            args.node_args,
        )
        .await?;
        return Ok(());
    }

    run_network(
        node_bin_path,
        args.node_launch_interval
            .unwrap_or(DEFAULT_NODE_LAUNCH_INTERVAL),
        args.node_count.unwrap_or(DEFAULT_NODE_COUNT),
        args.node_args,
        args.flame,
    )
    .await?;

    Ok(())
}

async fn check_flamegraph_prerequisites() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return Err(eyre!("Flamegraph cannot be used on Windows"));
    }

    let output = Command::new("cargo")
        .arg("install")
        .arg("--list")
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !stdout.contains("flamegraph") {
        return Err(
            eyre!("You do not appear to have an installation of flamegraph")
                .suggestion("Please run 'cargo flamegraph install' and try again"),
        );
    }

    let output = Command::new("which").arg("perf").output()?;
    if !output.status.success() {
        return Err(eyre!(
            "You do not appear to have the 'perf' tool installed, which is required for \
                using flamegraph"
        )
        .suggestion("Please install 'perf' on your OS"));
    }

    Ok(())
}

async fn build_node() -> Result<()> {
    let mut args = vec!["build", "--release"];

    // Keep features consistent to avoid recompiling.
    if cfg!(feature = "chaos") {
        println!("*** Building testnet with CHAOS enabled. Watch out. ***");
        args.push("--features");
        args.push("chaos");
    }
    if cfg!(feature = "statemap") {
        args.extend(["--features", "statemap"]);
    }
    if cfg!(feature = "otlp") {
        args.extend(["--features", "otlp"]);
    }
    if cfg!(feature = "unstable-wiremsg-debuginfo") {
        args.extend(["--features", "unstable-wiremsg-debuginfo"]);
    }

    info!("Building safenode");
    debug!("Building safenode with args: {:?}", args);
    let _ = Command::new("cargo")
        .args(args.clone())
        .current_dir("sn_node")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;
    info!("safenode built successfully");
    Ok(())
}

async fn run_network(
    node_bin_path: PathBuf,
    node_launch_interval: u64,
    node_count: u32,
    node_args: Vec<String>,
    flamegraph_mode: bool,
) -> Result<()> {
    let (mut testnet, network_contacts_path) = Testnet::configure()
        .node_bin_path(node_bin_path)
        .node_launch_interval(node_launch_interval)
        .clear_nodes_dir()
        .flamegraph_mode(flamegraph_mode)
        .build()?;
    testnet.launch_genesis(None, node_args.clone())?;
    testnet.launch_nodes(node_count as usize, &network_contacts_path, node_args)?;
    testnet.configure_network_contacts(&network_contacts_path)?;
    Ok(())
}

async fn join_network(
    node_bin_path: PathBuf,
    node_launch_interval: u64,
    node_count: u32,
    network_contacts_path: Option<PathBuf>,
    node_args: Vec<String>,
) -> Result<()> {
    let (mut testnet, default_network_contacts_path) = Testnet::configure()
        .node_bin_path(node_bin_path)
        .node_launch_interval(node_launch_interval)
        .build()?;
    let network_contacts_path = network_contacts_path.unwrap_or(default_network_contacts_path);
    testnet.launch_nodes(node_count as usize, &network_contacts_path, node_args)?;
    testnet.configure_network_contacts(&network_contacts_path)?;
    Ok(())
}

fn init_tracing() -> Result<()> {
    let mut filter = EnvFilter::try_new(BASE_TRACING_DIRECTIVES)
        .map_err(|_| eyre!("BUG: hard-coded tracing directives are invalid"))?;

    let extra_directives = std::env::var(EnvFilter::DEFAULT_ENV)
        .map_or_else(
            |error| match error {
                std::env::VarError::NotPresent => Ok(None),
                std::env::VarError::NotUnicode(_) => Err(eyre!(error)),
            },
            |filter| Ok(Some(EnvFilter::try_new(filter)?)),
        )
        .map_err(|_| eyre!("Invalid value for {}", EnvFilter::DEFAULT_ENV))?;

    if let Some(extra_directives) = extra_directives {
        for directive in extra_directives.to_string().split(',') {
            filter = filter.add_directive(
                directive
                    .parse()
                    .expect("BUG: invalid directive in parsed EnvFilter"),
            );
        }
    }

    tracing_subscriber::fmt().with_env_filter(filter).init();

    Ok(())
}
