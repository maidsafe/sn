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

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use color_eyre::{Section, SectionExt};
use eyre::{eyre, Context, ErrReport, Result};
use self_update::{cargo_crate_version, Status};
use sn_interface::network_knowledge::SectionTree;
use sn_node::comm::{Comm, Inbox, Outbox, OutgoingMsg};
use sn_node::node::{start_node, Config, Error as NodeError, Event, MembershipEvent};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::{io::Write, process::exit};
use tokio::{
    sync::{mpsc, RwLock},
    time::{sleep, Duration},
};
use tracing::{self, error, info, trace, warn};

const JOIN_TIMEOUT_SEC: u64 = 100;
const BOOTSTRAP_RETRY_TIME_SEC: u64 = 5;

mod log;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Create a new runtime for a node
    // let rt = tokio::runtime::new();
    // .enable_all()
    // .thread_name("sn_node")
    // // 16mb here for windows stack size, which was being exceeded previously
    // .thread_stack_size(16 * 1024 * 1024)
    // .build()?;

    // We need this to pass into join flow
    // TODO: refactor this to be created by comms and just return Cmds
    // This will need join flow to parse cmds...
    let (sender_for_msgs_received_in_comms, msg_receiver_channel) = mpsc::channel(100);

    // rt.block_on(async {
    let mut config = Config::new().await?;

    let _guard = log::init_node_logging(&config)?;
    trace!("Initial node config: {config:?}");

    let local_addr = config
        .local_addr
        .unwrap_or_else(|| SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)));

    let (mut comm, intitial_contact_address) = if config.is_first() {
        let comm = Comm::first_node(
            local_addr,
            config.network_config().clone(),
            sender_for_msgs_received_in_comms,
        )
        .await?;

        (comm, None)
    } else {
        // get initial contacts
        let path = config.network_contacts_file().ok_or_else(|| {
            NodeError::Configuration("Could not obtain network contacts file path".to_string())
        })?;
        let network_contacts = SectionTree::from_disk(&path).await?;
        let section_elders = {
            let sap = network_contacts
                .closest(&xor_name::rand::random(), None)
                .ok_or_else(|| {
                    NodeError::Configuration("Could not obtain closest SAP".to_string())
                })?;
            sap.elders_vec()
        };
        let bootstrap_nodes: Vec<SocketAddr> =
            section_elders.iter().map(|node| node.addr()).collect();

        let (comm, intitial_contact_address) = Comm::make_initial_contact(
            local_addr,
            bootstrap_nodes.as_slice(),
            config.network_config().clone(),
            sender_for_msgs_received_in_comms,
        )
        .await?;

        (comm, Some(intitial_contact_address))
    };

    let send_msg_channel = comm.send_msg_channel();
    let addr = comm.socket_addr();

    // SO here we have comm...
    // we need that in a thread with its own event loop
    // and we spawn a freash thread just for that
    // with a sender available outside...
    let _handle = tokio::spawn(async move {
        info!("hey1");
        let r = comm.run_comm_loop().await;
        info!("hey2: {r:?}");
    });

    info!("Node runtime started");
    create_runtime_and_node(
        &config,
        send_msg_channel.clone(),
        msg_receiver_channel,
        addr,
        intitial_contact_address,
    )
    .await?;

    Ok(())
}

/// Create a tokio runtime per `run_node` instance.
async fn create_runtime_and_node(
    config: &Config,
    outbox: Outbox,
    inbox: Inbox,
    addr: SocketAddr,
    intitial_contact_address: Option<SocketAddr>,
) -> Result<()> {
    let local = tokio::task::LocalSet::new();

    local
        .run_until(async move {
            // loops ready to catch any ChurnJoinMiss
            match run_node(config, outbox, inbox, addr, intitial_contact_address).await {
                Ok(_) => {
                    info!("Node has finished running, no runtime errors were reported");
                }
                Err(error) => {
                    warn!("Node instance finished with an error: {error:?}");
                }
            };
        })
        .await;

    Ok(())
}

async fn run_node(
    config: &Config,
    outbox: Outbox,
    inbox: Inbox,
    addr: SocketAddr,
    intitial_contact_address: Option<SocketAddr>,
) -> Result<()> {
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
    info!("\n{}\n{}", message, "=".repeat(message.len()));

    let our_pid = std::process::id();

    let log = format!("The network is not accepting nodes right now. Retrying after {BOOTSTRAP_RETRY_TIME_SEC} seconds");

    let join_timeout = Duration::from_secs(JOIN_TIMEOUT_SEC);
    let bootstrap_retry_duration = Duration::from_secs(BOOTSTRAP_RETRY_TIME_SEC);

    let inbox = Arc::new(RwLock::new(inbox));
    let (_node, mut event_stream) = loop {
        match start_node(
            config,
            join_timeout,
            outbox.clone(),
            inbox.clone(),
            addr,
            intitial_contact_address,
        )
        .await
        {
            Ok(result) => break result,
            Err(NodeError::CannotConnectEndpoint(qp2p::EndpointError::Upnp(error))) => {
                return Err(error).suggestion(
                    "You can disable port forwarding by supplying --skip-auto-port-forwarding. Without port\n\
                    forwarding, your machine must be publicly reachable by the given\n\
                    --public-addr. If your machine is not publicly reachable, you may have to\n\
                    adjust your router settings to either:\n\
                    \n\
                    - Resolve the error (e.g. by enabling UPnP).\n\
                    - Manually configure port forwarding, such that your machine is publicly \
                      reachable, and supplying that address with --public-addr."
                        .header("Disable port forwarding or change your router settings"),
                );
            }
            Err(NodeError::TryJoinLater) => {
                println!("{}", log);
                info!("{}", log);
                // return Err(NodeError::TryJoinLater.into())
            }
            Err(NodeError::NodeNotReachable(addr)) => {
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
            Err(NodeError::JoinTimeout) => {
                let message = format!("(PID: {our_pid}): Encountered a timeout while trying to join the network. Retrying after {BOOTSTRAP_RETRY_TIME_SEC} seconds.");
                println!("{}", &message);
                error!("{}", &message);
                // return Err(NodeError::JoinTimeout.into())
            }
            Err(e) => {
                let log_path = if let Some(path) = config.log_dir() {
                    format!("{}", path.display())
                } else {
                    "unknown".to_string()
                };

                error!("{}", &message);

                return Err(e).wrap_err(format!(
                    "Cannot start node (log path: {}). If this is the first node on the network pass the local \
                    address to be used using --first", log_path)
                );
            }
        }
        sleep(bootstrap_retry_duration).await;
    };
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
            sleep(Duration::from_secs(1)).await;
            warn!("[Chaos] (PID: {our_pid}): ChaoticStartupCrash");
            return Err(NodeError::ChaoticStartupCrash).map_err(ErrReport::msg);
        }
    }

    // this keeps node running
    while let Some(event) = event_stream.next().await {
        trace!("Node event! {}", event);
        if let Event::Membership(MembershipEvent::RemovedFromSection) = event {
            return Err(NodeError::RemovedFromSection).map_err(ErrReport::msg);
        }
    }

    Ok(())
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
