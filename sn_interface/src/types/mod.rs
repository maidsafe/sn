// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! SAFE network data types.

/// public key types (ed25519)
pub mod keys;
/// Standardised log markers for various events
pub mod log_markers;
/// Register data type
pub mod register;
/// Encoding utils
pub mod utils;

mod address;
mod cache;
mod chunk;
mod errors;
mod peer;

use crate::messaging::data::CmdResponse;
pub use crate::messaging::{
    data::{Error as DataError, RegisterCmd},
    SectionSig,
};

pub use address::{ChunkAddress, DataAddress, RegisterAddress, SpentbookAddress};
pub use cache::Cache;
pub use chunk::{Chunk, MAX_CHUNK_SIZE_IN_BYTES};
pub use errors::{Error, Result};
pub use keys::{
    keypair::{BlsKeypairShare, Encryption, Keypair, OwnerType, Signing},
    public_key::PublicKey,
    secret_key::SecretKey,
    signature::{Signature, SignatureShare},
};
pub use peer::Peer;

use serde::{Deserialize, Serialize};
use xor_name::XorName;

// TODO: temporary type tag for spentbook since its underlying data type is
// still not implemented, it uses a Public Register for now.
pub const SPENTBOOK_TYPE_TAG: u64 = 0;

const REGISTER_CMD_SIZE: usize = 300;

/// Register data exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicatedRegisterLog {
    ///
    pub address: RegisterAddress,
    ///
    pub op_log: Vec<RegisterCmd>,
}

///
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub enum ReplicatedData {
    /// A chunk of data.
    Chunk(Chunk),
    /// A single cmd for a register.
    RegisterWrite(RegisterCmd),
    /// An entire op log of a register.
    RegisterLog(ReplicatedRegisterLog),
    /// A single cmd for a spentbook.
    SpentbookWrite(RegisterCmd),
    /// An entire op log of a spentbook.
    SpentbookLog(ReplicatedRegisterLog),
}

impl ReplicatedData {
    pub fn name(&self) -> XorName {
        match self {
            Self::Chunk(chunk) => *chunk.name(),
            Self::RegisterLog(log) => *log.address.name(),
            Self::RegisterWrite(cmd) => *cmd.dst_address().name(),
            Self::SpentbookLog(log) => *log.address.name(),
            Self::SpentbookWrite(cmd) => *cmd.dst_address().name(),
        }
    }

    pub fn address(&self) -> DataAddress {
        match self {
            Self::Chunk(chunk) => DataAddress::Bytes(*chunk.address()),
            Self::RegisterLog(log) => DataAddress::Register(log.address),
            Self::RegisterWrite(cmd) => DataAddress::Register(cmd.dst_address()),
            Self::SpentbookLog(log) => {
                DataAddress::Spentbook(SpentbookAddress::new(*log.address.name()))
            }
            Self::SpentbookWrite(cmd) => {
                DataAddress::Spentbook(SpentbookAddress::new(*cmd.dst_address().name()))
            }
        }
    }
}
