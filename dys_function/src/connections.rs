// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::{get_mean_of, DysfunctionDetection, NodeIdentifier};
use std::collections::BTreeMap;
use xor_name::XorName;

impl DysfunctionDetection {
    /// Track a communication issue for a given node
    // TODO: Should we limit how long we track this for?
    pub fn track_comm_issue(&self, node_id: NodeIdentifier) {
        // initial entry setup if non existent
        let mut entry = self.communication_issues.entry(node_id).or_default();

        let value = entry.value_mut();
        *value += 1;
    }

    /// Calculate a score of this node, as compared to its closest neighbours...
    /// The average score of all neighbours is calculated, and the standard deviation therein.
    /// We then calculate the node's z-score and multiply excess pending ops by this
    pub(crate) fn calculate_connections_score(&self) -> BTreeMap<XorName, usize> {
        let mut score_map = BTreeMap::default();
        // loop over all node/neighbour comparisons
        for (node, neighbours) in self.get_node_and_neighbours_vec() {
            // let node = *node;
            let nodes_count = if let Some(entry) = self.communication_issues.get(&node) {
                *entry.value()
            } else {
                0
            };

            let mut all_neighbourhood_counts = vec![];
            for neighbour in neighbours {
                if let Some(entry) = self.communication_issues.get(&neighbour) {
                    let val = *entry.value();

                    all_neighbourhood_counts.push(val as f32);
                }
            }

            let avg_in_neighbourhood = get_mean_of(&all_neighbourhood_counts);
            trace!(
                "node has {nodes_count}, mean conns: {:?}",
                avg_in_neighbourhood
            );

            trace!("let pending ops: in hood: {:?}", all_neighbourhood_counts);

            let final_score = nodes_count;
            // let final_score = usize::max( nodes_count, avg_in_neighbourhood);
            // if nodes_count > avg_in_neighbourhood {
            //     nodes_count
            // } else {
            //     avg_in_neighbourhood
            // };

            debug!("{node} Conn score {final_score}");
            let _prev = score_map.insert(node, final_score);
        }

        score_map
    }
}

#[cfg(test)]
mod tests {
    use super::DysfunctionDetection;
    use crate::tests::{init_test_logger, ELDER_COUNT};

    use eyre::Error;
    use xor_name::XorName;

    type Result<T, E = Error> = std::result::Result<T, E>;

    // Above this, nodes should be sus
    pub(crate) const CONNECTION_PROBLEM_TOLERANCE: usize = 20;

    #[tokio::test]
    async fn conn_dys_is_tolerant_of_norms() -> Result<()> {
        let adults = (0..10).map(|_| XorName::random()).collect::<Vec<XorName>>();

        let dysfunctional_detection = DysfunctionDetection::new(adults.clone(), ELDER_COUNT);

        // Write data CONNECTION_PROBLEM_TOLERANCE times to the 10 adults
        for adult in &adults {
            for _ in 0..CONNECTION_PROBLEM_TOLERANCE {
                dysfunctional_detection.track_comm_issue(*adult);
            }
        }

        // Assert there are not any dysfuncitonal nodes
        // This is because all of them are within the tolerance ratio of each other
        assert_eq!(
            dysfunctional_detection
                .get_dysfunctional_node_names()
                .await
                .len(),
            0,
            "no nodes are dysfunctional"
        );
        assert_eq!(
            dysfunctional_detection
                .get_suspicious_node_names()
                .await
                .len(),
            0,
            "no nodes are suspect"
        );

        Ok(())
    }

    #[tokio::test]
    async fn conn_dysfunction_basics_sus_comes_first() -> Result<()> {
        init_test_logger();
        let _outer_span = tracing::info_span!("conn_dysfunction_basics_sus_comes_first").entered();

        let adults = (0..10).map(|_| XorName::random()).collect::<Vec<XorName>>();

        let dysfunctional_detection = DysfunctionDetection::new(adults.clone(), ELDER_COUNT);

        // Write data PENDING_OPS_TOLERANCE times to the 10 adults
        for adult in &adults {
            for _ in 0..CONNECTION_PROBLEM_TOLERANCE {
                dysfunctional_detection.track_comm_issue(*adult);
            }
        }

        // Add a new adults
        let new_adult = XorName::random();
        dysfunctional_detection.add_new_node(new_adult);

        // Assert total adult count
        assert_eq!(dysfunctional_detection.closest_nodes_to.len(), 11);

        // Add issues for our new adult connection issues
        for _ in 0..CONNECTION_PROBLEM_TOLERANCE + 10 {
            dysfunctional_detection.track_comm_issue(new_adult);
        }

        let sus = dysfunctional_detection.get_suspicious_node_names().await;
        // Assert that the new adult is detected as suspect.
        assert_eq!(sus.len(), 1, "one node is not sus");
        assert!(sus.contains(&new_adult), "our adult is not sus");

        let dysfunctional_nodes = dysfunctional_detection.get_dysfunctional_node_names().await;

        // Assert that the new adult is not yet dysfuncitonal
        assert!(
            !dysfunctional_nodes.contains(&new_adult),
            "our added node is dysfunctional when it should not be"
        );

        assert_eq!(
            dysfunctional_nodes.len(),
            0,
            "more nodes are dysfunctional than they should be"
        );

        // Add MORE connection issues... we should nopw get labelled as dysfunctional
        for _ in 0..CONNECTION_PROBLEM_TOLERANCE {
            dysfunctional_detection.track_comm_issue(new_adult);
        }

        let sus = dysfunctional_detection.get_suspicious_node_names().await;
        // Assert that the new adult is detected as suspect.
        assert!(sus.contains(&new_adult), "our adult is still sus");
        assert_eq!(sus.len(), 1, "only one adult is sus");

        let dysfunctional_nodes = dysfunctional_detection.get_dysfunctional_node_names().await;

        // Assert that the new adult is not NOW dysfuncitonal
        assert!(
            dysfunctional_nodes.contains(&new_adult),
            "our adult is now dysfunctional"
        );
        assert_eq!(
            dysfunctional_nodes.len(),
            1,
            "our adult is the only dysfunctional nodes"
        );

        Ok(())
    }
}
