// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{convert_to_error_msg, Error, FileStore, Result, UsedSpace};

use sn_interface::{
    messaging::system::NodeQueryResponse,
    types::{log_markers::LogMarker, Chunk, ChunkAddress},
};

use std::{
    fmt::{self, Display, Formatter},
    io::ErrorKind,
    path::Path,
};
use tracing::info;

const CHUNKS_DB_NAME: &str = "chunks";

/// Operations on data chunks.
#[derive(Clone, Debug)]
pub(super) struct ChunkStorage {
    file_store: FileStore,
}

impl ChunkStorage {
    pub(crate) fn new(path: &Path, used_space: UsedSpace) -> Result<Self> {
        Ok(Self {
            file_store: FileStore::new(path.join(CHUNKS_DB_NAME), used_space)?,
        })
    }

    pub(crate) fn addrs(&self) -> Vec<ChunkAddress> {
        self.file_store.list_all_chunk_addrs()
    }

    pub(crate) async fn remove_chunk(&self, address: &ChunkAddress) -> Result<()> {
        trace!("Removing chunk, {:?}", address);
        self.file_store.delete_chunk_data(address).await
    }

    pub(crate) async fn get_chunk(&self, address: &ChunkAddress) -> Result<Chunk> {
        debug!("Getting chunk {:?}", address);

        match self.file_store.read_chunk_data(address).await {
            Ok(res) => Ok(res),
            Err(error) => match error {
                Error::Io(io_error) if io_error.kind() == ErrorKind::NotFound => {
                    Err(Error::ChunkNotFound(*address.name()))
                }
                something_else => Err(something_else),
            },
        }
    }

    // Read chunk from local store and return NodeQueryResponse
    pub(crate) async fn get(&self, address: &ChunkAddress) -> NodeQueryResponse {
        trace!("{:?}", LogMarker::ChunkQueryReceviedAtAdult);
        NodeQueryResponse::GetChunk(self.get_chunk(address).await.map_err(convert_to_error_msg))
    }

    /// Store a chunk in the local disk store
    /// If that chunk was already in the local store, just overwrites it
    #[instrument(skip_all)]
    pub(super) async fn store(&self, chunk: Chunk) -> Result<()> {
        let addr = chunk.address();
        let chunk_filepath = self.file_store.chunk_addr_to_filepath(addr)?;
        if chunk_filepath.exists() {
            info!("{}: Data already exists, not storing: {:?}", self, addr);
            // Nothing more to do here
            return Err(Error::DataExists);
        }

        // cheap extra security check for space (prone to race conditions)
        // just so we don't go too much overboard
        // should not be triggered as chunks should not be sent to full adults
        if !self.file_store.can_add(chunk.value().len()) {
            return Err(Error::NotEnoughSpace);
        }

        // store the data
        trace!("{:?}", LogMarker::StoringChunk);
        self.file_store.write_chunk_data(chunk).await?;
        trace!("{:?}", LogMarker::StoredNewChunk);

        Ok(())
    }
}

impl Display for ChunkStorage {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "ChunkStorage")
    }
}
