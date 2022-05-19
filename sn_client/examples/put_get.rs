// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! sn_node provides the interface to Safe routing.  The resulting executable is the node
//! for the Safe network.

use tokio::time::{sleep, Duration, Instant};
use tracing::{debug, warn};

use sn_client::{Client, ClientConfig, Error, Result};
use sn_interface::init_logger;
use sn_interface::types::{utils::random_bytes, BytesAddress, Scope};
use tiny_keccak::{Hasher, Sha3};

use sn_client::utils::test_utils::read_network_conn_info;

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();

    run_chunk_soak().await?;

    Ok(())
}

const TEST_FILES_COUNT: &str = "TEST_FILES_COUNT";
const DEFAULT_FILES_COUNT: usize = 10;

/// Get the expected elder count for our network.
/// Defaults to DEFAULT_ELDER_COUNT, but can be overridden by the env var FILES_COUNT.
pub(crate) fn files_count() -> usize {
    // if we have an env var for this, lets override
    match std::env::var(TEST_FILES_COUNT) {
        Ok(count) => match count.parse() {
            Ok(count) => {
                warn!(
                    "FILES_COUNT countout set from env var TEST_FILES_COUNT: {:?}",
                    TEST_FILES_COUNT
                );
                count
            }
            Err(error) => {
                warn!("There was an error parsing {:?} env var. DEFAULT_FILES_COUNT will be used: {:?}", TEST_FILES_COUNT, error);
                DEFAULT_FILES_COUNT
            }
        },
        Err(_) => DEFAULT_FILES_COUNT,
    }
}

/// uploads data and verifies that data
pub async fn run_chunk_soak() -> Result<()> {
    let all_data_put = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));

    // now we read the data
    let (genesis_key, bootstrap_nodes) =
        read_network_conn_info().map_err(|_e| Error::NoNetworkKnowledge)?;

    debug!("Contacting nodes: {:?}", bootstrap_nodes);

    let files_to_put = files_count();

    let config = ClientConfig::new(None, None, genesis_key, None, None, None, None).await;

    let client = Client::new(config.clone(), bootstrap_nodes.clone(), None).await?;

    let mut put_tasks = vec![];
    // i is used to determine uppload size, so 0 is 0 bytes, which fails
    for i in 1..files_to_put + 1 {
        let client = client.clone();
        let all_data_put = all_data_put.clone();
        let put_handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
            let (address, hash) = upload_data_using_client(client, i).await?;
            all_data_put.write().await.push((address, hash));
            Ok(())
        });

        put_tasks.push(put_handle);
    }

    let start_putting = Instant::now();
    futures::future::join_all(put_tasks).await;
    let duration = start_putting.elapsed();
    println!("Time elapsed in while putting all data is: {:?}", duration);

    assert_eq!(
        all_data_put.read().await.len(),
        files_to_put,
        "put data len is same as we tried to put"
    );

    let client = Client::new(config, bootstrap_nodes, None).await?;

    println!("Now we retrieve the data");

    let start_reading = Instant::now();

    for (address, known_hash) in all_data_put.read().await.iter().as_ref() {
        println!("...reading bytes at address {:?} ...", address);
        let mut bytes = client.read_bytes(*address).await;

        let mut attempts = 1;
        while bytes.is_err() && attempts < 10 {
            attempts += 1;
            // do some retries to ensure we're not just timing out by chance
            sleep(Duration::from_secs(1)).await;
            println!(
                "attempt #{attempts}...reading bytes at address {:?} ...",
                address
            );

            bytes = client.read_bytes(*address).await;
        }

        let bytes = bytes?;

        let bytes_len_mbs = bytes.len() / (1024 * 1024);
        println!("{bytes_len_mbs}mbs read from {:?}:", address);

        let mut hasher = Sha3::v256();
        let mut data_hash = [0; 32];
        hasher.update(&bytes);
        hasher.finalize(&mut data_hash);

        assert_eq!(&data_hash, known_hash);
    }

    let duration = start_reading.elapsed();

    println!("Time elapsed in while reading all data: {:?}", duration);

    println!("All okay");

    Ok(())
}

#[allow(dead_code)]
async fn upload_data_using_fresh_client(iteration: usize) -> Result<(BytesAddress, [u8; 32])> {
    // Now we upload the data.
    let (genesis_key, bootstrap_nodes) =
        read_network_conn_info().map_err(|_e| Error::NoNetworkKnowledge)?;
    let config = ClientConfig::new(None, None, genesis_key, None, None, None, None).await;
    let client = Client::new(config, bootstrap_nodes, None).await?;

    upload_data_using_client(client, iteration).await
}

async fn upload_data_using_client(
    client: Client,
    iteration: usize,
) -> Result<(BytesAddress, [u8; 32])> {
    let one_mb = 1024 * 1024;
    // start small and build up
    let bytes = random_bytes(one_mb * iteration);

    let bytes_len_mbs = iteration;
    println!("{bytes_len_mbs}mbs putting");

    let mut hasher = Sha3::v256();
    let mut output = [0; 32];
    hasher.update(&bytes);
    hasher.finalize(&mut output);
    let bytes_len = bytes.len();

    println!("==================== Upload iteration {iteration:?} ======================= ");
    println!("Storing bytes.len : {bytes_len:?} w/ hash {:?}", output);

    let start_putting = Instant::now();
    let address = client.upload(bytes, Scope::Public).await?;
    let duration = start_putting.elapsed();

    println!(
        "Time elapsed in while putting {bytes_len_mbs}mbs: {:?}",
        duration
    );

    println!("Bytes stored at xorname: {:?}", address);

    Ok((address, output))
}
