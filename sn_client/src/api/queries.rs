// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

// use std::collections::BTreeSet;

use super::Client;
use crate::{connections::QueryResult, errors::Error};

use sn_interface::{
    data_copy_count,
    messaging::{
        data::{DataQuery, DataQueryVariant, ServiceMsg},
        ServiceAuth, WireMsg,
    },
    types::{Peer, PublicKey, Signature},
};

use backoff::{backoff::Backoff, ExponentialBackoff};
use bytes::Bytes;
use tokio::time::Duration;
use tracing::{debug, info_span};

impl Client {
    /// Send a Query to the network and await a response.
    /// Queries are automatically retried using exponential backoff if the timeout is hit.
    #[instrument(skip(self), level = "debug")]
    pub async fn send_query(&self, query: DataQueryVariant) -> Result<QueryResult, Error> {
        self.send_query_with_retry(query, true).await
    }

    /// Send a Query to the network and await a response.
    /// Queries are not retried if the timeout is hit.
    #[instrument(skip(self), level = "debug")]
    pub async fn send_query_without_retry(
        &self,
        query: DataQueryVariant,
    ) -> Result<QueryResult, Error> {
        self.send_query_with_retry(query, false).await
    }

    // Send a Query to the network and await a response.
    // Queries are automatically retried if the timeout is hit
    // This function is a private helper.
    #[instrument(skip(self), level = "debug")]
    async fn send_query_with_retry(
        &self,
        query: DataQueryVariant,
        retry: bool,
    ) -> Result<QueryResult, Error> {
        let client_pk = self.public_key();
        let mut query = DataQuery {
            adult_index: 0,
            variant: query,
        };

        // Add jitter so not all clients retry at the same rate. This divider will knock on to the overall retry window
        // and should help prevent elders from being conseceutively overwhelmed
        trace!("Setting up query retry");

        let span = info_span!("Attempting a query");
        let _ = span.enter();
        let mut attempts = 1;
        let dst = query.variant.dst_name();
        // should we force a fresh connection to the nodes?
        let mut force_new_link = false;

        let max_interval = self.max_backoff_interval;

        let mut backoff = ExponentialBackoff {
            initial_interval: Duration::from_secs(1),
            max_interval,
            max_elapsed_time: Some(self.query_timeout),
            randomization_factor: 1.5,
            ..Default::default()
        };

        // this seems needed for custom settings to take effect
        backoff.reset();

        loop {
            let msg = ServiceMsg::Query(query.clone());
            let serialised_query = WireMsg::serialize_msg_payload(&msg)?;
            let signature = self.keypair.sign(&serialised_query);
            debug!(
                "Attempting {:?} (attempt #{}) will force new: {force_new_link}",
                query, attempts
            );

            // grab up to date destination section from our local network knowledge
            let (section_pk, elders) = self.session.get_query_elders(dst).await?;

            let res = self
                .send_signed_query_to_section(
                    query.clone(),
                    client_pk,
                    serialised_query.clone(),
                    signature.clone(),
                    Some((section_pk, elders.clone())),
                    force_new_link,
                )
                .await;

            attempts += 1;

            // In the next attempt, try the next adult, further away.
            query.adult_index += 1;
            // There should not be more than a certain amount of adults holding copies of the data. Retry the closest adult again.
            if query.adult_index >= data_copy_count() {
                query.adult_index = 0;

                force_new_link = true;

                if !retry {
                    // we dont want to retry beyond data_copy_count adults so
                    return res;
                }
            }

            if let Some(delay) = backoff.next_backoff() {
                // if we've an acceptable result, return instead of wait/retry loop
                if let Ok(result) = res {
                    if result.data_was_found() {
                        debug!("{query:?} sent and received okay");
                        return Ok(result);
                    } else {
                        warn!(
                            "Data not found... querying again until we hit query_timeout ({:?})",
                            self.query_timeout
                        );
                    }
                }

                debug!("Sleeping before trying query again: {delay:?} sleep for {query:?}");
                tokio::time::sleep(delay).await;
            } else {
                // we're done trying
                return res;
            }
        }
    }

    /// Send a Query to the network and await a response.
    /// This is part of a public API, for the user to
    /// provide the serialised and already signed query.
    pub async fn send_signed_query(
        &self,
        query: DataQuery,
        client_pk: PublicKey,
        serialised_query: Bytes,
        signature: Signature,
    ) -> Result<QueryResult, Error> {
        debug!("Sending Query: {:?}", query);
        self.send_signed_query_to_section(
            query,
            client_pk,
            serialised_query,
            signature,
            None,
            false,
        )
        .await
    }

    // Private helper to send a signed query, with the option to define the destination section.
    // If no destination section is provided, it will be derived from the query content.
    async fn send_signed_query_to_section(
        &self,
        query: DataQuery,
        client_pk: PublicKey,
        serialised_query: Bytes,
        signature: Signature,
        dst_section_info: Option<(bls::PublicKey, Vec<Peer>)>,
        force_new_link: bool,
    ) -> Result<QueryResult, Error> {
        let auth = ServiceAuth {
            public_key: client_pk,
            signature,
        };

        self.session
            .send_query(
                query,
                auth,
                serialised_query,
                #[cfg(feature = "traceroute")]
                self.public_key(),
                dst_section_info,
                force_new_link,
            )
            .await
    }
}
