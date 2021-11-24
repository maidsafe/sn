// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use thiserror::Error;

/// Custom Result type for url crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Error type returned by the API
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// InvalidXorUrl
    #[error("InvalidXorUrl: {0}")]
    InvalidXorUrl(String),
    /// InvalidInput
    #[error("InvalidInput: {0}")]
    InvalidInput(String),
    /// UnsupportedMediaType
    #[error("UnsupportedMediaType: {0}")]
    UnsupportedMediaType(String),
}
