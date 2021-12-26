// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use super::register::EntryHash;
use crate::safeurl::{ContentType, SafeUrl, XorUrl};
use crate::{Error, Result, Safe, Scope};
use log::debug;
use safe_network::types::DataAddress;
use std::collections::BTreeSet;
use xor_name::XorName;

pub type MultimapKey = Vec<u8>;
pub type MultimapValue = Vec<u8>;
pub type MultimapKeyValue = (MultimapKey, MultimapValue);
pub type MultimapKeyValues = BTreeSet<(EntryHash, MultimapKeyValue)>;

const MULTIMAP_REMOVED_MARK: &[u8] = b"";

impl Safe {
    /// Create a Multimap on the network
    pub async fn multimap_create(
        &self,
        name: Option<XorName>,
        type_tag: u64,
        private: bool,
        dry_run: bool,
    ) -> Result<XorUrl> {
        debug!("Creating a Multimap");
        let (xorname, op_batch) = self
            .safe_client
            .create_register(name, type_tag, None, private, dry_run)
            .await?;

        if !dry_run {
            self.safe_client.apply_register_ops(op_batch).await?;
        }

        let scope = if private {
            Scope::Private
        } else {
            Scope::Public
        };

        let xorurl = SafeUrl::encode_register(
            xorname,
            type_tag,
            scope,
            ContentType::Multimap,
            self.xorurl_base,
        )?;

        Ok(xorurl)
    }

    /// Return the value of a Multimap on the network corresponding to the key provided
    pub async fn multimap_get_by_key(&self, url: &str, key: &[u8]) -> Result<MultimapKeyValues> {
        debug!("Getting value by key from Multimap at: {}", url);
        let safeurl = self.parse_and_resolve_url(url).await?;

        self.fetch_multimap_value_by_key(&safeurl, key).await
    }

    /// Return the value of a Multimap on the network corresponding to the hash provided
    pub async fn multimap_get_by_hash(
        &self,
        url: &str,
        hash: EntryHash,
    ) -> Result<MultimapKeyValue> {
        debug!("Getting value by hash from Multimap at: {}", url);
        let safeurl = self.parse_and_resolve_url(url).await?;

        self.fetch_multimap_value_by_hash(&safeurl, hash).await
    }

    // Return the value (by a provided key) of a Multimap on
    // the network without resolving the SafeUrl
    pub(crate) async fn fetch_multimap_value_by_key(
        &self,
        safeurl: &SafeUrl,
        key: &[u8],
    ) -> Result<MultimapKeyValues> {
        let entries = self.fetch_multimap_values(safeurl).await?;
        Ok(entries
            .into_iter()
            .filter(|(_, (entry_key, _))| entry_key == key)
            .collect())
    }

    /// Insert a key-value pair into a Multimap on the network
    pub async fn multimap_insert(
        &self,
        multimap_url: &str,
        entry: MultimapKeyValue,
        replace: BTreeSet<EntryHash>,
        dry_run: bool,
    ) -> Result<EntryHash> {
        debug!("Inserting '{:?}' into Multimap at {}", entry, multimap_url);
        let serialised_entry = rmp_serde::to_vec_named(&entry).map_err(|err| {
            Error::Serialisation(format!(
                "Couldn't serialise the Multimap entry '{:?}': {:?}",
                entry, err
            ))
        })?;

        let data = serialised_entry.to_vec();
        let safeurl = Safe::parse_url(multimap_url)?;
        let address = match safeurl.address() {
            DataAddress::Register(reg_address) => reg_address,
            other => {
                return Err(Error::InvalidXorUrl(format!(
                    "The multimap url {} has an {:?} address.\
                    To insert an entry into a multimap, the address must be a register address.",
                    multimap_url, other
                )))
            }
        };

        let (entry_hash, op_batch) = self
            .safe_client
            .write_to_register(address, data, replace)
            .await?;

        if !dry_run {
            self.safe_client.apply_register_ops(op_batch).await?;
        }

        Ok(entry_hash)
    }

    /// Remove entries from a Multimap on the network
    /// This tombstones the removed entries, effectively hiding them if they where the latest
    /// Note that they are still stored on the network as history is kept,
    /// and you can still access them with their EntryHash
    pub async fn multimap_remove(
        &self,
        url: &str,
        to_remove: BTreeSet<EntryHash>,
        dry_run: bool,
    ) -> Result<EntryHash> {
        debug!("Removing from Multimap at {}: {:?}", url, to_remove);
        let safeurl = Safe::parse_url(url)?;
        let address = match safeurl.address() {
            DataAddress::Register(reg_address) => reg_address,
            other => {
                return Err(Error::InvalidXorUrl(format!(
                    "The multimap url {} has an {:?} address.\
                    To remove an entry from a multimap, the address must be a register address.",
                    url, other
                )))
            }
        };
        let (entry_hash, op_batch) = self
            .safe_client
            .write_to_register(address, MULTIMAP_REMOVED_MARK.to_vec(), to_remove)
            .await?;

        if !dry_run {
            self.safe_client.apply_register_ops(op_batch).await?;
        }

        Ok(entry_hash)
    }

    // Crate's helper to return the value of a Multimap on
    // the network without resolving the SafeUrl,
    // filtering by hash if a version is provided
    pub(crate) async fn fetch_multimap_values(
        &self,
        safeurl: &SafeUrl,
    ) -> Result<MultimapKeyValues> {
        let entries = match self.register_fetch_entries(safeurl).await {
            Ok(data) => {
                debug!("Multimap retrieved...");
                Ok(data)
            }
            Err(Error::EmptyContent(_)) => Err(Error::EmptyContent(format!(
                "Multimap found at \"{}\" was empty",
                safeurl.to_string()
            ))),
            Err(Error::ContentNotFound(_)) => Err(Error::ContentNotFound(format!(
                "No Multimap found at \"{}\"",
                safeurl.to_string()
            ))),
            other => other,
        }?;

        // We parse each entry in the Register as a 'MultimapKeyValue'
        let mut multimap_key_vals = MultimapKeyValues::new();
        for (hash, entry) in entries.iter() {
            if entry == MULTIMAP_REMOVED_MARK {
                // this is a tombstone entry created to delete some old entries
                continue;
            }
            let key_val = Self::decode_multimap_entry(entry)?;
            multimap_key_vals.insert((*hash, key_val));
        }
        Ok(multimap_key_vals)
    }

    // Crate's helper to return the value of a Multimap on
    // the network without resolving the SafeUrl,
    // optionally filtering by hash and/or key.
    pub(crate) async fn fetch_multimap_value_by_hash(
        &self,
        safeurl: &SafeUrl,
        hash: EntryHash,
    ) -> Result<MultimapKeyValue> {
        let entry = match self.register_fetch_entry(safeurl, hash).await {
            Ok(data) => {
                debug!("Multimap retrieved...");
                Ok(data)
            }
            Err(Error::EmptyContent(_)) => Err(Error::EmptyContent(format!(
                "Multimap found at \"{}\" was empty",
                safeurl
            ))),
            Err(Error::ContentNotFound(_)) => Err(Error::ContentNotFound(
                "No Multimap found at this address".to_string(),
            )),
            Err(other) => Err(other),
        }?;

        // We parse the entry in the Register as a 'MultimapKeyValue'
        if entry == MULTIMAP_REMOVED_MARK {
            Err(Error::EmptyContent(format!(
                "Entry found at \"{}\" is a tombstone (deletion marker)",
                safeurl
            )))
        } else {
            let key_val = Self::decode_multimap_entry(&entry)?;
            Ok(key_val)
        }
    }

    fn decode_multimap_entry(entry: &[u8]) -> Result<MultimapKeyValue> {
        rmp_serde::from_slice(entry)
            .map_err(|err| Error::ContentError(format!("Couldn't parse Multimap entry: {:?}", err)))
    }
}

#[cfg(test)]
mod tests {
    use crate::{app::test_helpers::new_safe_instance, retry_loop, retry_loop_for_pattern};
    use anyhow::Result;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn test_multimap_create() -> Result<()> {
        let safe = new_safe_instance().await?;

        let xorurl = safe.multimap_create(None, 25_000, false, false).await?;
        let xorurl_priv = safe.multimap_create(None, 25_000, true, false).await?;

        let key = b"".to_vec();
        let received_data = safe.multimap_get_by_key(&xorurl, &key).await?;
        let received_data_priv = safe.multimap_get_by_key(&xorurl_priv, &key).await?;

        assert_eq!(received_data, Default::default());
        assert_eq!(received_data_priv, Default::default());

        Ok(())
    }

    #[tokio::test]
    async fn test_multimap_insert() -> Result<()> {
        let safe = new_safe_instance().await?;
        let key = b"key".to_vec();
        let val = b"value".to_vec();
        let key_val = (key.clone(), val.clone());

        let val2 = b"value2".to_vec();
        let key_val2 = (key.clone(), val2.clone());

        let xorurl = safe.multimap_create(None, 25_000, false, false).await?;
        let xorurl_priv = safe.multimap_create(None, 25_000, true, false).await?;

        let _ = safe.multimap_get_by_key(&xorurl, &key).await?;
        let _ = safe.multimap_get_by_key(&xorurl_priv, &key).await?;

        let hash = safe
            .multimap_insert(&xorurl, key_val.clone(), BTreeSet::new(), false)
            .await?;
        let hash_priv = safe
            .multimap_insert(&xorurl_priv, key_val.clone(), BTreeSet::new(), false)
            .await?;

        let received_data = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl, &key), Ok(v) if !v.is_empty())?;
        let received_data_priv = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl_priv, &key), Ok(v) if !v.is_empty())?;

        assert_eq!(
            received_data,
            vec![(hash, key_val.clone())].into_iter().collect()
        );
        assert_eq!(
            received_data_priv,
            vec![(hash_priv, key_val.clone())].into_iter().collect()
        );

        // Let's now test an insert which replace the previous value for a key
        let hashes_to_replace = vec![hash].into_iter().collect();
        let hash2 = safe
            .multimap_insert(&xorurl, key_val2.clone(), hashes_to_replace, false)
            .await?;
        let hashes_priv_to_replace = vec![hash_priv].into_iter().collect();
        let hash_priv2 = safe
            .multimap_insert(
                &xorurl_priv,
                key_val2.clone(),
                hashes_priv_to_replace,
                false,
            )
            .await?;

        let received_data = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl, &key),
                                                    Ok(v) if v.iter().all(|(_, kv)| *kv != key_val))?;
        let received_data_priv = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl_priv, &key),
                                                         Ok(v) if v.iter().all(|(_, kv)| *kv != key_val))?;

        assert_eq!(
            received_data,
            vec![(hash2, key_val2.clone())].into_iter().collect()
        );
        assert_eq!(
            received_data_priv,
            vec![(hash_priv2, key_val2.clone())].into_iter().collect()
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_multimap_get_by_hash() -> Result<()> {
        let safe = new_safe_instance().await?;
        let key = b"key".to_vec();
        let val = b"value".to_vec();
        let key_val = (key.clone(), val.clone());
        let key2 = b"key2".to_vec();
        let val2 = b"value2".to_vec();
        let key_val2 = (key2.clone(), val2.clone());

        let xorurl = safe.multimap_create(None, 25_000, false, false).await?;
        let xorurl_priv = safe.multimap_create(None, 25_000, true, false).await?;

        let _ = safe.multimap_get_by_key(&xorurl, &key).await?;
        let _ = safe.multimap_get_by_key(&xorurl_priv, &key).await?;

        let hash = safe
            .multimap_insert(&xorurl, key_val.clone(), BTreeSet::new(), false)
            .await?;
        let hash2 = safe
            .multimap_insert(&xorurl, key_val2.clone(), BTreeSet::new(), false)
            .await?;

        let hash_priv = safe
            .multimap_insert(&xorurl_priv, key_val.clone(), BTreeSet::new(), false)
            .await?;
        let hash_priv2 = safe
            .multimap_insert(&xorurl_priv, key_val2.clone(), BTreeSet::new(), false)
            .await?;

        let received_data = safe.multimap_get_by_hash(&xorurl, hash).await?;
        let received_data_priv = safe.multimap_get_by_hash(&xorurl_priv, hash_priv).await?;

        assert_eq!(received_data, key_val.clone());
        assert_eq!(received_data_priv, key_val);

        let received_data = safe.multimap_get_by_hash(&xorurl, hash2).await?;
        let received_data_priv = safe.multimap_get_by_hash(&xorurl_priv, hash_priv2).await?;

        assert_eq!(received_data, key_val2.clone());
        assert_eq!(received_data_priv, key_val2);

        Ok(())
    }

    #[tokio::test]
    async fn test_multimap_remove() -> Result<()> {
        let safe = new_safe_instance().await?;
        let key = b"key".to_vec();
        let val = b"value".to_vec();
        let key_val = (key.clone(), val.clone());

        let xorurl = safe.multimap_create(None, 25_000, false, false).await?;
        let xorurl_priv = safe.multimap_create(None, 25_000, true, false).await?;

        let _ = retry_loop!(safe.multimap_get_by_key(&xorurl, &key));
        let _ = retry_loop!(safe.multimap_get_by_key(&xorurl_priv, &key));

        let hash = safe
            .multimap_insert(&xorurl, key_val.clone(), BTreeSet::new(), false)
            .await?;
        let hash_priv = safe
            .multimap_insert(&xorurl_priv, key_val, BTreeSet::new(), false)
            .await?;

        let received_data = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl, &key), Ok(v) if !v.is_empty())?;
        let received_data_priv = retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl_priv, &key), Ok(v) if !v.is_empty())?;

        assert_eq!(received_data.len(), 1);
        let (read_hash, read_key_val) = received_data.into_iter().next().unwrap();
        assert_eq!(
            (read_hash, read_key_val),
            (hash, (key.clone(), val.clone()))
        );

        let hashes_to_remove = vec![hash].into_iter().collect();
        let removed_mark_hash = safe
            .multimap_remove(&xorurl, hashes_to_remove, false)
            .await?;
        assert_ne!(removed_mark_hash, hash);

        assert_eq!(
            retry_loop_for_pattern!(safe.multimap_get_by_key(&xorurl, &key),
                                    Ok(entries) if entries.is_empty())?,
            Default::default()
        );

        assert_eq!(received_data_priv.len(), 1);
        let (read_hash, read_key_val) = received_data_priv.into_iter().next().unwrap();
        assert_eq!((read_hash, read_key_val), (hash_priv, (key.clone(), val)));

        let hashes_to_remove = vec![hash_priv].into_iter().collect();
        let removed_mark_hash = safe
            .multimap_remove(&xorurl_priv, hashes_to_remove, false)
            .await?;
        assert_ne!(removed_mark_hash, hash_priv);

        assert_eq!(
            retry_loop_for_pattern!(
                safe.multimap_get_by_key(&xorurl_priv, &key),
                Ok(entries) if entries.is_empty())?,
            Default::default()
        );

        Ok(())
    }
}
