// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use std::collections::BTreeSet;

use crate::routing::{Prefix, XorName};

use crate::node::network::Network;

#[derive(Clone)]
pub(crate) struct AdultReader {
    network: Network,
}

impl AdultReader {
    /// Access to the current state of our adult constellation
    pub(crate) fn new(network: Network) -> Self {
        Self { network }
    }

    /// Get the sections's current Prefix
    pub(crate) async fn our_prefix(&self) -> Prefix {
        self.network.our_prefix().await
    }

    /// Dynamic state
    pub(crate) async fn non_full_adults_closest_to(
        &self,
        name: &XorName,
        full_adults: &BTreeSet<XorName>,
        count: usize,
    ) -> BTreeSet<XorName> {
        self.network
            .our_adults_sorted_by_distance_to(name)
            .await
            .into_iter()
            .filter(|name| !full_adults.contains(name))
            .take(count)
            .collect::<BTreeSet<_>>()
    }
}
