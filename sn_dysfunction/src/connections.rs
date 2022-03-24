// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::{error::Result, DysfunctionDetection, NodeIdentifier};
use std::time::SystemTime;

impl DysfunctionDetection {
    /// Track a communication issue for a given node
    pub async fn track_comm_issue(&self, node_id: NodeIdentifier) -> Result<()> {
        // initial entry setup if non existent
        let mut entry = self.communication_issues.entry(node_id).or_default();

        let mut queue = entry.value_mut().write().await;
        queue.push_back(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs(),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{init_test_logger, ELDER_COUNT};
    use crate::{DysfunctionDetection, DysfunctionSeverity};

    use eyre::Error;
    use xor_name::{rand::random as random_xorname, XorName};

    type Result<T, E = Error> = std::result::Result<T, E>;

    // Above this, nodes should be sus
    // this is only counting last RECENT minutes atm
    pub(crate) const NORMAL_CONNECTION_PROBLEM_COUNT: usize = 5;
    pub(crate) const SUSPECT_CONNECTION_PROBLEM_COUNT: usize = 15;
    pub(crate) const DYSFUNCTIONAL_CONNECTION_PROBLEM_COUNT: usize = 25;

    #[tokio::test]
    async fn conn_dys_is_tolerant_of_norms() -> Result<()> {
        let adults = (0..10).map(|_| random_xorname()).collect::<Vec<XorName>>();

        let dysfunctional_detection = DysfunctionDetection::new(adults.clone(), ELDER_COUNT);

        // Write data CONNECTION_PROBLEM_TOLERANCE times to the 10 adults
        for adult in &adults {
            for _ in 0..NORMAL_CONNECTION_PROBLEM_COUNT {
                dysfunctional_detection.track_comm_issue(*adult).await?;
            }
        }

        // Assert there are not any dysfuncitonal nodes
        // This is because all of them are within the tolerance ratio of each other
        assert_eq!(
            dysfunctional_detection
                .get_nodes_beyond_severity(DysfunctionSeverity::Dysfunctional)
                .await?
                .len(),
            0,
            "no nodes are dysfunctional"
        );
        assert_eq!(
            dysfunctional_detection
                .get_nodes_beyond_severity(DysfunctionSeverity::Suspicious)
                .await?
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

        let adults = (0..10).map(|_| random_xorname()).collect::<Vec<XorName>>();

        let dysfunctional_detection = DysfunctionDetection::new(adults.clone(), ELDER_COUNT);

        // Write data PENDING_OPS_TOLERANCE times to the 10 adults
        for adult in &adults {
            for _ in 0..NORMAL_CONNECTION_PROBLEM_COUNT {
                dysfunctional_detection.track_comm_issue(*adult).await?;
            }
        }

        // Add a new adults
        let new_adult = random_xorname();
        dysfunctional_detection.add_new_node(new_adult);

        // Assert total adult count
        assert_eq!(dysfunctional_detection.closest_nodes_to.len(), 11);

        // Add issues for our new adult connection issues
        for _ in 0..SUSPECT_CONNECTION_PROBLEM_COUNT {
            dysfunctional_detection.track_comm_issue(new_adult).await?;
        }

        let sus = dysfunctional_detection
            .get_nodes_beyond_severity(DysfunctionSeverity::Suspicious)
            .await?;
        // Assert that the new adult is detected as suspect.
        assert_eq!(sus.len(), 1, "only one node is sus");
        assert!(sus.contains(&new_adult), "our adult is not sus");

        let dysfunctional_nodes = dysfunctional_detection
            .get_nodes_beyond_severity(DysfunctionSeverity::Dysfunctional)
            .await?;

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
        for _ in 0..DYSFUNCTIONAL_CONNECTION_PROBLEM_COUNT - SUSPECT_CONNECTION_PROBLEM_COUNT {
            // get to the dys total only
            dysfunctional_detection.track_comm_issue(new_adult).await?;
        }

        let sus = dysfunctional_detection
            .get_nodes_beyond_severity(DysfunctionSeverity::Suspicious)
            .await?;
        // Assert that the new adult is detected as suspect.
        assert!(sus.contains(&new_adult), "our adult is still sus");
        assert_eq!(sus.len(), 1, "only one adult is sus");

        debug!("=============================");
        let dysfunctional_nodes = dysfunctional_detection
            .get_nodes_beyond_severity(DysfunctionSeverity::Dysfunctional)
            .await?;

        // Assert that the new adult is not NOW dysfuncitonal
        assert!(
            dysfunctional_nodes.contains(&new_adult),
            "our adult should now be dysfunctional but is not"
        );
        assert_eq!(
            dysfunctional_nodes.len(),
            1,
            "our node should be the only dysfunctional nodes"
        );

        Ok(())
    }
}
