// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::{EndUser, MessageId};
use crate::routing::XorName;
use crate::types::ChunkAddress;
use itertools::Itertools;
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};

use crate::node::capacity::CHUNK_COPY_COUNT;

const NEIGHBOUR_COUNT: usize = 2;
const MIN_PENDING_OPS: usize = 10;
const PENDING_OP_TOLERANCE_RATIO: f64 = 0.1;

#[derive(Clone, Debug)]
struct ReadOperation {
    head_address: ChunkAddress,
    origin: EndUser,
    targets: BTreeSet<XorName>,
    responded_with_success: bool,
}

pub struct AdultLiveness {
    ops: HashMap<MessageId, ReadOperation>,
    pending_ops: HashMap<XorName, usize>,
    closest_adults: HashMap<XorName, Vec<XorName>>,
}

impl AdultLiveness {
    pub fn new() -> Self {
        Self {
            ops: HashMap::default(),
            pending_ops: HashMap::default(),
            closest_adults: HashMap::default(),
        }
    }

    // Inserts a new read operation
    // Returns false if the operation already existed.
    pub fn new_read(
        &mut self,
        msg_id: MessageId,
        head_address: ChunkAddress,
        origin: EndUser,
        targets: BTreeSet<XorName>,
    ) -> bool {
        let new_operation = if let Entry::Vacant(entry) = self.ops.entry(msg_id) {
            let _ = entry.insert(ReadOperation {
                head_address,
                origin,
                targets: targets.clone(),
                responded_with_success: false,
            });
            true
        } else {
            false
        };
        if new_operation {
            self.increment_pending_op(&targets);
        }
        new_operation
    }

    pub fn retain_members_only(&mut self, current_members: BTreeSet<XorName>) {
        let old_members = self.closest_adults.keys().cloned().collect::<Vec<_>>();
        for name in old_members {
            if !current_members.contains(&name) {
                let _ = self.pending_ops.remove(&name);
                let _ = self.closest_adults.remove(&name);
                let message_ids = self.ops.keys().cloned().collect::<Vec<_>>();
                // TODO(after T4): For write operations perhaps we need to write it to a different Adult
                for msg_id in message_ids {
                    self.remove_target(msg_id, &name);
                }
            }
        }
        self.recompute_closest_adults();
    }

    pub fn remove_target(&mut self, msg_id: MessageId, name: &XorName) {
        if let Some(count) = self.pending_ops.get_mut(name) {
            let counter = *count;
            if counter > 0 {
                *count -= 1;
            }
        }
        let complete = if let Some(operation) = self.ops.get_mut(&msg_id) {
            let ReadOperation { targets, .. } = operation;
            let _ = targets.remove(name);
            targets.is_empty()
        } else {
            true
        };
        if complete {
            let _ = self.ops.remove(&msg_id);
        }
    }

    pub fn record_adult_read_liveness(
        &mut self,
        correlation_id: MessageId,
        src: &XorName,
        success: bool,
    ) -> Option<(ChunkAddress, EndUser)> {
        self.remove_target(correlation_id, src);
        let op = self.ops.get_mut(&correlation_id);
        op.and_then(|op| {
            let ReadOperation {
                head_address,
                origin,
                targets,
                responded_with_success,
            } = op;

            if targets.len() < CHUNK_COPY_COUNT && *responded_with_success {
                None
            } else {
                *responded_with_success = success;
                Some((*head_address, *origin))
            }
        })
    }

    fn increment_pending_op(&mut self, targets: &BTreeSet<XorName>) {
        for node in targets {
            *self.pending_ops.entry(*node).or_insert(0) += 1;
            if !self.closest_adults.contains_key(node) {
                let _ = self.closest_adults.insert(*node, Vec::new());
                self.recompute_closest_adults();
            }
        }
    }

    pub fn recompute_closest_adults(&mut self) {
        let closest_adults_collection = self
            .closest_adults
            .iter()
            .map(|(key, _)| {
                let closest_adults = self
                    .closest_adults
                    .keys()
                    .filter(|name| key != *name)
                    .sorted_by(|lhs, rhs| key.cmp_distance(lhs, rhs))
                    .take(NEIGHBOUR_COUNT)
                    .copied()
                    .collect::<Vec<_>>();

                (key.to_owned(), closest_adults)
            })
            .collect::<Vec<(XorName, Vec<XorName>)>>();

        self.closest_adults.extend(closest_adults_collection);
    }

    pub fn find_unresponsive_adults(&self) -> Vec<(XorName, usize)> {
        let mut unresponsive_adults = Vec::new();
        for (adult, neighbours) in &self.closest_adults {
            if let Some(max_pending_by_neighbours) = neighbours
                .iter()
                .map(|neighbour| self.pending_ops.get(neighbour).unwrap_or(&0))
                .max()
            {
                let adult_pending_ops = *self.pending_ops.get(adult).unwrap_or(&0);
                if adult_pending_ops > MIN_PENDING_OPS
                    && max_pending_by_neighbours > &MIN_PENDING_OPS
                    && adult_pending_ops as f64 * PENDING_OP_TOLERANCE_RATIO
                        > *max_pending_by_neighbours as f64
                {
                    tracing::info!(
                        "Pending ops for {}: {} Neighbour max: {}",
                        adult,
                        adult_pending_ops,
                        max_pending_by_neighbours
                    );
                    unresponsive_adults.push((*adult, adult_pending_ops));
                }
            }
        }
        unresponsive_adults
    }
}
