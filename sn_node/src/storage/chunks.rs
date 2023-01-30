// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{
    /*list_files_in,*/ prefix_tree_path, used_space::StorageLevel, Error, Result, UsedSpace,
};

use sn_interface::{
    messaging::system::NodeQueryResponse,
    types::{log_markers::LogMarker, Chunk, ChunkAddress},
};

use bytes::Bytes;
use dashmap::DashMap;
use hex::FromHex;
use std::{
    fmt::{self, Display, Formatter},
    //io::{self, ErrorKind},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::fs::metadata;
use tracing::info;
use xor_name::XorName;

const CHUNKS_STORE_DIR_NAME: &str = "chunks";

/// Operations on data chunks.
#[derive(Clone, Debug)]
pub(super) struct ChunkStorage {
    file_store_path: PathBuf,
    used_space: UsedSpace,
    cache: Arc<DashMap<PathBuf, Bytes>>,
}

impl ChunkStorage {
    /// Creates a new `ChunkStorage` at the specified root location
    ///
    /// If the location specified already contains a `ChunkStorage`, it is simply used
    ///
    /// Used space of the dir is tracked
    pub(super) fn new(path: &Path, used_space: UsedSpace) -> Result<Self> {
        Ok(Self {
            file_store_path: path.join(CHUNKS_STORE_DIR_NAME),
            used_space,
            cache: Arc::new(DashMap::new()),
        })
    }

    pub(super) fn addrs(&self) -> Vec<ChunkAddress> {
        /* CACHE-ONLY
        list_files_in(&self.file_store_path)
            .iter()
            .filter_map(|filepath| Self::chunk_filepath_to_address(filepath).ok())
            .collect()
        */
        self.cache
            .iter()
            .filter_map(|entry| {
                let filepath = entry.key();
                Self::chunk_filepath_to_address(filepath).ok()
            })
            .collect()
    }

    fn chunk_filepath_to_address(path: &Path) -> Result<ChunkAddress> {
        let filename = path
            .file_name()
            .ok_or_else(|| Error::NoFilename(path.to_path_buf()))?
            .to_str()
            .ok_or_else(|| Error::InvalidFilename(path.to_path_buf()))?;

        let xorname = XorName(<[u8; 32]>::from_hex(filename)?);
        Ok(ChunkAddress(xorname))
    }

    fn chunk_addr_to_filepath(&self, addr: &ChunkAddress) -> PathBuf {
        let xorname = *addr.name();
        let path = prefix_tree_path(&self.file_store_path, xorname);
        let filename = hex::encode(xorname);
        path.join(filename)
    }

    pub(super) async fn remove_chunk(&self, address: &ChunkAddress) -> Result<()> {
        trace!("Removing chunk, {:?}", address);
        let filepath = self.chunk_addr_to_filepath(address);
        let meta = metadata(filepath.clone()).await?;
        /* CACHE-ONLY
        remove_file(filepath).await?;
        */
        let _ = self.cache.remove(&filepath);
        // CACHE-ONLY

        self.used_space.decrease(meta.len() as usize);
        Ok(())
    }

    pub(super) async fn get_chunk(&self, address: &ChunkAddress) -> Result<Chunk> {
        let filepath = self.chunk_addr_to_filepath(address);
        debug!("Getting chunk {:?} at {}", address, filepath.display());
        /* CACHE-ONLY
        match read(filepath).await {
        */
        let cached_bytes = self.cache.get(&filepath);
        // CACHE-ONLY
        match cached_bytes {
            Some(bytes) => {
                let chunk = Chunk::new(bytes.to_vec().into());
                if chunk.address() != address {
                    // This can happen if the content read is empty, or incomplete,
                    // possibly due to an issue with the OS synchronising to disk,
                    // resulting in a mismatch with recreated address of the Chunk.
                    Err(Error::ChunkAddrMismatch {
                        addr: *address,
                        chunk_addr: *chunk.address(),
                    })
                } else {
                    Ok(chunk)
                }
            }
            /*
            Err(io_error @ io::Error { .. }) if io_error.kind() == ErrorKind::NotFound => {
                Err(Error::ChunkNotFound(*address.name()))
            }
            Err(other) => Err(other.into()),
            */
            None => Err(Error::ChunkNotFound(*address.name())),
        }
    }

    // Read chunk from local store and return NodeQueryResponse
    pub(super) async fn get(&self, address: &ChunkAddress) -> NodeQueryResponse {
        trace!(
            "{:?} {address:?}",
            LogMarker::ChunkQueryReceviedAtStoringNode
        );
        NodeQueryResponse::GetChunk(self.get_chunk(address).await.map_err(|error| error.into()))
    }

    /// Store a chunk in the local disk store unless it is already there
    #[instrument(skip_all)]
    pub(super) async fn store(&self, chunk: &Chunk) -> Result<StorageLevel> {
        let addr = chunk.address();
        let filepath = self.chunk_addr_to_filepath(addr);

        /* CACHE-ONLY
        if filepath.exists() {
            info!(
                "{}: Chunk data already exists, not storing: {:?}",
                self, addr
            );
            // Nothing more to do here
            return Ok(StorageLevel::NoChange);
        }
        */

        // Cheap extra security check for space (prone to race conditions)
        // just so we don't go too much overboard
        // should not be triggered as chunks should not be sent to full adults
        if !self.used_space.can_add(chunk.value().len()) {
            return Err(Error::NotEnoughSpace);
        }

        // Store the data on disk
        trace!("{:?} {addr:?}", LogMarker::StoringChunk);
        /* CACHE-ONLY
        if let Some(dirs) = filepath.parent() {
            create_dir_all(dirs).await?;
        }

        let mut file = File::create(filepath).await?;

        file.write_all(chunk.value()).await?;
        // Let's sync up OS data to disk to reduce the chances of
        // concurrent reading failing by reading an empty/incomplete file
        file.sync_data().await?;
        */
        if self
            .cache
            .insert(filepath.clone(), chunk.value().clone())
            .is_some()
        {
            info!("{self}: Chunk data already existed: {addr:?}");
        }

        let storage_level = self.used_space.increase(chunk.value().len());
        trace!(
            "{:?} {addr:?} at {}",
            LogMarker::StoredNewChunk,
            filepath.display()
        );

        Ok(storage_level)
    }
}

impl Display for ChunkStorage {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "ChunkStorage")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_interface::types::utils::random_bytes;

    use eyre::{eyre, Result};
    use futures::future::join_all;
    use rayon::prelude::*;
    use tempfile::tempdir;

    fn init_file_store() -> ChunkStorage {
        let root = tempdir().expect("Failed to create temporary directory for chunk disk store");
        ChunkStorage::new(root.path(), UsedSpace::default())
            .expect("Failed to create chunk disk store")
    }

    #[tokio::test]
    async fn test_write_read_chunk() {
        let storage = init_file_store();
        // test that a range of different chunks return the written chunk
        for _ in 0..10 {
            let chunk = Chunk::new(random_bytes(100));

            let _ = storage.store(&chunk).await.expect("Failed to write chunk.");

            let read_chunk = storage
                .get_chunk(chunk.address())
                .await
                .expect("Failed to read chunk.");

            assert_eq!(chunk.value(), read_chunk.value());
        }
    }

    #[tokio::test]
    async fn test_write_read_async_multiple_chunks() {
        let store = init_file_store();
        let size = 100;
        let chunks: Vec<Chunk> = std::iter::repeat_with(|| Chunk::new(random_bytes(size)))
            .take(7)
            .collect();
        write_and_read_chunks(&chunks, store).await;
    }

    #[tokio::test]
    async fn test_write_read_async_multiple_identical_chunks() {
        let store = init_file_store();
        let chunks: Vec<Chunk> = std::iter::repeat(Chunk::new(Bytes::from("test_concurrent")))
            .take(7)
            .collect();
        write_and_read_chunks(&chunks, store).await;
    }

    #[tokio::test]
    async fn test_read_chunk_empty_file() -> Result<()> {
        let storage = init_file_store();

        let chunk = Chunk::new(random_bytes(100));
        let address = chunk.address();

        // create chunk file but with empty content
        /*
        let filepath = storage.chunk_addr_to_filepath(address);
        if let Some(dirs) = filepath.parent() {
            create_dir_all(dirs).await?;
        }
        let mut file = File::create(&filepath).await?;
        file.write_all(b"").await?;
        */

        // trying to read the chunk shall return ChunkNotFound error since
        // its content shouldn't match chunk address
        match storage.get_chunk(address).await {
            Ok(chunk) => Err(eyre!(
                "Unexpected Chunk read (size: {}): {chunk:?}",
                chunk.value().len()
            )),
            Err(Error::ChunkNotFound(name)) => {
                assert_eq!(name, *address.name(), "Wrong Chunk name returned in error");
                Ok(())
            }
            Err(other) => Err(eyre!("Unexpected Error type returned: {other:?}")),
        }
    }

    async fn write_and_read_chunks(chunks: &[Chunk], storage: ChunkStorage) {
        // write all chunks
        let mut tasks = Vec::new();
        for c in chunks.iter() {
            tasks.push(async { storage.store(c).await.map(|_| *c.address()) });
        }
        let results = join_all(tasks).await;

        // read all chunks
        let tasks = results.iter().flatten().map(|addr| storage.get_chunk(addr));
        let results = join_all(tasks).await;
        let read_chunks: Vec<&Chunk> = results.iter().flatten().collect();

        // verify all written were read
        assert!(chunks
            .par_iter()
            .all(|c| read_chunks.iter().any(|r| r.value() == c.value())))
    }
}
