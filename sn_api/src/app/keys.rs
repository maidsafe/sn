// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Safe;
use crate::{Error, Result, SafeUrl};

use bls::SecretKey as BlsSecretKey;
use sn_interface::types::{Keypair, SecretKey};

use hex::encode;
use std::path::Path;
use xor_name::XorName;

impl Safe {
    /// Check the XOR/NRS-URL corresponds to the public key derived from the provided client id.
    pub async fn validate_sk_for_url(&self, secret_key: &SecretKey, url: &str) -> Result<String> {
        let derived_xorname = match secret_key {
            SecretKey::Ed25519(sk) => {
                let pk: ed25519_dalek::PublicKey = sk.into();
                XorName(pk.to_bytes())
            }
            _ => {
                return Err(Error::InvalidInput(
                    "Cannot form a keypair from a BlsKeyShare at this time.".to_string(),
                ))
            }
        };

        let safeurl = self.parse_and_resolve_url(url).await?;
        if safeurl.xorname() != derived_xorname {
            Err(Error::InvalidInput(
                "The URL doesn't correspond to the public key derived from the provided secret key"
                    .to_string(),
            ))
        } else {
            Ok(encode(&derived_xorname))
        }
    }

    /// Generate a new random BLS keypair along with a URL for the public key.
    pub fn new_keypair_with_pk_url(&self) -> Result<(Keypair, SafeUrl)> {
        let keypair = Keypair::new_bls();
        let xorname = XorName::from(keypair.public_key());
        let url = SafeUrl::from_safekey(xorname)?;
        Ok((keypair, url))
    }

    /// Serializes a `SecretKey` to hex in a file at a given path.
    ///
    /// If the path already exists it will be overwritten.
    pub fn serialize_bls_key(
        &self,
        secret_key: &BlsSecretKey,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let hex = secret_key.to_hex();
        std::fs::write(&path, hex)?;
        Ok(())
    }

    /// Deserializes a `Keypair` from file at a given path.
    ///
    /// A utility to help callers working with keypairs avoid using serde or bincode directly.
    pub fn deserialize_bls_key(&self, path: impl AsRef<Path>) -> Result<BlsSecretKey> {
        deserialize_bls_key(path)
    }
}

/// Deserializes a `Keypair` from file at a given path.
///
/// A utility to help callers working with keypairs avoid using serde or bincode directly.
///
/// This exists as an independent function in addition to being a function of the safe client
/// because some deserialization needs to be performed in the CLI before it has access to a client.
pub fn deserialize_bls_key(path: impl AsRef<Path>) -> Result<BlsSecretKey> {
    let hex = std::fs::read_to_string(path)?;
    Ok(BlsSecretKey::from_hex(&hex)?)
}

#[cfg(test)]
mod tests {
    use super::{Safe, SafeUrl};
    use sn_interface::types::Keypair;

    use assert_fs::prelude::*;
    use bls::SecretKey as BlsSecretKey;
    use color_eyre::{eyre::eyre, Result};
    use predicates::prelude::*;
    use xor_name::XorName;

    #[test]
    fn new_keypair_should_generate_bls_keypair() -> Result<()> {
        let safe = Safe::dry_runner(None);
        let (keypair, url) = safe.new_keypair_with_pk_url()?;

        // Make sure the URL points to the same public key in the keypair.
        // This may seem a silly check, but it would be possible for that returned URL to point to
        // anything.
        let xorname = XorName::from(keypair.public_key());
        let url2 = SafeUrl::from_safekey(xorname)?;
        assert_eq!(url, url2);

        match keypair {
            Keypair::Bls(_) => Ok(()),
            _ => Err(eyre!("A BLS keypair should be generated by default.")),
        }
    }

    #[test]
    fn serialize_keypair_should_serialize_a_bls_keypair_to_file() -> Result<()> {
        let safe = Safe::dry_runner(None);
        let tmp_dir = assert_fs::TempDir::new()?;
        let serialized_keypair_file = tmp_dir.child("serialized_keypair");

        let sk = BlsSecretKey::random();
        let _ = safe.serialize_bls_key(&sk, serialized_keypair_file.path())?;

        serialized_keypair_file.assert(predicate::path::is_file());

        let sk_hex = std::fs::read_to_string(serialized_keypair_file.path())?;
        let sk2 = BlsSecretKey::from_hex(&sk_hex)?;
        assert_eq!(sk, sk2);
        Ok(())
    }

    #[test]
    fn deserialize_keypair_should_deserialize_a_bls_keypair_from_file() -> Result<()> {
        let safe = Safe::dry_runner(None);
        let tmp_dir = assert_fs::TempDir::new()?;
        let serialized_keypair_file = tmp_dir.child("serialized_keypair");

        let sk = BlsSecretKey::random();
        let _ = safe.serialize_bls_key(&sk, serialized_keypair_file.path())?;

        let sk2 = safe.deserialize_bls_key(serialized_keypair_file.path())?;
        assert_eq!(sk, sk2);
        Ok(())
    }
}
