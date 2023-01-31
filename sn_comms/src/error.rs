// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use sn_interface::types::Peer;
use thiserror::Error;

/// The type returned by the `sn_routing` message handling methods.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Internal error.
#[derive(Debug, Error)]
#[allow(missing_docs)]
pub enum Error {
    /// Any unknown node comms should be bidi, initiated by the other side
    #[error("Attempted to create a connection to an unknown node: {0:?}")]
    CreatingConnectionToUnknownNode(Peer),
    #[error("Cannot connect to the endpoint: {0}")]
    CannotConnectEndpoint(#[from] qp2p::EndpointError),
    #[error("Address not reachable: {0}")]
    AddressNotReachable(#[from] qp2p::RpcError),
    #[error("Content of a received message is inconsistent.")]
    InvalidMessage,
    #[error("Failed to send a message to {0}")]
    FailedSend(Peer),
}

impl From<qp2p::SendError> for Error {
    fn from(error: qp2p::SendError) -> Self {
        Self::AddressNotReachable(qp2p::RpcError::Send(error))
    }
}
