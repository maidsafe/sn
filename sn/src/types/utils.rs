// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// https://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use super::errors::convert_bincode_error;
use super::{Error, Result};
use crate::prefix_map::NetworkPrefixMap;
use bytes::Bytes;
use multibase::{self, Base};
use rand::rngs::OsRng;
use rand::Rng;
use rayon::current_num_threads;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Wrapper for raw bincode::serialise.
pub fn serialise<T: Serialize>(data: &T) -> Result<Vec<u8>> {
    bincode::serialize(data).map_err(convert_bincode_error)
}

/// Wrapper for bincode::deserialize.
pub(crate) fn deserialise<'a, T>(bytes: &'a [u8]) -> Result<T>
where
    T: Deserialize<'a>,
{
    bincode::deserialize(bytes).map_err(convert_bincode_error)
}

/// Wrapper for z-Base-32 multibase::encode.
pub(crate) fn encode<T: Serialize>(data: &T) -> Result<String> {
    let bytes = serialise(&data)?;
    Ok(multibase::encode(Base::Base32Z, &bytes))
}

/// Wrapper for z-Base-32 multibase::decode.
pub(crate) fn decode<I: AsRef<str>, O: DeserializeOwned>(encoded: I) -> Result<O> {
    let (base, decoded) =
        multibase::decode(encoded).map_err(|e| Error::FailedToParse(e.to_string()))?;
    if base != Base::Base32Z {
        return Err(Error::FailedToParse(format!(
            "Expected z-base-32 encoding, but got {:?}",
            base
        )));
    }
    deserialise(&decoded).map_err(|e| Error::FailedToParse(e.to_string()))
}

pub(crate) async fn write_data_to_disk<T: Serialize>(data: &T, path: &Path) -> Result<()> {
    trace!("writing data to disk {:?}", path);
    let serialized = rmp_serde::to_vec(data).map_err(|e| Error::Serialisation(e.to_string()))?;

    let mut file = File::create(path)
        .await
        .map_err(|e| Error::FileHandling(e.to_string()))?;

    let _ = file
        .write_all(&serialized)
        .await
        .map_err(|e| Error::FileHandling(e.to_string()))?;

    file.sync_all()
        .await
        .map_err(|e| Error::FileHandling(e.to_string()))?;

    Ok(())
}

pub(crate) async fn read_prefix_map_from_disk(path: &Path) -> Result<NetworkPrefixMap> {
    // Read NetworkPrefixMap from disk if present else create a new one
    match File::open(path).await {
        Ok(mut prefix_map_file) => {
            let mut prefix_map_contents = vec![];
            let _ = prefix_map_file
                .read_to_end(&mut prefix_map_contents)
                .await
                .map_err(|err| {
                    Error::FileHandling(format!(
                        "Error reading PrefixMap from {:?}: {:?}",
                        path, err
                    ))
                })?;

            let prefix_map: NetworkPrefixMap = rmp_serde::from_slice(&prefix_map_contents)
                .map_err(|err| {
                    Error::FileHandling(format!(
                        "Error deserializing PrefixMap from disk: {:?}",
                        err
                    ))
                })?;
            Ok(prefix_map)
        }
        Err(e) => Err(Error::FailedToParse(e.to_string())),
    }
}

/// Easily create a `BTreeSet`.
#[macro_export]
macro_rules! btree_set {
    ($($item:expr),*) => {{
        let mut _set = ::std::collections::BTreeSet::new();
        $(
            let _prev = _set.insert($item);
        )*
        _set
    }};

    ($($item:expr),*,) => {
        btree_set![$($item),*]
    };
}

/// Easily create a `BTreeMap` with the key => value syntax.
#[macro_export]
macro_rules! btree_map {
    () => ({
        ::std::collections::BTreeMap::new()
    });

    ($($key:expr => $value:expr),*) => {{
        let mut _map = ::std::collections::BTreeMap::new();
        $(
            let _prev = _map.insert($key, $value);
        )*
        _map
    }};

    ($($key:expr => $value:expr),*,) => {
        btree_map![$($key => $value),*]
    };
}

/// Generates a random vector using provided `length`.
pub fn random_bytes(length: usize) -> Bytes {
    use rayon::prelude::*;
    let threads = current_num_threads();

    if threads > length {
        let mut rng = OsRng;
        return ::std::iter::repeat(())
            .map(|()| rng.gen::<u8>())
            .take(length)
            .collect();
    }

    let per_thread = length / threads;
    let remainder = length % threads;

    let mut bytes: Vec<u8> = (0..threads)
        .par_bridge()
        .map(|_| vec![0u8; per_thread])
        .map(|mut bytes| {
            let bytes = bytes.as_mut_slice();
            rand::thread_rng().fill(bytes);
            bytes.to_owned()
        })
        .flatten()
        .collect();

    bytes.extend(vec![0u8; remainder]);

    Bytes::from(bytes)
}
