// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use qp2p::Config as QuicP2pConfig;
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tracing::warn;

const DEFAULT_LOCAL_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::UNSPECIFIED, 0);

/// Default amount of time to wait for operations to succeed (query/cmd) before giving up and returning an error.
pub const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// Default amount of time to wait (to keep the client alive) after sending a cmd. This allows AE messages to be parsed/resent.
/// Larger PUT operations may need larger ae wait time
pub const DEFAULT_ACK_WAIT: Duration = Duration::from_secs(10);

const SN_QUERY_TIMEOUT: &str = "SN_QUERY_TIMEOUT";
const SN_CMD_TIMEOUT: &str = "SN_CMD_TIMEOUT";
const SN_AE_WAIT: &str = "SN_AE_WAIT";

/// Configuration for `sn_client`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientConfig {
    /// The local address to bind to.
    pub local_addr: SocketAddr,
    /// QuicP2p options.
    pub qp2p: QuicP2pConfig,
    /// The amount of time to wait for responses to queries before giving up and returning an error.
    pub query_timeout: Duration,
    /// The amount of time to wait for cmds to not error before giving up and returning an error.
    pub cmd_timeout: Duration,
    /// The amount of time to wait after a cmd is sent for AE flows to complete.
    pub cmd_ack_wait: Duration,
}

impl ClientConfig {
    /// Returns a new `Config` instance.
    ///
    /// This will try to read `QuicP2P` configuration from `config_file_path`, or else use the default
    /// `QuicP2P` config. In either case, `bootstrap_nodes` will be used to override the initial
    /// network contacts.
    ///
    /// If `local_addr` is not specified, `127.0.0.1:0` will be used (e.g. localhost with a random
    /// port).
    ///
    /// If `query_timeout` is not specified, [`DEFAULT_OPERATION_TIMEOUT`] will be used.
    pub async fn new(
        local_addr: Option<SocketAddr>,
        qp2p_config: Option<QuicP2pConfig>,
        query_timeout: Option<Duration>,
        cmd_timeout: Option<Duration>,
        cmd_ack_wait: Option<Duration>,
    ) -> Self {
        let qp2p = qp2p_config.unwrap_or_default();

        let query_timeout = query_timeout.unwrap_or(DEFAULT_OPERATION_TIMEOUT);
        let cmd_timeout = cmd_timeout.unwrap_or(DEFAULT_OPERATION_TIMEOUT);
        let cmd_ack_wait = cmd_ack_wait.unwrap_or(DEFAULT_ACK_WAIT);

        // if we have an env var for this, let's override
        let query_timeout = match std::env::var(SN_QUERY_TIMEOUT) {
            Ok(timeout) => match timeout.parse() {
                Ok(time) => {
                    warn!(
                        "Query timeout set from env var {}: {}s",
                        SN_QUERY_TIMEOUT, time
                    );
                    Duration::from_secs(time)
                }
                Err(error) => {
                    warn!("There was an error parsing {} env var value: '{}'. Default or client configured query timeout will be used: {:?}", SN_QUERY_TIMEOUT, timeout, error);
                    query_timeout
                }
            },
            Err(_) => query_timeout,
        };

        // if we have an env var for this, let's override
        let cmd_timeout = match std::env::var(SN_CMD_TIMEOUT) {
            Ok(timeout) => match timeout.parse() {
                Ok(time) => {
                    warn!(
                        "Query timeout set from env var {}: {}s",
                        SN_CMD_TIMEOUT, time
                    );
                    Duration::from_secs(time)
                }
                Err(error) => {
                    warn!("There was an error parsing {} env var value: '{}'. Default or client configured cmd timeout will be used: {:?}", SN_CMD_TIMEOUT, timeout, error);
                    cmd_timeout
                }
            },
            Err(_) => cmd_timeout,
        };

        // if we have an env var for this, let's override
        let cmd_ack_wait = match std::env::var(SN_AE_WAIT) {
            Ok(timeout) => match timeout.parse() {
                Ok(time) => {
                    warn!(
                        "Client AE wait post-put set from env var {}: {}s",
                        SN_AE_WAIT, time
                    );
                    Duration::from_secs(time)
                }
                Err(error) => {
                    warn!("There was an error parsing {} env var value: '{}'. Default or client configured query timeout will be used: {:?}", SN_AE_WAIT, timeout, error);
                    cmd_ack_wait
                }
            },
            Err(_) => cmd_ack_wait,
        };

        info!(
            "Client set to use a query timeout of {:?}, and AE await post-put for {:?}",
            query_timeout, cmd_ack_wait
        );
        Self {
            local_addr: local_addr.unwrap_or_else(|| SocketAddr::from(DEFAULT_LOCAL_ADDR)),
            qp2p,
            query_timeout,
            cmd_timeout,
            cmd_ack_wait,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::init_logger;
    use bincode::serialize;
    use eyre::Result;
    use rand::{distributions::Alphanumeric, thread_rng, Rng};
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::fs::create_dir_all;

    // 1. Verify that `Config::new()` generates the correct default config
    //    when the file is not found. The default config shall have the provided
    //    config path in the `boostrap_cache_dir` field.
    // 2. Write the default config file to temp directory.
    // 3. Assert that `Config::new()` reads the default config written to disk.
    // 4. Verify that `Config::new()` returns the correct default config when no path is provided.
    #[tokio::test(flavor = "multi_thread")]
    async fn custom_config_path() -> Result<()> {
        init_logger();

        let temp_dir = tempdir()?;
        let root_dir = temp_dir.path().to_path_buf();
        let cfg_filename: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(15)
            .map(char::from)
            .collect();
        let config_filepath = root_dir.join(&cfg_filename);

        // In the absence of a config file, the config handler
        // should initialize bootstrap_cache_dir only
        let config = ClientConfig::new(None, None, None, None, None).await;
        // convert to string for assert
        let mut str_path = root_dir
            .to_str()
            .ok_or(eyre::eyre!("No path for to_str".to_string()))?
            .to_string();
        // normalise for mac
        if str_path.ends_with('/') {
            let _some_last_char = str_path.pop();
        }

        let expected_query_timeout = std::env::var(SN_QUERY_TIMEOUT)
            .map(|v| {
                v.parse()
                    .map(Duration::from_secs)
                    .unwrap_or(DEFAULT_OPERATION_TIMEOUT)
            })
            .unwrap_or(DEFAULT_OPERATION_TIMEOUT);

        let expected_cmd_timeout = std::env::var(SN_CMD_TIMEOUT)
            .map(|v| {
                v.parse()
                    .map(Duration::from_secs)
                    .unwrap_or(DEFAULT_OPERATION_TIMEOUT)
            })
            .unwrap_or(DEFAULT_OPERATION_TIMEOUT);

        let expected_cmd_ack_wait = std::env::var(SN_AE_WAIT)
            .map(|v| {
                v.parse()
                    .map(Duration::from_secs)
                    .unwrap_or(DEFAULT_ACK_WAIT)
            })
            .unwrap_or(DEFAULT_ACK_WAIT);

        let expected_config = ClientConfig {
            local_addr: (Ipv4Addr::UNSPECIFIED, 0).into(),
            qp2p: QuicP2pConfig {
                ..Default::default()
            },
            query_timeout: expected_query_timeout,
            cmd_timeout: expected_cmd_timeout,
            cmd_ack_wait: expected_cmd_ack_wait,
        };
        assert_eq!(format!("{:?}", config), format!("{:?}", expected_config));
        assert_eq!(serialize(&config)?, serialize(&expected_config)?);

        create_dir_all(&root_dir).await?;
        let mut file = File::create(&config_filepath)?;

        let config_on_disk = ClientConfig::new(None, None, None, None, None).await;
        serde_json::to_writer_pretty(&mut file, &config_on_disk)?;
        file.sync_all()?;

        let read_cfg = ClientConfig::new(None, None, None, None, None).await;
        assert_eq!(serialize(&config_on_disk)?, serialize(&read_cfg)?);

        Ok(())
    }
}
