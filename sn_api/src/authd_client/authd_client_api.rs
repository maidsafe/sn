// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{
    common::send_authd_request,
    constants::{SN_AUTHD_ENDPOINT_HOST, SN_AUTHD_ENDPOINT_PORT},
    notifs_endpoint::jsonrpc_listen,
};

use crate::{AuthReq, AuthedAppsList, Error, Result, SafeAuthReqId};

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio::{
    sync::{mpsc, oneshot},
    task,
};
use tracing::{debug, error, info, trace};

#[cfg(not(target_os = "windows"))]
const SN_AUTHD_EXECUTABLE: &str = "sn_authd";

#[cfg(target_os = "windows")]
const SN_AUTHD_EXECUTABLE: &str = "sn_authd.exe";

const ENV_VAR_SN_AUTHD_PATH: &str = "SN_AUTHD_PATH";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AuthdStatus {
    pub safe_unlocked: bool,
    pub num_auth_reqs: u32,
    pub num_notif_subs: u32,
    pub authd_version: String,
}

// Type of the list of pending authorisation requests
pub type PendingAuthReqs = Vec<AuthReq>;

// Type of the function/callback invoked for notifying and querying if an authorisation request
// shall be allowed. All the relevant information about the authorisation request is passed as args to the callback.
pub type AuthAllowPrompt = dyn Fn(AuthReq) -> Option<bool> + std::marker::Send + std::marker::Sync;

// Authenticator method for getting a status report of the sn_authd
const SN_AUTHD_METHOD_STATUS: &str = "status";

// Authenticator method for unlocking a Safe
const SN_AUTHD_METHOD_UNLOCK: &str = "unlock";

// Authenticator method for locking a Safe
const SN_AUTHD_METHOD_LOCK: &str = "lock";

// Authenticator method for creating a new 'Safe'
const SN_AUTHD_METHOD_CREATE: &str = "create";

// Authenticator method for fetching list of authorised apps
const SN_AUTHD_METHOD_AUTHED_APPS: &str = "authed-apps";

// Authenticator method for revoking applications and/or permissions
const SN_AUTHD_METHOD_REVOKE: &str = "revoke";

// Authenticator method for retrieving the list of pending authorisation requests
const SN_AUTHD_METHOD_AUTH_REQS: &str = "auth-reqs";

// Authenticator method for allowing an authorisation request
const SN_AUTHD_METHOD_ALLOW: &str = "allow";

// Authenticator method for denying an authorisation request
const SN_AUTHD_METHOD_DENY: &str = "deny";

// Authenticator method for subscribing to authorisation requests notifications
const SN_AUTHD_METHOD_SUBSCRIBE: &str = "subscribe";

// Authenticator method for unsubscribing from authorisation requests notifications
const SN_AUTHD_METHOD_UNSUBSCRIBE: &str = "unsubscribe";

// authd subcommand to update the binary to new available released version
const SN_AUTHD_CMD_UPDATE: &str = "update";

// authd subcommand to start the daemon
const SN_AUTHD_CMD_START: &str = "start";

// authd subcommand to stop the daemon
const SN_AUTHD_CMD_STOP: &str = "stop";

// authd subcommand to restart the daemon
const SN_AUTHD_CMD_RESTART: &str = "restart";

// Authd Client API
pub struct SafeAuthdClient {
    pub authd_endpoint: String,
    pub authd_cert_path: PathBuf,
    pub authd_notify_cert_path: PathBuf,
    pub authd_notify_key_path: PathBuf,
    subscribed_endpoint: Option<(String, task::JoinHandle<()>, task::JoinHandle<()>)>,
    // TODO: add a session_token field to use for communicating with authd for restricted operations,
    // we should restrict operations like subscribe, or allow/deny, to only be accepted with a valid token
    // session_token: String,
}

impl Drop for SafeAuthdClient {
    fn drop(&mut self) {
        trace!("SafeAuthdClient instance being dropped...");
        // Let's try to unsubscribe if we had a subscribed endpoint
        match &self.subscribed_endpoint {
            None => {}
            Some((url, _, _)) => {
                match futures::executor::block_on(send_unsubscribe(
                    url,
                    &self.authd_endpoint,
                    &self.authd_cert_path,
                )) {
                    Ok(msg) => {
                        debug!("{}", msg);
                    }
                    Err(err) => {
                        // We are still ok, it was just us trying to be nice and unsubscribe if possible
                        // It could be the case we were already unsubscribe automatically by authd before
                        // we were attempting to do it now, which can happen due to our endpoint
                        // being unresponsive, so it's all ok
                        debug!("Failed to unsubscribe endpoint from authd: {}", err);
                    }
                }
            }
        }
    }
}

impl SafeAuthdClient {
    pub fn new<P: AsRef<Path>>(
        endpoint: Option<String>,
        authd_cert_path: P,
        authd_notify_cert_path: P,
        authd_notify_key_path: P,
    ) -> Self {
        let endpoint = match endpoint {
            None => format!("{SN_AUTHD_ENDPOINT_HOST}:{SN_AUTHD_ENDPOINT_PORT}"),
            Some(endpoint) => endpoint,
        };
        debug!("Creating new authd client for endpoint {}", endpoint);
        Self {
            authd_endpoint: endpoint,
            authd_cert_path: PathBuf::from(authd_cert_path.as_ref()),
            authd_notify_cert_path: PathBuf::from(authd_notify_cert_path.as_ref()),
            authd_notify_key_path: PathBuf::from(authd_notify_key_path.as_ref()),
            subscribed_endpoint: None,
        }
    }

    // Print out the version of the Authenticator binary
    pub fn version(&self, authd_path: Option<&str>) -> Result<()> {
        authd_run_cmd(authd_path, &["--version"])
    }

    // Update the Authenticator binary to a new released version
    pub fn update(&self, authd_path: Option<&str>) -> Result<()> {
        authd_run_cmd(authd_path, &[SN_AUTHD_CMD_UPDATE])
    }

    // Start the Authenticator daemon
    pub fn start(&self, authd_path: Option<&str>) -> Result<()> {
        authd_run_cmd(
            authd_path,
            &[SN_AUTHD_CMD_START, "--listen", &self.authd_endpoint],
        )
    }

    // Stop the Authenticator daemon
    pub fn stop(&self, authd_path: Option<&str>) -> Result<()> {
        authd_run_cmd(authd_path, &[SN_AUTHD_CMD_STOP])
    }

    // Restart the Authenticator daemon
    pub fn restart(&self, authd_path: Option<&str>) -> Result<()> {
        authd_run_cmd(
            authd_path,
            &[SN_AUTHD_CMD_RESTART, "--listen", &self.authd_endpoint],
        )
    }

    // Send a request to remote authd endpoint to obtain a status report
    pub async fn status(&mut self) -> Result<AuthdStatus> {
        debug!("Attempting to retrieve status report from remote authd...");
        let status_report = send_authd_request::<AuthdStatus>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_STATUS,
            serde_json::Value::Null,
        )
        .await?;

        info!(
            "SAFE status report retrieved successfully: {:?}",
            status_report
        );
        Ok(status_report)
    }

    // Send an action request to remote authd endpoint to unlock a Safe
    pub async fn unlock(&mut self, passphrase: &str, password: &str) -> Result<()> {
        debug!("Attempting to unlock a Safe on remote authd...");
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_UNLOCK,
            json!(vec![passphrase, password]),
        )
        .await?;

        info!("The Safe was unlocked successful: {}", authd_response);
        // TODO: store the authd session token, replacing an existing one
        // self.session_token = authd_response;

        Ok(())
    }

    // Send an action request to remote authd endpoint to lock a Safe
    pub async fn lock(&mut self) -> Result<()> {
        debug!("Locking the Safe on a remote authd...");
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_LOCK,
            serde_json::Value::Null,
        )
        .await?;

        info!("Locking action was successful: {}", authd_response);

        // TODO: clean up the stored authd session token
        // self.session_token = "".to_string();

        Ok(())
    }

    // Sends a request to create an 'Safe' to the SAFE Authenticator
    // TODO: accept a payment proof to be used to pay the cost of creating the 'Safe'
    pub async fn create(&self, passphrase: &str, password: &str) -> Result<()> {
        debug!("Attempting to create a Safe using remote authd...");
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_CREATE,
            json!(vec![passphrase, password]),
        )
        .await?;

        debug!("Creation of a Safe was successful: {}", authd_response);
        Ok(())
    }

    // Get the list of applications authorised from remote authd
    pub async fn authed_apps(&self) -> Result<AuthedAppsList> {
        debug!("Attempting to fetch list of authorised apps from remote authd...");
        let authed_apps_list = send_authd_request::<AuthedAppsList>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_AUTHED_APPS,
            serde_json::Value::Null,
        )
        .await?;

        debug!(
            "List of applications authorised successfully received: {:?}",
            authed_apps_list
        );
        Ok(authed_apps_list)
    }

    // Revoke all permissions from an application
    pub async fn revoke_app(&self, app_id: &str) -> Result<()> {
        debug!(
            "Requesting to revoke permissions from application: {}",
            app_id
        );
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_REVOKE,
            json!(app_id),
        )
        .await?;

        debug!(
            "Application revocation action successful: {}",
            authd_response
        );
        Ok(())
    }

    // Get the list of pending authorisation requests from remote authd
    pub async fn auth_reqs(&self) -> Result<PendingAuthReqs> {
        debug!("Attempting to fetch list of pending authorisation requests from remote authd...");
        let auth_reqs_list = send_authd_request::<PendingAuthReqs>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_AUTH_REQS,
            serde_json::Value::Null,
        )
        .await?;

        debug!(
            "List of pending authorisation requests successfully received: {:?}",
            auth_reqs_list
        );
        Ok(auth_reqs_list)
    }

    // Allow an authorisation request
    pub async fn allow(&self, req_id: SafeAuthReqId) -> Result<()> {
        debug!("Requesting to allow authorisation request: {}", req_id);
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_ALLOW,
            json!(req_id.to_string()),
        )
        .await?;

        debug!(
            "Action to allow authorisation request was successful: {}",
            authd_response
        );
        Ok(())
    }

    // Deny an authorisation request
    pub async fn deny(&self, req_id: SafeAuthReqId) -> Result<()> {
        debug!("Requesting to deny authorisation request: {}", req_id);
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_DENY,
            json!(req_id.to_string()),
        )
        .await?;

        debug!(
            "Action to deny authorisation request was successful: {}",
            authd_response
        );
        Ok(())
    }

    // Subscribe a callback to receive notifications to allow/deny authorisation requests
    // We support having only one subscripton at a time, a previous subscription will be dropped
    pub async fn subscribe<
        CB: 'static + Fn(AuthReq) -> Option<bool> + std::marker::Send + std::marker::Sync,
    >(
        &mut self,
        endpoint_url: &str,
        _app_id: &str,
        allow_cb: CB,
    ) -> Result<()> {
        debug!("Subscribing to receive authorisation requests notifications...",);

        let cert_path = self.authd_cert_path.to_str().ok_or_else(|| {
            Error::AuthdClientError("Could not convert authd_cert_path to string".to_string())
        })?;
        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_SUBSCRIBE,
            json!(vec![endpoint_url, cert_path]),
        ).await.map_err(|err| Error::AuthdClientError(format!("Failed when trying to subscribe endpoint URL ({endpoint_url}) to receive authorisation request for self-auth: {err}")))?;

        debug!(
            "Successfully subscribed to receive authorisation requests notifications: {}",
            authd_response
        );

        // Start listening first
        // We need a channel to receive auth req notifications from the thread running the QUIC endpoint
        let (tx, mut rx) = mpsc::unbounded_channel::<(AuthReq, oneshot::Sender<Option<bool>>)>();

        let listen = endpoint_url.to_string();
        // TODO: if there was a previous subscription,
        // make sure we kill/stop the previously created tasks

        let authd_notify_cert_path = self.authd_notify_cert_path.clone();
        let authd_notify_key_path = self.authd_notify_key_path.clone();
        let endpoint_thread_join_handle = tokio::spawn(async move {
            match jsonrpc_listen(&listen, &authd_notify_cert_path, &authd_notify_key_path, tx).await
            {
                Ok(()) => {
                    info!("Endpoint successfully launched for receiving auth req notifications");
                }
                Err(err) => {
                    error!(
                        "Failed to launch endpoint for receiving auth req notifications: {}",
                        err
                    );
                }
            }
        });

        let cb = Box::new(allow_cb);
        let cb_thread_join_handle = tokio::spawn(async move {
            while let Some((auth_req, decision_tx)) = rx.recv().await {
                debug!(
                    "Notification for authorisation request ({}) from app ID '{}' received",
                    auth_req.req_id, auth_req.app_id
                );

                // Let's get the decision from the user by invoking the callback provided
                let user_decision = cb(auth_req);

                // Send the decision received back to authd-client so it
                // can in turn send it to authd
                match decision_tx.send(user_decision) {
                    Ok(_) => debug!("Auth req decision sent to authd"),
                    Err(_) => error!("Auth req decision couldn't be sent back to authd"),
                };
            }
        });

        self.subscribed_endpoint = Some((
            endpoint_url.to_string(),
            endpoint_thread_join_handle,
            cb_thread_join_handle,
        ));

        Ok(())
    }

    // Subscribe an endpoint URL where notifications to allow/deny authorisation requests shall be sent
    pub async fn subscribe_url(&self, endpoint_url: &str) -> Result<()> {
        debug!(
            "Subscribing '{}' as endpoint for authorisation requests notifications...",
            endpoint_url
        );

        let authd_response = send_authd_request::<String>(
            &self.authd_cert_path,
            &self.authd_endpoint,
            SN_AUTHD_METHOD_SUBSCRIBE,
            json!(vec![endpoint_url]),
        )
        .await?;

        debug!(
            "Successfully subscribed a URL for authorisation requests notifications: {}",
            authd_response
        );
        Ok(())
    }

    // Unsubscribe from notifications to allow/deny authorisation requests
    pub async fn unsubscribe(&mut self, endpoint_url: &str) -> Result<()> {
        debug!("Unsubscribing from authorisation requests notifications...",);
        let authd_response =
            send_unsubscribe(endpoint_url, &self.authd_endpoint, &self.authd_cert_path).await?;
        debug!(
            "Successfully unsubscribed from authorisation requests notifications: {}",
            authd_response
        );

        // If the URL is the same as the endpoint locally launched, terminate the thread
        if let Some((url, _, _)) = &self.subscribed_endpoint {
            if endpoint_url == url {
                // TODO: send signal to stop the currently running tasks
                self.subscribed_endpoint = None;
            }
        }

        Ok(())
    }
}

async fn send_unsubscribe(
    endpoint_url: &str,
    authd_endpoint: &str,
    authd_cert_path: &Path,
) -> Result<String> {
    send_authd_request::<String>(
        authd_cert_path,
        authd_endpoint,
        SN_AUTHD_METHOD_UNSUBSCRIBE,
        json!(endpoint_url),
    )
    .await
}

fn authd_run_cmd(authd_path: Option<&str>, args: &[&str]) -> Result<()> {
    let mut path = get_authd_bin_path(authd_path)?;
    path.push(SN_AUTHD_EXECUTABLE);
    let path_str = path.display().to_string();
    debug!("Attempting to {} authd from '{}' ...", args[0], path_str);

    let output = Command::new(&path_str)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| {
            Error::AuthdClientError(format!("Failed to execute authd from '{path_str}': {err}",))
        })?;

    if output.status.success() {
        io::stdout()
            .write_all(&output.stdout)
            .map_err(|err| Error::AuthdClientError(format!("Failed to output stdout: {err}")))?;
        Ok(())
    } else {
        match output.status.code() {
            Some(10) => {
                // sn_authd exit code 10 is sn_authd::errors::Error::AuthdAlreadyStarted
                Err(Error::AuthdAlreadyStarted(format!(
                       "Failed to start sn_authd daemon '{path_str}' as an instance seems to be already running",
                   )))
            }
            Some(_) | None => Err(Error::AuthdError(format!(
                "Failed when invoking sn_authd executable from '{path_str}'",
            ))),
        }
    }
}

fn get_authd_bin_path(authd_path: Option<&str>) -> Result<PathBuf> {
    match authd_path {
        Some(p) => Ok(PathBuf::from(p)),
        None => {
            // if SN_AUTHD_PATH is set it then overrides default
            if let Ok(authd_path) = std::env::var(ENV_VAR_SN_AUTHD_PATH) {
                Ok(PathBuf::from(authd_path))
            } else {
                let mut path = dirs_next::home_dir().ok_or_else(|| {
                    Error::AuthdClientError("Failed to obtain user's home path".to_string())
                })?;

                path.push(".safe");
                path.push("authd");
                Ok(path)
            }
        }
    }
}
