//! A builder to instantiate a [`Client`]
//!
//! # Example
//!
//! ```no_run
//! # #[tokio::main]
//! # async fn main() -> Result<(), sn_client::Error> {
//! use sn_client::api::Client;
//! use xor_name::XorName;
//!
//! let client = Client::builder().build().await?;
//! let _bytes = client.read_bytes(XorName::from_content("example".as_bytes())).await?;
//!
//! # Ok(())
//! # }
//! ```
use crate::{connections::Session, Client, Error, DEFAULT_NETWORK_CONTACTS_FILE_NAME};

use qp2p::Config as Qp2pConfig;
use sn_dbc::Owner;
use sn_interface::{network_knowledge::SectionTree, types::Keypair};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;

/// Environment variable used to convert into [`ClientBuilder::query_timeout`] (seconds)
pub const ENV_QUERY_TIMEOUT: &str = "SN_QUERY_TIMEOUT";
/// Environment variable used to convert into [`ClientBuilder::query_timeout`] (seconds)
pub const ENV_MAX_RETRIES: &str = "SN_MAX_RETRIES";
/// Environment variable used to convert into [`ClientBuilder::cmd_timeout`] (seconds)
pub const ENV_CMD_TIMEOUT: &str = "SN_CMD_TIMEOUT";
/// Environment variable used to convert into [`ClientBuilder::cmd_ack_wait`] (seconds)
pub const ENV_AE_WAIT: &str = "SN_AE_WAIT";

/// Bind by default to all network interfaces on a OS assigned port
pub const DEFAULT_LOCAL_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::UNSPECIFIED, 0);
/// Default timeout to use before timing out queries and commands
pub const DEFAULT_QUERY_CMD_TIMEOUT: Duration = Duration::from_secs(120);
/// Max retries to be attempted in the DEFAULT_QUERY_CMD_TIMEOUT; DEFAULT_QUERY_CMD_TIMEOUT / DEFAULT_MAX_QUERY_CMD_RETRIES ~ second per try
/// (though exponential backoff exists)
pub const DEFAULT_MAX_QUERY_CMD_RETRIES: usize = 20;

/// Build a [`crate::Client`]
#[derive(Debug, Default)]
pub struct ClientBuilder {
    keypair: Option<Keypair>,
    dbc_owner: Option<Owner>,
    local_addr: Option<SocketAddr>,
    qp2p: Option<Qp2pConfig>,
    query_timeout: Option<Duration>,
    max_retries: Option<usize>,
    cmd_timeout: Option<Duration>,
    cmd_ack_wait: Option<Duration>,
    network_contacts: Option<SectionTree>,
}

impl ClientBuilder {
    /// Instantiate a builder with default parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the keypair associated with the queries sent from this client.
    pub fn keypair(mut self, kp: impl Into<Option<Keypair>>) -> Self {
        self.keypair = kp.into();
        self
    }

    /// Set the DBC owner associated with this client.
    pub fn dbc_owner(mut self, owner: impl Into<Option<Owner>>) -> Self {
        self.dbc_owner = owner.into();
        self
    }

    /// Local address to bind client endpoint to
    pub fn local_addr(mut self, addr: impl Into<Option<SocketAddr>>) -> Self {
        self.local_addr = addr.into();
        self
    }

    /// QuicP2p options
    pub fn qp2p(mut self, cfg: impl Into<Option<Qp2pConfig>>) -> Self {
        self.qp2p = cfg.into();
        self
    }

    /// Time to wait for responses to queries before giving up and returning an error
    pub fn query_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.query_timeout = timeout.into();
        self
    }

    /// Max retries within `query_timeout` for any one operation
    pub fn max_retries(mut self, max_retries: impl Into<Option<usize>>) -> Self {
        self.max_retries = max_retries.into();
        self
    }

    /// Time to wait for cmds to not error before giving up and returning an error
    pub fn cmd_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.cmd_timeout = timeout.into();
        self
    }

    /// Time to wait after a cmd is sent for AE flows to complete
    pub fn cmd_ack_wait(mut self, time: impl Into<Option<Duration>>) -> Self {
        self.cmd_ack_wait = time.into();
        self
    }

    /// SectionTree used to bootstrap the client on the network
    pub fn network_contacts(mut self, pm: impl Into<Option<SectionTree>>) -> Self {
        self.network_contacts = pm.into();
        self
    }

    /// Read options from environment variables:
    /// - [`Self::query_timeout()`] from [`ENV_QUERY_TIMEOUT`]
    /// - [`Self::max_retries()`] from [`ENV_MAX_RETRIES`]
    /// - [`Self::cmd_timeout()`] from [`ENV_CMD_TIMEOUT`]
    /// - [`Self::cmd_ack_wait()`] from [`ENV_AE_WAIT`]
    pub fn from_env(mut self) -> Self {
        if let Ok(Some(v)) = env_parse(ENV_QUERY_TIMEOUT) {
            self.query_timeout = Some(Duration::from_secs(v));
        }
        if let Ok(Some(v)) = env_parse(ENV_MAX_RETRIES) {
            self.max_retries = Some(v);
        }
        if let Ok(Some(v)) = env_parse(ENV_CMD_TIMEOUT) {
            self.cmd_timeout = Some(Duration::from_secs(v));
        }
        if let Ok(Some(v)) = env_parse(ENV_AE_WAIT) {
            self.cmd_ack_wait = Some(Duration::from_secs(v));
        }

        self
    }

    /// Instantiate the [`Client`] using the parameters passed to this builder.
    ///
    /// In case parameters have not been passed to this builder, defaults will be used:
    /// - `[Self::keypair]` and `[Self::dbc_owner]` are randomly generated
    /// - `[Self::query_timeout`] and `[Self::cmd_timeout]` default to [`DEFAULT_QUERY_CMD_TIMEOUT`]
    /// - `[Self::max_retries`] and `[Self::cmd_timeout]` default to [`DEFAULT_MAX_QUERY_CMD_RETRIES`]
    /// - `[Self::cmd_ack_wait`] defaults to [`DEFAULT_ACK_WAIT`]
    /// - [`qp2p::Config`] will default to it's [`Default`] impl
    /// - Network contacts file will be read from a standard location
    pub async fn build(self) -> Result<Client, Error> {
        let max_retries = self.max_retries.unwrap_or(DEFAULT_MAX_QUERY_CMD_RETRIES);
        let query_timeout = self.query_timeout.unwrap_or(DEFAULT_QUERY_CMD_TIMEOUT);
        let cmd_timeout = self.cmd_timeout.unwrap_or(DEFAULT_QUERY_CMD_TIMEOUT);

        let network_contacts = match self.network_contacts {
            Some(pm) => pm,
            None => {
                let network_contacts_dir = default_network_contacts_path()?;
                SectionTree::from_disk(&network_contacts_dir)
                    .await
                    .map_err(|err| Error::NetworkContacts(err.to_string()))?
            }
        };

        let mut qp2p = self.qp2p.unwrap_or_default();
        // If `idle_timeout` is not set, set it to 6 seconds (instead of 18s default).
        if qp2p.idle_timeout.is_none() {
            qp2p.idle_timeout = Some(Duration::from_secs(6));
        }

        let session = Session::new(
            qp2p,
            self.local_addr
                .unwrap_or_else(|| SocketAddr::from(DEFAULT_LOCAL_ADDR)),
            network_contacts,
        )?;

        let keypair = self.keypair.unwrap_or_else(Keypair::new_ed25519);
        let dbc_owner = self
            .dbc_owner
            .unwrap_or_else(|| Owner::from_random_secret_key(&mut rand::thread_rng()));

        let client = Client {
            keypair,
            dbc_owner,
            session,
            query_timeout,
            max_retries,
            cmd_timeout,
            chunks_cache: Arc::new(RwLock::new(Default::default())),
        };
        client.connect().await?;

        Ok(client)
    }
}

/// Parse environment variable. Returns `Ok(None)` if environment variable isn't set.
fn env_parse<F: FromStr>(s: &str) -> Result<Option<F>, F::Err> {
    let v = match std::env::var(s) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    F::from_str(&v).map(|v| Some(v))
}

fn default_network_contacts_path() -> Result<PathBuf, Error> {
    // Use `$HOME/.safe/network_contacts` directory
    let path = dirs_next::home_dir()
        .ok_or_else(|| {
            crate::Error::NetworkContacts("Could not read user's home directory".to_string())
        })?
        .join(".safe")
        .join("network_contacts")
        .join(DEFAULT_NETWORK_CONTACTS_FILE_NAME);

    Ok(path)
}

#[cfg(test)]
mod tests {}
