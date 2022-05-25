// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

#[cfg(test)]
mod test_client;

#[cfg(test)]
pub use test_client::{
    create_test_client, create_test_client_with, get_dbc_owner_from_secret_key_hex,
};

#[cfg(test)]
pub use sn_interface::init_logger;

use crate::Error;

#[cfg(test)]
use backoff::ExponentialBackoff;
use eyre::Result;
#[cfg(test)]
use std::{future::Future, time::Duration};

///
pub type ClientResult<T> = Result<T, Error>;

///
#[allow(clippy::needless_question_mark)]
#[cfg(test)]
pub async fn run_w_backoff_delayed<R, F, Fut>(f: F, _retries: u8, delay: usize) -> Result<R, Error>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<R, backoff::Error<Error>>>,
{
    tokio::time::sleep(tokio::time::Duration::from_secs(delay as u64)).await;
    let res = retry(
        || async { Ok(f().await?) },
        tokio::time::Duration::from_secs(5),
        tokio::time::Duration::from_secs(180),
    )
    .await;
    if res.is_err() {
        Err(Error::NoResponse)
    } else {
        res
    }
}

#[cfg(test)]
fn retry<R, E, Fn, Fut>(
    op: Fn,
    initial_interval: Duration,
    max_elapsed_time: Duration,
) -> impl Future<Output = Result<R, E>>
where
    Fn: FnMut() -> Fut,
    Fut: Future<Output = Result<R, backoff::Error<E>>>,
{
    let backoff = ExponentialBackoff {
        initial_interval,
        max_interval: max_elapsed_time,
        max_elapsed_time: Some(max_elapsed_time),
        ..Default::default()
    };

    backoff::future::retry(backoff, op)
}

#[cfg(test)]
#[macro_export]
/// Helper for tests to retry an operation awaiting for a successful response result
macro_rules! retry_loop {
    ($async_func:expr) => {
        loop {
            match $async_func.await {
                Ok(val) => break val,
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            }
        }
    };
}

#[cfg(test)]
#[macro_export]
/// Helper for tests to retry an operation awaiting for a successful response result
macro_rules! retry_err_loop {
    ($async_func:expr) => {
        loop {
            match $async_func.await {
                Ok(_) => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
                Err(err) => break err,
            }
        }
    };
}

#[cfg(test)]
#[macro_export]
/// Helper for tests to retry an operation awaiting for a specific result
macro_rules! retry_loop_for_pattern {
    ($async_func:expr, $pattern:pat $(if $cond:expr)?) => {
        loop {
            let result = $async_func.await;
            match &result {
                $pattern $(if $cond)? => break result,
                Ok(_) | Err(_) => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            }
        }
    };
}
