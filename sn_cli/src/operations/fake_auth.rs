// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use crate::APP_ID;
use tracing::debug;
use sn_api::Safe;
use color_eyre::Result;

pub async fn authorise_cli(
    _safe: Safe,
    _endpoint: Option<String>,
    _is_self_authing: bool,
) -> Result<()> {
    debug!("Fake-auth is enabled so we don't try to read the credentials file or send authorisation request");
    Ok(())
}

pub fn clear_credentials() -> Result<()> {
    debug!("Fake-auth is enabled so we don't try to clear the credentials file");
    Ok(())
}

pub async fn connect(mut safe: Safe) -> Result<()> {
    debug!("Fake-auth is enabled so we don't try to read the credentials file");

    safe.connect(APP_ID, Some("fake-auth-credentials"))
        .await
        .context("Unexpected error when trying to connect with fake auth/network")
}
