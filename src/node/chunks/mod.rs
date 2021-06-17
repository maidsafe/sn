// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod chunk_storage;

use crate::messaging::{
    client::{ChunkRead, ChunkWrite},
    MessageId,
};
use crate::node::{
    node_ops::{NodeDuties, NodeDuty},
    Result,
};
use crate::types::{Chunk, ChunkAddress, PublicKey};
use chunk_storage::ChunkStorage;
use log::info;
use std::{
    fmt::{self, Display, Formatter},
    path::Path,
};

/// At 50% full, the node will report that it's reaching full capacity.
pub const MAX_STORAGE_USAGE_RATIO: f64 = 0.5;

/// Operations on data chunks.
pub(crate) struct Chunks {
    chunk_storage: ChunkStorage,
}

impl Chunks {
    pub async fn new(path: &Path, max_capacity: u64) -> Result<Self> {
        Ok(Self {
            chunk_storage: ChunkStorage::new(path, max_capacity).await?,
        })
    }

    pub fn keys(&self) -> Vec<ChunkAddress> {
        self.chunk_storage.keys()
    }

    pub async fn remove_chunk(&mut self, address: &ChunkAddress) -> Result<()> {
        self.chunk_storage.delete_chunk(address).await
    }

    pub fn get_chunk(&self, address: &ChunkAddress) -> Result<Chunk> {
        self.chunk_storage.get_chunk(address)
    }

    pub fn read(&self, read: &ChunkRead, msg_id: MessageId) -> NodeDuty {
        let ChunkRead::Get(address) = read;
        self.chunk_storage.get(address, msg_id)
    }

    pub async fn write(
        &mut self,
        write: &ChunkWrite,
        msg_id: MessageId,
        requester: PublicKey,
    ) -> Result<NodeDuty> {
        match &write {
            ChunkWrite::New(data) => self.chunk_storage.store(&data).await,
            ChunkWrite::DeletePrivate(address) => {
                self.chunk_storage.delete(*address, msg_id, requester).await
            }
        }
    }

    pub async fn check_storage(&self) -> Result<NodeDuties> {
        info!("Checking used storage");
        if self.chunk_storage.used_space_ratio().await > MAX_STORAGE_USAGE_RATIO {
            Ok(NodeDuties::from(NodeDuty::ReachingMaxCapacity))
        } else {
            Ok(vec![])
        }
    }

    /// Stores a chunk that Elders sent to it for replication.
    pub async fn store_for_replication(&mut self, chunk: Chunk) -> Result<NodeDuty> {
        self.chunk_storage.store_for_replication(chunk).await?;
        Ok(NodeDuty::NoOp)
    }
}

impl Display for Chunks {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "Chunks")
    }
}
