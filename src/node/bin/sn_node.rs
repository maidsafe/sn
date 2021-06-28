// Copyright 2021 MaidSafe.net limited.
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
    unused_results
)]

use log::{self, error, info};
use safe_network::node::{add_connection_info, set_connection_info, utils, Config, Error, Node};
use self_update::{cargo_crate_version, Status};
use std::{io::Write, process};
use structopt::{clap, StructOpt};
use tokio::time::{sleep, Duration};

const BOOTSTRAP_RETRY_TIME: u64 = 3; // in minutes
use safe_network::routing;

/// Runs a Safe Network node.
#[tokio::main]
async fn main() {
    run_node().await
}

async fn run_node() {
    let config = match Config::new() {
        Ok(cfg) => cfg,
        Err(e) => {
            println!("Failed to create Config: {:?}", e);
            return exit(1);
        }
    };

    if let Some(c) = &config.completions() {
        match c.parse::<clap::Shell>() {
            Ok(shell) => match gen_completions_for_shell(shell) {
                Ok(buf) => {
                    std::io::stdout().write_all(&buf).unwrap_or_else(|e| {
                        println!("Failed to print shell completions. {}", e);
                    });
                }
                Err(e) => println!("{}", e),
            },
            Err(e) => println!("Unknown completions option. {}", e),
        }
        // we exit program on both success and error.
        return;
    }

    //  Logger handle should be kept in scope
    let _logger = match utils::init_logging(&config) {
        Err(e) => {
            println!("Error setting up logging {:?}", e);
            return exit(1);
        }
        Ok(logger) => logger,
    };

    if config.update() || config.update_only() {
        match update() {
            Ok(status) => {
                if let Status::Updated { .. } = status {
                    println!("Node has been updated. Please restart.");
                    exit(0);
                }
            }
            Err(e) => error!("Updating node failed: {:?}", e),
        }

        if config.update_only() {
            exit(0);
        }
    }

    let message = format!(
        "Running {} v{}",
        Config::clap().get_name(),
        env!("CARGO_PKG_VERSION")
    );
    info!("\n\n{}\n{}", message, "=".repeat(message.len()));

    let log = format!(
        "The network is not accepting nodes right now. Retrying after {} minutes",
        BOOTSTRAP_RETRY_TIME
    );

    let (mut node, event_stream) = loop {
        match Node::new(&config).await {
            Ok(result) => break result,
            Err(Error::Routing(routing::Error::TryJoinLater)) => {
                println!("{}", log);
                info!("{}", log);
            }
            Err(Error::Routing(routing::Error::NodeNotReachable(addr))) => {
                let err_msg = format!(
                    "Unfortunately we are unable to establish a connection to your machine ({}) either through a \
                    public IP address, or via IGD on your router. Please ensure that IGD is enabled on your router - \
                    if it is and you are still unable to add your node to the testnet, then skip adding a node for this \
                    testnet iteration. You can still use the testnet as a client, uploading and downloading content, etc. \
                    https://safenetforum.org/",
                    addr
                );
                println!("{}", err_msg);
                error!("{}", err_msg);
                exit(1);
            }
            Err(Error::JoinTimeout) => {
                let message = format!("Encountered a timeout while trying to join the network. Retrying after {} minutes.", BOOTSTRAP_RETRY_TIME);
                println!("{}", &message);
                error!("{}", &message);
            }
            Err(e) => {
                println!("Cannot start node due to error: {:?}. If this is the first node on the network \
                 pass the local address to be used using --first. Exiting", e);
                error!("Cannot start node due to error: {:?}. If this is the first node on the network \
                 pass the local address to be used using --first. Exiting", e);
                exit(1);
            }
        }
        sleep(Duration::from_secs(BOOTSTRAP_RETRY_TIME * 60)).await;
    };

    let our_conn_info = node.our_connection_info();

    if config.is_first() {
        set_connection_info(our_conn_info).unwrap_or_else(|err| {
            error!("Unable to write our connection info to disk: {:?}", err);
        });
    } else {
        add_connection_info(our_conn_info).unwrap_or_else(|err| {
            error!("Unable to add our connection info to disk: {:?}", err);
        });
    }

    match node.run(event_stream).await {
        Ok(()) => exit(0),
        Err(e) => {
            println!("Cannot start node due to error: {:?}", e);
            error!("Cannot start node due to error: {:?}", e);
            exit(1);
        }
    }
}

fn exit(exit_code: i32) {
    log::logger().flush();
    process::exit(exit_code);
}

fn update() -> Result<Status, Box<dyn (::std::error::Error)>> {
    info!("Checking for updates...");
    let target = self_update::get_target();

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("maidsafe")
        .repo_name("safe_network")
        .with_target(&target)
        .build()?
        .fetch()?;

    if !releases.is_empty() {
        log::debug!("Target for update is {}", target);
        log::debug!("Found releases: {:#?}\n", releases);
        let bin_name = if target.contains("pc-windows") {
            "sn_node.exe"
        } else {
            "sn_node"
        };
        let status = self_update::backends::github::Update::configure()
            .repo_owner("maidsafe")
            .repo_name("safe_network")
            .target(&target)
            .bin_name(&bin_name)
            .show_download_progress(true)
            .no_confirm(true)
            .current_version(cargo_crate_version!())
            .build()?
            .update()?;
        println!("Update status: '{}'!", status.version());
        Ok(status)
    } else {
        println!("Current version is '{}'", cargo_crate_version!());
        println!("No releases are available for updates");
        Ok(Status::UpToDate(
            "No releases are available for updates".to_string(),
        ))
    }
}

fn gen_completions_for_shell(shell: clap::Shell) -> Result<Vec<u8>, String> {
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
    Config::clap().gen_completions_to(exec_name, shell, &mut buf);

    Ok(buf)
}
