// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

#[cfg(feature = "app")]
mod app;
#[cfg(any(feature = "app", feature = "authd_client"))]
mod ipc;

#[cfg(feature = "authd_client")]
mod authd_client;
#[cfg(feature = "authenticator")]
mod authenticator;
mod common;
mod constants;
mod errors;
mod url;

// re-export these useful types from sn_data_types
pub use safe_network::types::{
    BytesAddress, DataAddress, Keypair, PublicKey, RegisterAddress, SafeKeyAddress, Scope,
    SecretKey,
};

#[cfg(feature = "app")]
pub use app::*;
#[cfg(any(feature = "app", feature = "authd_client"))]
pub use ipc::*;

#[cfg(feature = "app")]
pub use xor_name::XorName;

#[cfg(feature = "authenticator")]
pub use authenticator::*;

#[cfg(feature = "authd_client")]
pub use authd_client::*;

#[cfg(any(feature = "authenticator", feature = "authd_client"))]
pub use common::auth_types::*;

pub use common::{ed_sk_from_hex, sk_to_hex};

pub use errors::{Error, Result};

// use the local url module (in this crate) and not the external rust url crate
pub use crate::url::*;
