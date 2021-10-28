// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

#[cfg(feature = "self-update")]
use super::helpers::download_and_install_github_release_asset;
use color_eyre::{eyre::bail, eyre::eyre, eyre::WrapErr, Result};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::{
    collections::{BTreeSet, HashMap},
    fs::create_dir_all,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
use structopt::StructOpt;
use tracing::debug;

#[cfg(not(target_os = "windows"))]
const SN_NODE_EXECUTABLE: &str = "sn_node";

#[cfg(target_os = "windows")]
const SN_NODE_EXECUTABLE: &str = "sn_node.exe";

fn run_safe_cmd(
    args: &[&str],
    envs: Option<HashMap<String, String>>,
    ignore_errors: bool,
    verbosity: u8,
) -> Result<()> {
    let env: HashMap<String, String> = envs.unwrap_or_default();

    let msg = format!("Running 'safe' with args {:?} ...", args);
    if verbosity > 1 {
        println!("{}", msg);
    }
    debug!("{}", msg);

    let _child = Command::new("safe")
        .args(args)
        .envs(&env)
        .stdout(Stdio::inherit())
        .stderr(if ignore_errors {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .wrap_err_with(|| format!("Failed to run 'safe' with args '{:?}'", args))?;

    Ok(())
}

/// Tries to print the version of the node binary pointed to
pub fn node_version(node_path: Option<PathBuf>) -> Result<()> {
    let bin_path = get_node_bin_path(node_path)?.join(SN_NODE_EXECUTABLE);
    let path_str = bin_path.display().to_string();

    if !bin_path.as_path().is_file() {
        return Err(eyre!(format!(
            "node executable not found at '{}'.",
            path_str
        )));
    }

    let output = Command::new(&path_str)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| {
            eyre!(format!(
                "Failed to execute node from '{}': {}",
                path_str, err
            ))
        })?;

    if output.status.success() {
        io::stdout()
            .write_all(&output.stdout)
            .map_err(|err| eyre!(format!("failed to write to stdout: {}", err)))
    } else {
        Err(eyre!(
            "Failed to get node version nodes when invoking executable from '{}': {}",
            path_str,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(not(feature = "self-update"))]
pub fn node_install(_vault_path: Option<PathBuf>, version: Option<String>) -> Result<()> {
    eyre!("Self updates are disabled")
}

#[cfg(feature = "self-update")]
pub fn node_install(node_path: Option<PathBuf>, version: Option<String>) -> Result<()> {
    let target_path = get_node_bin_path(node_path)?;
    let _ = download_and_install_github_release_asset(
        target_path,
        SN_NODE_EXECUTABLE,
        "safe_network",
        version,
    )?;
    Ok(())
}

pub fn node_run(
    node_path: Option<PathBuf>,
    nodes_dir: &str,
    verbosity: u8,
    interval: &str,
    num_of_nodes: &str,
    ip: Option<String>,
    test: bool,
) -> Result<()> {
    let node_path = get_node_bin_path(node_path)?;

    let arg_node_path = node_path.join(SN_NODE_EXECUTABLE).display().to_string();
    debug!("Running node from {}", arg_node_path);

    let nodes_dir = node_path.join(nodes_dir);
    if !nodes_dir.exists() {
        println!("Creating '{}' folder", nodes_dir.display());
        create_dir_all(nodes_dir.clone())
            .wrap_err("Couldn't create target path to store nodes' generated data")?;
    }
    let arg_nodes_dir = nodes_dir.display().to_string();
    println!("Storing nodes' generated data at {}", arg_nodes_dir);

    // Let's create an args array to pass to the network launcher tool
    let mut sn_launch_tool_args = vec![
        "sn_launch_tool",
        "--node-path",
        &arg_node_path,
        "--nodes-dir",
        &arg_nodes_dir,
        "--interval",
        interval,
        "--num-nodes",
        num_of_nodes,
    ];

    let interval_as_int = &interval.parse::<u64>().unwrap();

    if let Some(ref launch_ip) = ip {
        sn_launch_tool_args.push("--ip");
        sn_launch_tool_args.push(launch_ip);
    } else {
        sn_launch_tool_args.push("--local");
    }

    debug!(
        "Running network launch tool with args: {:?}",
        sn_launch_tool_args
    );

    // We can now call the tool with the args
    println!("Launching local Safe network...");
    sn_launch_tool::Launch::from_iter_safe(&sn_launch_tool_args)
        .map_err(|e| eyre!(e))
        .and_then(|launch| launch.run())
        .wrap_err("Error launching node")?;

    let interval_duration = Duration::from_secs(interval_as_int * 15);
    thread::sleep(interval_duration);

    let ignore_errors = true;
    let report_errors = false;

    if test {
        println!("Setting up authenticator against local Safe network...");

        // stop authd
        let stop_auth_args = vec!["auth", "stop"];
        run_safe_cmd(&stop_auth_args, None, ignore_errors, verbosity)?;

        let between_command_interval = Duration::from_secs(interval_as_int * 5);
        thread::sleep(between_command_interval);

        // start authd
        let start_auth_args = vec!["auth", "start"];
        run_safe_cmd(&start_auth_args, None, report_errors, verbosity)?;

        thread::sleep(between_command_interval);

        // Q: can we assume network is correct here? Or do we need to do networks switch?
        let passphrase: String = thread_rng().sample_iter(&Alphanumeric).take(15).collect();
        let password: String = thread_rng().sample_iter(&Alphanumeric).take(15).collect();

        // setup env for create / unlock
        let mut env = HashMap::new();
        env.insert("SAFE_AUTH_PASSPHRASE".to_string(), passphrase);
        env.insert("SAFE_AUTH_PASSWORD".to_string(), password);

        // create a Safe
        let create = vec!["auth", "create", "--test-coins"];

        run_safe_cmd(&create, Some(env.clone()), report_errors, verbosity)?;
        thread::sleep(between_command_interval);

        // unlock the Safe
        let unlock = vec!["auth", "unlock", "--self-auth"];
        run_safe_cmd(&unlock, Some(env), report_errors, verbosity)?;
    }

    Ok(())
}

pub fn node_join(
    node_path: Option<PathBuf>,
    node_data_dir: &str,
    verbosity: u8,
    contacts: &BTreeSet<SocketAddr>,
    local_addr: Option<SocketAddr>,
    public_addr: Option<SocketAddr>,
    clear_data: bool,
) -> Result<()> {
    let node_path = get_node_bin_path(node_path)?;

    let arg_node_path = node_path.join(SN_NODE_EXECUTABLE).display().to_string();
    debug!("Running node from {}", arg_node_path);

    let node_data_dir = node_path.join(node_data_dir);
    if !node_data_dir.exists() {
        println!("Creating '{}' folder", node_data_dir.display());
        create_dir_all(node_data_dir.clone())
            .wrap_err("Couldn't create target path to store nodes' generated data")?;
    }
    let arg_nodes_dir = node_data_dir.display().to_string();
    println!("Storing nodes' generated data at {}", arg_nodes_dir);

    // Let's create an args array to pass to the network launcher tool
    let mut sn_launch_tool_args = vec![
        "sn_launch_tool-join",
        "-v",
        "--node-path",
        &arg_node_path,
        "--nodes-dir",
        &arg_nodes_dir,
    ];

    let local_addr_str = local_addr.map(|addr| addr.to_string());
    let public_addr_str = public_addr.map(|addr| addr.to_string());

    if let Some(ref local) = local_addr_str {
        sn_launch_tool_args.push("--local-addr");
        sn_launch_tool_args.push(local);
    }

    if let Some(ref public) = public_addr_str {
        sn_launch_tool_args.push("--public-addr");
        sn_launch_tool_args.push(public);
    }

    if clear_data {
        sn_launch_tool_args.push("--clear-data");
    }

    let mut verbosity_arg = String::from("-");
    if verbosity > 0 {
        let v = "y".repeat(verbosity as usize);
        println!("V: {}", v);
        verbosity_arg.push_str(&v);
        sn_launch_tool_args.push(&verbosity_arg);
    }

    sn_launch_tool_args.push("--hard-coded-contacts");
    let contacts_list = contacts
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<String>>();

    for peer in &contacts_list {
        sn_launch_tool_args.push(peer);
    }

    debug!(
        "Running network launch tool with args: {:?}",
        sn_launch_tool_args
    );

    // We can now call the tool with the args
    println!("Starting a node to join a Safe network...");
    sn_launch_tool::Join::from_iter_safe(&sn_launch_tool_args)
        .map_err(|e| eyre!(e))
        .and_then(|launch| launch.run())
        .wrap_err("Error launching node")?;
    Ok(())
}

pub fn node_shutdown(node_path: Option<PathBuf>) -> Result<()> {
    let node_exec_name = match node_path {
        Some(ref path) => {
            let filepath = path.as_path();
            if filepath.is_file() {
                match filepath.file_name() {
                    Some(filename) => match filename.to_str() {
                        Some(name) => name,
                        None => bail!("Node path provided ({}) contains invalid unicode chars", filepath.display()),
                    }
                    None => bail!("Node path provided ({}) is invalid as it doens't include the executable filename", filepath.display()),
                }
            } else {
                bail!("Node path provided ({}) is invalid as it doens't include the executable filename", filepath.display())
            }
        }
        None => SN_NODE_EXECUTABLE,
    };

    debug!(
        "Killing all running nodes launched with {}...",
        node_exec_name
    );
    kill_nodes(node_exec_name)
}

fn get_node_bin_path(node_path: Option<PathBuf>) -> Result<PathBuf> {
    match node_path {
        Some(p) => Ok(p),
        None => {
            let mut path =
                dirs_next::home_dir().ok_or_else(|| eyre!("Failed to obtain user's home path"))?;

            path.push(".safe");
            path.push("node");
            Ok(path)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn kill_nodes(exec_name: &str) -> Result<()> {
    let output = Command::new("killall")
        .arg(exec_name)
        .output()
        .wrap_err_with(|| {
            format!(
                "Error when atempting to stop nodes ({}) processes",
                exec_name
            )
        })?;

    if output.status.success() {
        println!(
            "Success, all processes instances of {} were stopped!",
            exec_name
        );
        Ok(())
    } else {
        Err(eyre!(
            "Failed to stop nodes ({}) processes: {}",
            exec_name,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(target_os = "windows")]
fn kill_nodes(exec_name: &str) -> Result<()> {
    let output = Command::new("taskkill")
        .args(&["/F", "/IM", exec_name])
        .output()
        .wrap_err_with(|| {
            format!(
                "Error when atempting to stop nodes ({}) processes",
                exec_name
            )
        })?;

    if output.status.success() {
        println!(
            "Success, all processes instances of {} were stopped!",
            exec_name
        );
        Ok(())
    } else {
        Err(eyre!(
            "Failed to stop nodes ({}) processes: {}",
            exec_name,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn node_update(node_path: Option<PathBuf>) -> Result<()> {
    let node_path = get_node_bin_path(node_path)?;

    let arg_node_path = node_path.join(SN_NODE_EXECUTABLE).display().to_string();
    debug!("Updating node at {}", arg_node_path);

    let child = Command::new(&arg_node_path)
        .args(vec!["--update-only"])
        .spawn()
        .wrap_err_with(|| format!("Failed to update node at '{}'", arg_node_path))?;

    let output = child
        .wait_with_output()
        .wrap_err_with(|| format!("Failed to update node at '{}'", arg_node_path))?;

    if output.status.success() {
        io::stdout()
            .write_all(&output.stdout)
            .wrap_err("Failed to output stdout")?;
        Ok(())
    } else {
        Err(eyre!(
            "Failed when invoking node executable from '{}':\n{}",
            arg_node_path,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
