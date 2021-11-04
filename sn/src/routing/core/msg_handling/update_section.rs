// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::Core;
use crate::routing::{error::Result, Event};
use std::collections::BTreeSet;

impl Core {
    pub(crate) async fn fire_node_event_for_any_new_adults(&self) -> Result<()> {
        let old_adults: BTreeSet<_> = self
            .network_knowledge
            .live_adults()
            .await
            .iter()
            .map(|p| p.name())
            .collect();

        if self.is_not_elder().await {
            let current_adults: BTreeSet<_> = self
                .network_knowledge
                .live_adults()
                .await
                .iter()
                .map(|p| p.name())
                .collect();
            let added: BTreeSet<_> = current_adults.difference(&old_adults).copied().collect();
            let removed: BTreeSet<_> = old_adults.difference(&current_adults).copied().collect();

            if !added.is_empty() || !removed.is_empty() {
                self.send_event(Event::AdultsChanged {
                    remaining: old_adults.intersection(&current_adults).copied().collect(),
                    added,
                    removed,
                })
                .await;
            }
        }

        Ok(())
    }
}
