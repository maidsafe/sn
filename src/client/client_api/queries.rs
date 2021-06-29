// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Client;
use crate::client::{connections::QueryResult, errors::Error};
use crate::messaging::client::{ClientSig, Query};
use crate::types::{PublicKey, Signature};
use log::debug;
use tokio::time::Duration;
const QUERY_TIMEOUT: Duration = Duration::from_secs(20); // 20 seconds

impl Client {
    /// Send a Query to the network and await a response
    pub(crate) async fn send_signed_query(
        &self,
        query: Query,
        client_pk: PublicKey,
        signature: Signature,
    ) -> Result<QueryResult, Error> {
        debug!("Sending Query: {:?}", query);
        let client_sig = ClientSig {
            public_key: client_pk,
            signature,
        };

        self.session.send_query(query, client_sig).await
    }

    // Send a Query to the network and await a response.
    // This function is a helper private to this module.
    pub(crate) async fn send_query(&self, query: Query) -> Result<QueryResult, Error> {
        let client_pk = self.public_key();
        let signature = self.keypair.sign(b"TODO");

        tokio::time::timeout(
            QUERY_TIMEOUT,
            self.send_signed_query(query, client_pk, signature),
        )
        .await
        .map_err(|_| Error::NoResponse)?
    }
}
