// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{to_db_key::ToDbKey, utils, Error, Result};
use pickledb::PickleDb;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fmt::Debug,
    marker::PhantomData,
    path::{Path, PathBuf},
};
use xor_name::XorName;

const TRANSFERS_DIR_NAME: &str = "transfers";
const DB_EXTENSION: &str = ".db";

/// Disk storage for transfers.
pub struct TransferStore<TEvent: Debug + Serialize + DeserializeOwned> {
    db: PickleDb,
    db_path: PathBuf,
    _phantom: PhantomData<TEvent>,
}

pub struct DeletableStore {
    db_path: PathBuf,
}

impl DeletableStore {
    pub fn delete(&self) -> Result<()> {
        std::fs::remove_file(self.db_path.as_path()).map_err(Error::Io)
    }
}

impl<'a, TEvent: Debug + Serialize + DeserializeOwned> TransferStore<TEvent>
where
    TEvent: 'a,
{
    pub fn new(id: XorName, root_dir: &Path) -> Result<Self> {
        let db_dir = root_dir.join(Path::new(TRANSFERS_DIR_NAME));
        let db_name = format!("{}{}", id.to_db_key()?, DB_EXTENSION);
        let db_path = db_dir.join(db_name.clone());
        Ok(Self {
            db: utils::new_auto_dump_db(db_dir.as_path(), db_name)?,
            db_path,
            _phantom: PhantomData::default(),
        })
    }

    pub fn as_deletable(&self) -> DeletableStore {
        DeletableStore {
            db_path: self.db_path.clone(),
        }
    }

    ///
    pub fn get_all(&self) -> Vec<TEvent> {
        let keys = self.db.get_all();

        let mut events: Vec<(usize, TEvent)> = keys
            .iter()
            .filter_map(|key| {
                let value = self.db.get::<TEvent>(key);
                let key = key.parse::<usize>();
                match value {
                    Some(v) => match key {
                        Ok(k) => Some((k, v)),
                        _ => None,
                    },
                    None => None,
                }
            })
            .collect();

        events.sort_by(|(key_a, _), (key_b, _)| key_a.partial_cmp(key_b).unwrap());

        let events: Vec<TEvent> = events.into_iter().map(|(_, val)| val).collect();

        events
    }

    ///
    pub fn try_insert(&mut self, event: TEvent) -> Result<()> {
        let key = &self.db.total_keys().to_string();
        if self.db.exists(key) {
            return Err(Error::Logic(format!(
                "Key exists: {}. Event: {:?}",
                key, event
            )));
        }
        self.db.set(key, &event).map_err(Error::PickleDb)
    }
}

#[cfg(test)]
mod test {
    use super::TransferStore;
    use crate::node::{Error, Result};
    use crate::types::{
        Credit, CreditAgreementProof, CreditId, PublicKey, ReplicaEvent, SignedCredit, Token,
        TransferPropagated,
    };
    use bls::SecretKeySet;
    use bls::{PublicKeySet, SecretKey, SecretKeyShare};
    use std::collections::BTreeMap;
    use tempdir::TempDir;

    #[test]
    fn history() -> Result<()> {
        let id = xor_name::XorName::random();
        let tmp_dir = TempDir::new("root")?;
        let root_dir = tmp_dir.into_path();
        let mut store = TransferStore::new(id, &root_dir)?;
        let wallet_id = get_random_pk();
        let mut rng = rand::thread_rng();
        let bls_secret_key = SecretKeySet::random(0, &mut rng);
        let genesis_credit_proof = get_credit(
            10,
            wallet_id,
            bls_secret_key.public_keys(),
            bls_secret_key.secret_key_share(0),
        )?;
        store.try_insert(ReplicaEvent::TransferPropagated(TransferPropagated {
            credit_proof: genesis_credit_proof.clone(),
        }))?;

        let events = store.get_all();
        assert_eq!(events.len(), 1);

        match &events[0] {
            ReplicaEvent::TransferPropagated(TransferPropagated { credit_proof, .. }) => {
                assert_eq!(credit_proof, &genesis_credit_proof)
            }
            other => {
                return Err(Error::Logic(format!(
                    "Incorrect Replica event: {:?}",
                    other
                )))
            }
        }

        Ok(())
    }

    fn get_random_pk() -> PublicKey {
        PublicKey::from(SecretKey::random().public_key())
    }

    /// Produces a genesis balance for a new network.
    fn get_credit(
        balance: u64,
        id: PublicKey,
        peer_replicas: PublicKeySet,
        secret_key_share: SecretKeyShare,
    ) -> Result<CreditAgreementProof> {
        let credit = Credit {
            id: CreditId::default(),
            amount: Token::from_nano(balance),
            recipient: id,
            msg: "genesis".to_string(),
        };

        // actor instances' signatures over > credit <

        let serialised_credit = bincode::serialize(&credit).map_err(Error::Bincode)?;

        let mut credit_sig_shares = BTreeMap::new();
        let credit_sig_share = secret_key_share.sign(serialised_credit);
        let _ = credit_sig_shares.insert(0, credit_sig_share);

        println!("Aggregating actor signature..");

        // Combine shares to produce the main signature.
        let actor_signature = crate::types::Signature::Bls(
            peer_replicas
                .combine_signatures(&credit_sig_shares)
                .map_err(|_| Error::CouldNotCombineSignatures)?,
        );

        let signed_credit = SignedCredit {
            credit,
            actor_signature,
        };

        // replicas signatures over > signed_credit <

        let serialised_credit = bincode::serialize(&signed_credit).map_err(Error::Bincode)?;

        let mut credit_sig_shares = BTreeMap::new();
        let credit_sig_share = secret_key_share.sign(serialised_credit);
        let _ = credit_sig_shares.insert(0, credit_sig_share);

        println!("Aggregating replica signature..");

        let debiting_replicas_sig = crate::types::Signature::Bls(
            peer_replicas
                .combine_signatures(&credit_sig_shares)
                .map_err(|_| Error::CouldNotCombineSignatures)?,
        );

        Ok(CreditAgreementProof {
            signed_credit,
            debiting_replicas_sig,
            debiting_replicas_keys: peer_replicas,
        })
    }
}
