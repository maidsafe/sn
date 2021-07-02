// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{
    data::{Data, DataId},
    DataStore, Error, Result as DataStoreResult, Result, Subdir, ToDbKey, UsedSpace,
};
use crate::types::{ChunkAddress, DataAddress};
use rand::{distributions::Standard, rngs::ThreadRng, Rng};
use serde::{Deserialize, Serialize};
use std::{path::Path, u64};
use tempdir::TempDir;
use xor_name::XorName;

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TestData {
    id: Id,
    value: Vec<u8>,
}

impl Data for TestData {
    type Id = Id;

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct Id(u64);

impl ToDbKey for Id {}
impl DataId for Id {
    fn to_data_address(&self) -> DataAddress {
        DataAddress::Chunk(ChunkAddress::Public(XorName::from_content(&[&self
            .0
            .to_be_bytes()])))
    }
}

impl Subdir for DataStore<TestData> {
    fn subdir() -> &'static Path {
        Path::new("test")
    }
}

// TODO: use seedable rng
fn new_rng() -> ThreadRng {
    rand::thread_rng()
}

fn temp_dir() -> DataStoreResult<TempDir> {
    TempDir::new("test").map_err(|e| Error::TempDirCreationFailed(e.to_string()))
}

struct Chunks {
    data_and_sizes: Vec<(Vec<u8>, u64)>,
    total_size: u64,
}

impl Chunks {
    // Construct random amount of randomly-sized chunks, keeping track of the total size of all
    // chunks when serialised.
    fn gen<R: Rng>(rng: &mut R) -> Result<Self> {
        let mut chunks = Self {
            data_and_sizes: vec![],
            total_size: 0,
        };
        let chunk_count: u8 = rng.gen();
        for _ in 0..chunk_count {
            let size: u8 = rng.gen();
            let data = TestData {
                id: Id(0),
                value: rng.sample_iter(&Standard).take(size as usize).collect(),
            };
            let serialised_size = bincode::serialized_size(&data).map_err(Error::Bincode)?;

            chunks.total_size += serialised_size;
            chunks.data_and_sizes.push((data.value, serialised_size));
        }
        Ok(chunks)
    }
}

#[tokio::test]
async fn used_space_increases() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::<TestData>::new(root.path(), used_space).await?;

    let used_space_before = data_store.total_used_space().await;

    for (index, (data, _size)) in chunks.data_and_sizes.iter().enumerate().rev() {
        let the_data = &TestData {
            id: Id(index as u64),
            value: data.clone(),
        };

        assert!(!data_store.has(&the_data.id).await?);
        data_store.put(the_data).await?;
        assert!(data_store.has(&the_data.id).await?);
    }

    let mut used_space_after = data_store.total_used_space().await;

    while used_space_before >= used_space_after {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        used_space_after = data_store.total_used_space().await;
    }

    assert!(used_space_after > used_space_before);

    Ok(())
}

#[tokio::test]
#[ignore = "it doesn't decrease.."]
async fn used_space_decreases() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::<TestData>::new(root.path(), used_space).await?;

    for (index, (data, _size)) in chunks.data_and_sizes.iter().enumerate().rev() {
        let the_data = &TestData {
            id: Id(index as u64),
            value: data.clone(),
        };

        assert!(!data_store.has(&the_data.id).await?);
        data_store.put(the_data).await?;
        assert!(data_store.has(&the_data.id).await?);
    }

    let used_space_before = data_store.total_used_space().await;

    for key in data_store.keys().await? {
        data_store.delete(&key).await?;
    }

    let mut used_space_after = data_store.total_used_space().await;

    while used_space_after >= used_space_before {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        used_space_after = data_store.total_used_space().await;
    }

    Ok(())
}

#[tokio::test]
async fn successful_put() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::<TestData>::new(root.path(), used_space).await?;

    for (index, (data, _size)) in chunks.data_and_sizes.iter().enumerate().rev() {
        let the_data = &TestData {
            id: Id(index as u64),
            value: data.clone(),
        };
        assert!(!data_store.has(&the_data.id).await?);
        data_store.put(the_data).await?;
        assert!(data_store.has(&the_data.id).await?);
    }

    let mut keys = data_store.keys().await?;
    keys.sort();
    assert_eq!(
        (0..chunks.data_and_sizes.len())
            .map(|i| Id(i as u64))
            .collect::<Vec<_>>(),
        keys
    );

    Ok(())
}

#[tokio::test]
async fn failed_put_when_not_enough_space() -> Result<()> {
    let mut rng = new_rng();
    let root = temp_dir()?;
    let capacity = 32;
    let used_space = UsedSpace::new(capacity);
    let data_store = DataStore::new(root.path(), used_space).await?;

    let data = TestData {
        id: Id(rng.gen()),
        value: rng
            .sample_iter(&Standard)
            .take((capacity + 1) as usize)
            .collect(),
    };

    match data_store.put(&data).await {
        Err(Error::NotEnoughSpace) => (),
        x => {
            return Err(super::Error::InvalidOperation(format!(
                "Unexpected: {:?}",
                x
            )))
        }
    }

    Ok(())
}

#[tokio::test]
async fn delete() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::new(root.path(), used_space).await?;

    for (index, (data, _size)) in chunks.data_and_sizes.iter().enumerate() {
        let the_data = &TestData {
            id: Id(index as u64),
            value: data.clone(),
        };
        data_store.put(the_data).await?;

        while !data_store.has(&the_data.id).await? {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        data_store.delete(&the_data.id).await?;

        while data_store.has(&the_data.id).await? {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    Ok(())
}

#[tokio::test]
async fn put_and_get_value_should_be_same() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::new(root.path(), used_space).await?;

    for (index, (data, _)) in chunks.data_and_sizes.iter().enumerate() {
        data_store
            .put(&TestData {
                id: Id(index as u64),
                value: data.clone(),
            })
            .await?
    }

    for (index, (data, _)) in chunks.data_and_sizes.iter().enumerate() {
        let retrieved_value = data_store.get(&Id(index as u64)).await?;
        assert_eq!(*data, retrieved_value.value);
    }

    Ok(())
}

#[tokio::test]
#[ignore = "Has been failing for a long time, fix coming up."]
async fn overwrite_value() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::new(root.path(), used_space).await?;

    for (data, size) in chunks.data_and_sizes {
        data_store
            .put(&TestData {
                id: Id(0),
                value: data.clone(),
            })
            .await?;

        while !data_store.has(&Id(0)).await? {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        loop {
            let used_space = data_store.total_used_space().await;
            if used_space == size {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        assert_eq!(data_store.total_used_space().await, size);
        let retrieved_data = data_store.get(&Id(0)).await?;
        assert_eq!(data, retrieved_data.value);
    }

    Ok(())
}

#[tokio::test]
async fn get_fails_when_key_does_not_exist() -> Result<()> {
    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store: DataStore<TestData> = DataStore::new(root.path(), used_space).await?;

    let id = Id(new_rng().gen());
    match data_store.get(&id).await {
        Err(Error::KeyNotFound(_)) => (),
        x => {
            return Err(super::Error::InvalidOperation(format!(
                "Unexpected {:?}",
                x
            )))
        }
    }

    Ok(())
}

#[tokio::test]
async fn keys() -> Result<()> {
    let mut rng = new_rng();
    let chunks = Chunks::gen(&mut rng)?;

    let root = temp_dir()?;
    let used_space = UsedSpace::new(u64::MAX);
    let data_store = DataStore::new(root.path(), used_space).await?;

    for (index, (data, _)) in chunks.data_and_sizes.iter().enumerate() {
        let id = Id(index as u64);
        assert!(!data_store.keys().await?.contains(&id));
        data_store
            .put(&TestData {
                id,
                value: data.clone(),
            })
            .await?;

        let keys = data_store.keys().await?;
        assert!(keys.contains(&id));
        assert_eq!(keys.len(), index + 1);
    }

    for (index, _) in chunks.data_and_sizes.iter().enumerate() {
        let id = Id(index as u64);

        assert!(data_store.keys().await?.contains(&id));
        data_store.delete(&id).await?;

        let keys = data_store.keys().await?;
        assert!(!keys.contains(&id));
        assert_eq!(keys.len(), chunks.data_and_sizes.len() - index - 1);
    }

    Ok(())
}
