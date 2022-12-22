// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! sn_node provides the interface to Safe routing.  The resulting executable is the node
//! for the Safe network.
// boop
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
    unused_results,
    clippy::unwrap_used
)]

use sn_node::node::{start_node, Config, Error as NodeError};

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use color_eyre::{Section, SectionExt};
use eyre::{eyre, ErrReport, Result};
use self_update::{cargo_crate_version, Status};
use std::{io::Write, process::exit};
use tokio::runtime::Runtime;
use tokio::time::Duration;
use tracing::{self, error, info, trace, warn};

const JOIN_TIMEOUT_SEC: u64 = 100;
const BOOTSTRAP_RETRY_TIME_SEC: u64 = 30;

mod log;

fn main() -> Result<()> {
    color_eyre::install()?;

    let mut config = futures::executor::block_on(Config::new())?;
    config.network_config.max_concurrent_bidi_streams = Some(500);

    #[cfg(not(feature = "otlp"))]
    let _log_guard = log::init_node_logging(&config)?;
    #[cfg(feature = "otlp")]
    let (_rt, _guard) = {
        // init logging in a separate runtime if we are sending traces to an opentelemetry server
        let rt = Runtime::new()?;
        let guard = rt.block_on(async { log::init_node_logging(&config) })?;
        (rt, guard)
    };

    loop {
        println!("Node started");
        create_runtime_and_node(&config)?;

        // if we've had an issue, lets put the brakes on any crazy looping here
        std::thread::sleep(Duration::from_secs(1));

        // pull config again in case it has been updated meanwhile
        config = futures::executor::block_on(Config::new())?;
    }
}

/// Create a tokio runtime per `start_node` attempt.
/// This ensures any spawned tasks are closed before this would
/// be run again.
fn create_runtime_and_node(config: &Config) -> Result<()> {
    if let Some(c) = &config.completions() {
        let shell = c.parse().map_err(|err: String| eyre!(err))?;
        let buf = gen_completions_for_shell(shell, Config::command()).map_err(|err| eyre!(err))?;
        std::io::stdout().write_all(&buf)?;

        return Ok(());
    }

    if config.update() || config.update_only() {
        match update() {
            Ok(status) => {
                if let Status::Updated { .. } = status {
                    println!("Node has been updated. Please restart.");
                    exit(0);
                }
            }
            Err(e) => println!("Updating node failed: {:?}", e),
        }

        if config.update_only() {
            exit(0);
        }
    }

    let our_pid = std::process::id();
    let join_timeout = Duration::from_secs(JOIN_TIMEOUT_SEC);
    let bootstrap_retry_duration = Duration::from_secs(BOOTSTRAP_RETRY_TIME_SEC);
    let log_path = if let Some(path) = config.log_dir() {
        format!("{}", path.display())
    } else {
        "unknown".to_string()
    };

    loop {
        // make a fresh runtime
        let rt = Runtime::new()?;

        let message = format!(
            "Running {} v{}",
            Config::clap().get_name(),
            env!("CARGO_PKG_VERSION")
        );

        info!("\n{}\n{}", message, "=".repeat(message.len()));

        let outcome = rt.block_on(async {
            trace!("Initial node config: {config:?}");
            Ok::<_, ErrReport>(start_node(config, join_timeout).await)
        })?;

        match outcome {
            Ok((_node, mut rejoin_network_rx)) => {
                rt.block_on(async {
                    // Simulate failed node starts, and ensure that
                   #[cfg(feature = "chaos")]
                   {
                       use rand::Rng;
                       let mut rng = rand::thread_rng();
                       let x: f64 = rng.gen_range(0.0..1.0);

                       if !config.is_first() && x > 0.6 {
                           println!(
                               "\n =========== [Chaos] (PID: {our_pid}): Startup chaos crash w/ x of: {}. ============== \n",
                               x
                           );

                           // tiny sleep so testnet doesn't detect a fauly node and exit
                           tokio::time::sleep(Duration::from_secs(1)).await;
                           warn!("[Chaos] (PID: {our_pid}): ChaoticStartupCrash");
                           return Err(NodeError::ChaoticStartupCrash).map_err(ErrReport::msg);
                       }
                   }

                   // this keeps node running
                   if rejoin_network_rx.recv().await.is_some() {
                       return Err(NodeError::RemovedFromSection).map_err(ErrReport::msg);
                   }
                   Ok(())
                })?;
            }
            Err(NodeError::TryJoinLater) => {
                let message = format!(
                    "The network is not accepting nodes right now. \
                    Retrying after {BOOTSTRAP_RETRY_TIME_SEC} seconds."
                );
                println!("{message} Node log path: {log_path}");
                info!("{message}");
            }
            Err(error @ NodeError::NodeNotReachable(_)) => {
                let err = Err(error).suggestion(
                    "Unfortunately we are unable to establish a connection to your machine through its \
                    public IP address. This might involve forwarding ports on your router."
                        .header("Please ensure your node is externally reachable")
                );

                println!("{err:?}");
                error!("{err:?}");
                return err;
            }
            Err(NodeError::JoinTimeout) => {
                let message = format!("(PID: {our_pid}): Encountered a timeout while trying to join the network. Retrying after {BOOTSTRAP_RETRY_TIME_SEC} seconds.");
                println!("{message} Node log path: {log_path}");
                error!("{message}");
            }
            Err(error) => {
                let err = Err(error)
                    .suggestion(format!("Cannot start node. Node log path: {log_path}").header(
                    "If this is the first node on the network pass the local address to be used using --first",
                ));
                error!("{err:?}");
                return err;
            }
        }

        // actively shut down the runtime
        rt.shutdown_timeout(Duration::from_secs(2));
        // The sleep shall only need to be carried out when being asked to join later?
        // For the case of a timed_out, a retry can be carried out immediately?
        std::thread::sleep(bootstrap_retry_duration);
    }
}

fn update() -> Result<Status, Box<dyn (::std::error::Error)>> {
    info!("Checking for updates...");
    let target = self_update::get_target();

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("maidsafe")
        .repo_name("safe_network")
        .with_target(target)
        .build()?
        .fetch()?;

    if releases.is_empty() {
        println!("Current version is '{}'", cargo_crate_version!());
        println!("No releases are available for updates");
        return Ok(Status::UpToDate(
            "No releases are available for updates".to_string(),
        ));
    }

    tracing::debug!("Target for update is {}", target);
    tracing::debug!("Found releases: {:#?}\n", releases);
    let bin_name = if target.contains("pc-windows") {
        "sn_node.exe"
    } else {
        "sn_node"
    };
    let status = self_update::backends::github::Update::configure()
        .repo_owner("maidsafe")
        .repo_name("safe_network")
        .target(target)
        .bin_name(bin_name)
        .show_download_progress(true)
        .no_confirm(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: '{}'!", status.version());
    Ok(status)
}

fn gen_completions_for_shell(shell: Shell, mut cmd: clap::Command) -> Result<Vec<u8>, String> {
    // Get exe path
    let exe_path =
        std::env::current_exe().map_err(|err| format!("Can't get the exec path: {}", err))?;

    // get filename without preceding path as std::ffi::OsStr (C string)
    let exec_name_ffi = match exe_path.file_name() {
        Some(v) => v,
        None => {
            return Err(format!(
                "Can't extract file_name of executable from path {}",
                exe_path.display()
            ))
        }
    };

    // Convert OsStr to string.  Can fail if OsStr contains any invalid unicode.
    let exec_name = match exec_name_ffi.to_str() {
        Some(v) => v.to_string(),
        None => {
            return Err(format!(
                "Can't decode unicode in executable name '{:?}'",
                exec_name_ffi
            ))
        }
    };

    // Generates shell completions for <shell> and prints to stdout
    let mut buf: Vec<u8> = vec![];
    generate(shell, &mut cmd, exec_name, &mut buf);

    Ok(buf)
}
