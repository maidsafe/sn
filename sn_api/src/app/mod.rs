// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

// --------------------------------------------------------------------
// ------ The following is what's meant to be the public API -------

pub mod files;
pub mod keys;
pub mod multimap;
pub mod nrs;
pub mod register;
pub mod resolver;
pub mod wallet;

pub use crate::safeurl::*;
pub use consts::DEFAULT_XORURL_BASE;
pub use helpers::parse_tokens_amount;
pub use xor_name::{XorName, XOR_NAME_LEN};

// --------------------------------------------------------------------

mod auth;
mod consts;
mod helpers;

#[cfg(test)]
mod test_helpers;

use super::{common, constants, Error, Result};

use sn_client::{Client, ClientConfig, DEFAULT_OPERATION_TIMEOUT};
use sn_dbc::Owner;
use sn_interface::types::Keypair;

use std::{path::Path, time::Duration};
use tracing::debug;

const APP_NOT_CONNECTED: &str = "Application is not connected to the network";

#[derive(Clone)]
pub struct Safe {
    client: Option<Client>,
    pub xorurl_base: XorUrlBase,
    pub dry_run_mode: bool,
}

impl Safe {
    /// Create a Safe instance without connecting to the SAFE Network
    pub fn dry_runner(xorurl_base: Option<XorUrlBase>) -> Self {
        Self {
            client: None,
            xorurl_base: xorurl_base.unwrap_or(DEFAULT_XORURL_BASE),
            dry_run_mode: true,
        }
    }

    /// Create a Safe instance connected to the SAFE Network
    pub async fn connected(
        keypair: Option<Keypair>,
        config_path: Option<&Path>,
        xorurl_base: Option<XorUrlBase>,
        timeout: Option<Duration>,
        dbc_owner: Option<Owner>,
    ) -> Result<Self> {
        let mut safe = Self {
            client: None,
            xorurl_base: xorurl_base.unwrap_or(DEFAULT_XORURL_BASE),
            dry_run_mode: false,
        };

        safe.connect(keypair, config_path, timeout, dbc_owner)
            .await?;

        Ok(safe)
    }

    /// Connect to the SAFE Network
    pub async fn connect(
        &mut self,
        keypair: Option<Keypair>,
        config_path: Option<&Path>,
        timeout: Option<Duration>,
        dbc_owner: Option<Owner>,
    ) -> Result<()> {
        debug!("Connecting to SAFE Network...");

        let config_path = config_path.map(|p| p.to_path_buf());

        debug!("Client to be instantiated with specific pk?: {:?}", keypair);

        let config = ClientConfig::new(
            None,
            None,
            config_path.as_deref(),
            timeout.or(Some(DEFAULT_OPERATION_TIMEOUT)),
            timeout.or(Some(DEFAULT_OPERATION_TIMEOUT)),
            None,
        )
        .await;

        self.client = Some(
            Client::new(config, keypair, dbc_owner)
                .await
                .map_err(|err| {
                    Error::ConnectionError(format!(
                        "Failed to connect to the SAFE Network: {:?}",
                        err
                    ))
                })?,
        );

        debug!("Successfully connected to the Network!!!");

        Ok(())
    }

    /// Returns true if we already have a connection with the network
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    // Private helper to obtain the Client instance
    pub(crate) fn get_safe_client(&self) -> Result<&Client> {
        match &self.client {
            Some(client) => Ok(client),
            None => Err(Error::ConnectionError(APP_NOT_CONNECTED.to_string())),
        }
    }
}
