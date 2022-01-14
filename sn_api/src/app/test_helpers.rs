// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use crate::{ipc::NodeConfig, Safe};
use anyhow::{bail, Context, Result};
use rand::{distributions::Alphanumeric, rngs::OsRng, thread_rng, Rng};
use safe_network::types::{Keypair, PublicKey};
use std::{collections::BTreeSet, env::var, fs, net::SocketAddr, sync::Once};
use tracing_subscriber::{fmt, EnvFilter};

// Environment variable which can be set with the auth credentials
// to be used for all sn_api tests
const TEST_AUTH_CREDENTIALS: &str = "TEST_AUTH_CREDENTIALS";

// Environment variable which can be set with the bootstrapping contacts
// to be used for all sn_api tests
const TEST_BOOTSTRAPPING_PEERS: &str = "TEST_BOOTSTRAPPING_PEERS";

// Default file in home directory where bootstrapping contacts are usually found
const DEFAULT_PEER_FILE_IN_HOME: &str = ".safe/node/node_connection_info.config";

static INIT: Once = Once::new();

// Initialise logger for tests, this is run only once, even if called multiple times.
fn init_logger() {
    INIT.call_once(|| {
        fmt()
            // NOTE: comment out this line for more compact (but less readable) log output.
            .pretty()
            .with_env_filter(EnvFilter::from_default_env())
            .with_target(false)
            .init()
    });
}

// Instantiate a Safe instance
pub async fn new_safe_instance() -> Result<Safe> {
    init_logger();
    let credentials = match var(TEST_AUTH_CREDENTIALS) {
        Ok(val) => serde_json::from_str(&val).with_context(|| {
            format!(
                "Failed to parse credentials read from {} env var",
                TEST_AUTH_CREDENTIALS
            )
        })?,
        Err(_) => {
            let mut rng = OsRng;
            Keypair::new_ed25519(&mut rng)
        }
    };

    let bootstrap_contacts = get_bootstrap_contacts()?;
    let safe = Safe::connect(bootstrap_contacts, Some(credentials), None, None, None).await?;

    Ok(safe)
}

// Create a random NRS name
pub fn random_nrs_name() -> String {
    thread_rng().sample_iter(&Alphanumeric).take(15).collect()
}

fn read_default_peers_from_file() -> Result<(String, BTreeSet<SocketAddr>)> {
    let default_peer_file = match dirs_next::home_dir() {
        None => bail!(
            "Failed to obtain local home directory where to read {} from",
            DEFAULT_PEER_FILE_IN_HOME
        ),
        Some(mut paths) => {
            paths.push(DEFAULT_PEER_FILE_IN_HOME);
            paths
        }
    };

    let raw_json = fs::read_to_string(&default_peer_file).with_context(|| {
        format!(
            "Failed to read bootstraping contacts list from file: {:?}",
            &default_peer_file
        )
    })?;

    let info: (String, BTreeSet<SocketAddr>) =
        serde_json::from_str(&raw_json).with_context(|| {
            format!(
                "Failed to parse bootstraping contacts list from file: {:?}",
                &default_peer_file
            )
        })?;

    Ok(info)
}

fn get_bootstrap_contacts() -> Result<NodeConfig> {
    let (genesis_key_hex, bootstrap_contacts) = match var(TEST_BOOTSTRAPPING_PEERS) {
        Ok(val) => serde_json::from_str(&val).with_context(|| {
            format!(
                "Failed to parse bootstraping contacts list from {} env var",
                TEST_BOOTSTRAPPING_PEERS
            )
        })?,
        Err(_) => {
            // read default peers from the file we normally use for peers
            read_default_peers_from_file()?
        }
    };

    let genesis_key = PublicKey::bls_from_hex(&genesis_key_hex)?
        .bls()
        .ok_or_else(|| {
            anyhow::anyhow!("Unexpectedly failed to obtain network's (BLS) genesis key.")
        })?;

    Ok((genesis_key, bootstrap_contacts))
}

#[macro_export]
macro_rules! retry_loop {
    ($n:literal, $async_func:expr) => {{
        let mut retries: u64 = $n;
        loop {
            match $async_func.await {
                Ok(val) => break val,
                Err(_) if retries > 0 => {
                    retries -= 1;
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                }
                Err(e) => anyhow::bail!("Failed after {} retries: {:?}", $n, e),
            }
        }
    }};
    // Defaults to 10 retries if n is not provided
    ($async_func:expr) => {{
        retry_loop!(10, $async_func)
    }};
}

#[macro_export]
macro_rules! retry_loop_for_pattern {
    ($n:literal, $async_func:expr, $pattern:pat $(if $cond:expr)?) => {{
        let mut retries: u64 = $n;
        loop {
            let result = $async_func.await;
            match &result {
                $pattern $(if $cond)? => break result,
                Ok(_) | Err(_) if retries > 0 => {
                    retries -= 1;
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                },
                Err(e) => anyhow::bail!("Failed after {} retries: {:?}", $n, e),
                Ok(_) => anyhow::bail!("Failed to match pattern after {} retries", $n),
            }
        }
    }};
    // Defaults to 10 retries if n is not provided
    ($async_func:expr, $pattern:pat $(if $cond:expr)?) => {{
        retry_loop_for_pattern!(10, $async_func, $pattern $(if $cond)?)
    }};
}
