// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Client;
use crate::Error;

use sn_interface::{
    messaging::{
        data::{ClientMsg, DataCmd},
        ClientAuth, MsgId, WireMsg,
    },
    types::{PublicKey, Signature},
};

use backoff::{backoff::Backoff, ExponentialBackoff};
use bytes::Bytes;
use tokio::time::Duration;
use xor_name::XorName;

impl Client {
    /// Send a Cmd to the network and await a response.
    /// Cmds are not retried.
    #[instrument(skip(self), level = "debug")]
    pub async fn send_cmd_without_retry(&self, cmd: DataCmd) -> Result<(), Error> {
        self.send_cmd_with_retry(cmd, false).await
    }

    /// Sign data using the client keypair
    pub fn sign(&self, data: &[u8]) -> Signature {
        self.keypair.sign(data)
    }

    // Send a Cmd to the network and await a response.
    // Cmds are automatically retried if an error is returned
    // This function is a private helper.
    #[instrument(skip(self), level = "debug")]
    async fn send_cmd_with_retry(&self, cmd: DataCmd, retry: bool) -> Result<(), Error> {
        let client_pk = self.public_key();
        let dst_name = cmd.dst_name();

        let debug_cmd = format!("{:?}", cmd);

        let serialised_cmd = {
            let msg = ClientMsg::Cmd(cmd);
            WireMsg::serialize_msg_payload(&msg)?
        };
        let signature = self.sign(&serialised_cmd);

        let op_limit = self.cmd_timeout;
        let max_interval = self.max_backoff_interval;

        let mut backoff = ExponentialBackoff {
            initial_interval: Duration::from_secs(1),
            max_interval,
            max_elapsed_time: Some(op_limit),
            randomization_factor: 0.5,
            ..Default::default()
        };

        let msg_id = MsgId::new();

        // this seems needed for custom settings to take effect
        backoff.reset();

        let span = info_span!("Attempting a cmd with total timeout of {op_limit:?}");
        let _ = span.enter();

        let mut attempt = 1;
        let force_new_link = false;
        loop {
            debug!(
                "Attempting {:?} (attempt #{}), forcing new: {force_new_link}",
                debug_cmd, attempt
            );

            let res = self
                .send_signed_cmd(
                    dst_name,
                    client_pk,
                    msg_id,
                    serialised_cmd.clone(),
                    signature.clone(),
                    force_new_link,
                )
                .await;

            // force_new_link = true;

            // no retries wanted, so we bail on first response
            if !retry {
                break res;
            }

            if let Ok(cmd_result) = res {
                debug!("{debug_cmd} sent okay");
                break Ok(cmd_result);
            }

            trace!(
                "Failed response on {debug_cmd} cmd attempt #{attempt}: {:?}",
                res
            );

            attempt += 1;

            if let Some(delay) = backoff.next_backoff() {
                debug!("Sleeping for {delay:?} before trying attempt {attempt} cmd {debug_cmd:?} again");
                tokio::time::sleep(delay).await;
            } else {
                debug!("backoff done affter #{:?} attempts", attempt - 1);
                // we're done trying
                break res;
            }
        }
    }

    /// Send a signed `DataCmd` to the network.
    /// This is to be part of a public API, for the user to
    /// provide the serialised and already signed cmd.
    pub async fn send_signed_cmd(
        &self,
        dst_address: XorName,
        client_pk: PublicKey,
        msg_id: MsgId,
        serialised_cmd: Bytes,
        signature: Signature,
        force_new_link: bool,
    ) -> Result<(), Error> {
        let auth = ClientAuth {
            public_key: client_pk,
            signature,
        };

        self.session
            .send_cmd(
                dst_address,
                auth,
                serialised_cmd,
                msg_id,
                force_new_link,
                #[cfg(feature = "traceroute")]
                self.public_key(),
            )
            .await
    }

    /// Send a DataCmd to the network without awaiting for a response.
    /// Cmds are automatically retried using exponential backoff if an error is returned.
    /// This function is a helper private to this module.
    #[instrument(skip_all, level = "debug", name = "client-api send cmd")]
    pub(crate) async fn send_cmd(&self, cmd: DataCmd) -> Result<(), Error> {
        self.send_cmd_with_retry(cmd, true).await
    }
}
