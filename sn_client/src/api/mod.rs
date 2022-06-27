// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod cmds;
mod data;
mod file_apis;
mod queries;
mod register_apis;
mod spentbook_apis;

pub use register_apis::RegisterWriteAheadLog;

use crate::{connections::Session, errors::Error, ClientConfig};

use sn_dbc::{rng, Owner};
use sn_interface::{
    messaging::{
        data::{CmdError, DataQuery, DataQueryVariant, RegisterQuery, ServiceMsg},
        ServiceAuth, WireMsg,
    },
    network_knowledge::{prefix_map::NetworkPrefixMap, utils::read_prefix_map_from_disk},
    types::{Chunk, Keypair, Peer, PublicKey, RegisterAddress},
};

use bytes::Bytes;
use itertools::Itertools;
use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};
use tokio::{
    sync::{mpsc::Receiver, RwLock},
    time::Duration,
};
use tracing::{debug, info};
use uluru::LRUCache;
use xor_name::XorName;

// Maximum amount of Chunks to keep in our cal Chunks cache.
// Each Chunk is maximum types::MAX_CHUNK_SIZE_IN_BYTES, i.e. ~1MB
const CHUNK_CACHE_SIZE: usize = 50;

// Number of times to retry network probe on client startup
const NETWORK_PROBE_RETRY_COUNT: usize = 5; // 5 x 5 second wait in between = ~25 seconds (plus ~ 3 seconds in between attempts internal to `make_contact`)

// LRU cache to keep the Chunks we retrieve.
type ChunksCache = LRUCache<Chunk, CHUNK_CACHE_SIZE>;

/// Client object
#[derive(Clone, Debug)]
pub struct Client {
    keypair: Keypair,
    dbc_owner: Owner,
    #[allow(dead_code)]
    incoming_errors: Arc<RwLock<Receiver<CmdError>>>,
    session: Session,
    pub(crate) query_timeout: Duration,
    pub(crate) cmd_timeout: Duration,
    chunks_cache: Arc<RwLock<ChunksCache>>,
}

/// Easily manage connections to/from The Safe Network with the client and its APIs.
/// Use a random client for read-only or one-time operations.
/// Supply an existing, SecretKey which holds a SafeCoin balance to be able to perform
/// write operations.
impl Client {
    /// Create a Safe Network client instance. Either for an existing SecretKey (in which case) the client will attempt
    /// to retrieve the history of the key's balance in order to be ready for any token operations. Or if no SecreteKey
    /// is passed, a random keypair will be used, which provides a client that can only perform Read operations (at
    /// least until the client's SecretKey receives some token).
    ///
    /// # Examples
    ///
    /// TODO: update once data types are crdt compliant
    ///
    ///
    #[instrument(skip_all, level = "debug", name = "New client")]
    pub async fn new(
        config: ClientConfig,
        bootstrap_nodes: BTreeSet<SocketAddr>,
        optional_keypair: Option<Keypair>,
        dbc_owner: Option<Owner>,
    ) -> Result<Self, Error> {
        Client::create_with(config, bootstrap_nodes, optional_keypair, dbc_owner, true).await
    }

    #[instrument]
    pub(crate) async fn create_with(
        config: ClientConfig,
        bootstrap_nodes: BTreeSet<SocketAddr>,
        optional_keypair: Option<Keypair>,
        dbc_owner: Option<Owner>,
        read_prefixmap: bool,
    ) -> Result<Self, Error> {
        let keypair = match optional_keypair {
            Some(id) => {
                info!("Client started for specific pk: {:?}", id.public_key());
                id
            }
            None => {
                let keypair = Keypair::new_ed25519();
                info!(
                    "Client started for new randomly created pk: {:?}",
                    keypair.public_key()
                );
                keypair
            }
        };

        let home_dir = dirs_next::home_dir().ok_or(Error::CouldNotReadHomeDir)?;

        // Read NetworkPrefixMap from `.safe/prefix_map` if present else check client root dir
        let prefix_map = if read_prefixmap {
            match read_prefix_map_from_disk(
                &home_dir.join(format!(".safe/prefix_maps/{:?}", config.genesis_key)),
            )
            .await
            {
                Ok(prefix_map) => prefix_map,
                Err(e) => {
                    warn!("Could not read PrefixMap at '.safe/prefix_maps': {:?}", e);
                    info!(
                        "Creating a fresh PrefixMap with GenesisKey {:?}",
                        config.genesis_key
                    );
                    NetworkPrefixMap::new(config.genesis_key)
                }
            }
        } else {
            NetworkPrefixMap::new(config.genesis_key)
        };

        if config.genesis_key != prefix_map.genesis_key() {
            return Err(Error::GenesisKeyMismatch);
        }

        // Incoming error notifiers
        let (err_sender, err_receiver) = tokio::sync::mpsc::channel::<CmdError>(10);

        let client_pk = keypair.public_key();

        // Bootstrap to the network, connecting to a section based
        // on a public key of our choice.
        debug!(
            "Creating new session with genesis key: {:?} ",
            config.genesis_key
        );
        debug!(
            "Creating new session with genesis key (in hex format): {} ",
            hex::encode(config.genesis_key.to_bytes())
        );

        // Create a session with the network
        let session = Session::new(
            config.genesis_key,
            config.qp2p,
            err_sender,
            config.local_addr,
            config.cmd_ack_wait,
            prefix_map.clone(),
        )?;

        let client = Self {
            keypair,
            dbc_owner: dbc_owner
                .unwrap_or_else(|| Owner::from_random_secret_key(&mut rng::thread_rng())),
            session,
            incoming_errors: Arc::new(RwLock::new(err_receiver)),
            query_timeout: config.query_timeout,
            cmd_timeout: config.cmd_timeout,
            chunks_cache: Arc::new(RwLock::new(ChunksCache::default())),
        };

        // TODO: The message being sent below is a temporary solution to fetch network info for
        // the client. Ideally the client should be able to send proper AE-Probe messages to the
        // trigger the AE flows.

        fn generate_probe_msg(
            client: &Client,
            pk: PublicKey,
        ) -> Result<(XorName, ServiceAuth, Bytes), Error> {
            // Generate a random query to send a dummy message
            let random_dst_addr = xor_name::rand::random();
            let serialised_cmd = {
                let msg = ServiceMsg::Query(DataQuery {
                    adult_index: 0,
                    variant: DataQueryVariant::Register(RegisterQuery::Get(RegisterAddress {
                        name: random_dst_addr,
                        tag: 1,
                    })),
                });
                WireMsg::serialize_msg_payload(&msg)?
            };
            let signature = client.keypair.sign(&serialised_cmd);
            let auth = ServiceAuth {
                public_key: pk,
                signature,
            };

            Ok((random_dst_addr, auth, serialised_cmd))
        }

        let (random_dst_addr, auth, serialised_cmd) = generate_probe_msg(&client, client_pk)?;

        // either use our known prefixmap elders, or fallback to plain node config file
        let bootstrap_nodes = {
            if let Some(sap) = prefix_map.closest_or_opposite(&random_dst_addr, None) {
                sap.elders_vec()
            } else {
                // these peers will be nonsense peers, and dropped after we connect. Replaced by whatever SectionAuthorityProvider peers we have received
                // therefore we use a random name for them initially
                bootstrap_nodes
                    .iter()
                    .copied()
                    .map(|socket| Peer::new(xor_name::rand::random(), socket))
                    .collect_vec()
            }
        };

        let mut attempts = 0;
        let mut initial_probe = client
            .session
            .make_contact_with_nodes(
                bootstrap_nodes.clone(),
                random_dst_addr,
                auth.clone(),
                serialised_cmd,
            )
            .await;
        // Send the dummy message to probe the network for it's infrastructure details.
        while attempts < NETWORK_PROBE_RETRY_COUNT && initial_probe.is_err() {
            warn!(
                "Initial probe msg to network failed. Trying again (attempt {}): {:?}",
                attempts, initial_probe
            );

            if attempts == NETWORK_PROBE_RETRY_COUNT {
                // we've failed
                return Err(Error::NetworkContact);
            }

            attempts += 1;

            tokio::time::sleep(Duration::from_secs(5)).await;

            let (random_dst_addr, auth, serialised_cmd) = generate_probe_msg(&client, client_pk)?;

            initial_probe = client
                .session
                .make_contact_with_nodes(
                    bootstrap_nodes.clone(),
                    random_dst_addr,
                    auth,
                    serialised_cmd,
                )
                .await;
        }

        Ok(client)
    }

    /// Return the client's keypair.
    ///
    /// Useful for retrieving the PublicKey or KeyPair in the event you need to _sign_ something
    ///
    /// # Examples
    ///
    /// TODO: update once data types are crdt compliant
    ///
    pub fn keypair(&self) -> Keypair {
        self.keypair.clone()
    }

    /// Return the client's PublicKey.
    ///
    /// # Examples
    ///
    /// TODO: update once data types are crdt compliant
    ///
    pub fn public_key(&self) -> PublicKey {
        self.keypair().public_key()
    }

    /// Return the client's DBC owner, which will be a secret key.
    ///
    /// This can then be used to sign output DBCs during a DBC reissue.
    pub fn dbc_owner(&self) -> Owner {
        self.dbc_owner.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{
        create_test_client, create_test_client_with, get_dbc_owner_from_secret_key_hex,
    };
    use sn_interface::{init_logger, types::utils::random_bytes};

    use eyre::Result;
    use std::{
        collections::HashSet,
        net::{IpAddr, Ipv4Addr, SocketAddr},
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn client_creation() -> Result<()> {
        init_logger();
        let _client = create_test_client().await?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn client_nonsense_bootstrap_fails() -> Result<()> {
        init_logger();

        let mut nonsense_bootstrap = HashSet::new();
        let _ = nonsense_bootstrap.insert(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            3033,
        ));
        //let setup = create_test_client_with(None, Some(nonsense_bootstrap)).await;
        //assert!(setup.is_err());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_creation_with_existing_keypair() -> Result<()> {
        init_logger();

        let full_id = Keypair::new_ed25519();
        let pk = full_id.public_key();

        let client = create_test_client_with(Some(full_id), None, None, true).await?;
        assert_eq!(pk, client.public_key());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn long_lived_connection_survives() -> Result<()> {
        init_logger();

        let client = create_test_client().await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(40)).await;
        let bytes = random_bytes(self_encryption::MIN_ENCRYPTABLE_BYTES / 2);
        let _ = client.upload(bytes).await?;
        Ok(())
    }

    // Send is an important trait that assures futures can be run in a
    // multithreaded context. If a future depends on a non-Send future, directly
    // or indirectly, the future itself becomes non-Send and so on. Thus, it can
    // happen that high-level API functions will become non-Send by accident.
    #[test]
    fn client_is_send() {
        init_logger();

        fn require_send<T: Send>(_t: T) {}
        require_send(create_test_client());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_create_with_dbc_owner() -> Result<()> {
        init_logger();
        let dbc_owner = get_dbc_owner_from_secret_key_hex(
            "81ebce8339cb2a6e5cbf8b748215ba928acff7f92557b3acfb09a5b25e920d20",
        )
        .await?;

        let client = create_test_client_with(None, Some(dbc_owner.clone()), None, true).await?;
        assert_eq!(dbc_owner, client.dbc_owner());
        Ok(())
    }
}
