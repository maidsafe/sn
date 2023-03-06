// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{flow_ctrl::cmds::Cmd, MyNode, Result};

use sn_fault_detection::IssueType;
use sn_interface::types::Participant;

use std::collections::BTreeSet;
use xor_name::XorName;

impl MyNode {
    /// Handle error in communication with node.
    pub(crate) fn handle_comms_error(&self, participant: Participant, error: sn_comms::Error) {
        use sn_comms::Error::*;
        match error {
            ConnectingToUnknownNode(msg_id) => {
                trace!(
                    "Tried to send msg {msg_id:?} to unknown participant {participant}. No connection made.",
                );
            }
            CannotConnectEndpoint(_err) => {
                trace!("Cannot connect to endpoint: {participant}");
            }
            AddressNotReachable(_err) => {
                trace!("Address not reachable: {participant}");
            }
            FailedSend(msg_id) => {
                trace!("Could not send {msg_id:?}, lost known participant: {participant}");
            }
            InvalidMsgReceived(msg_id) => {
                trace!("Invalid msg {msg_id:?} received from {participant}");
            }
        }
        // Track comms issue if this is a node in our section.
        if self
            .network_knowledge
            .is_section_member(&participant.name())
        {
            self.track_node_issue(participant.name(), IssueType::Communication);
        }
    }

    pub(crate) fn cast_offline_proposals(&mut self, names: &BTreeSet<XorName>) -> Result<Vec<Cmd>> {
        // Don't send the `Offline` proposal to the node being lost as that send would fail,
        // triggering a chain of further `Offline` proposals.
        let elders: Vec<_> = self
            .network_knowledge
            .section_auth()
            .elders()
            .filter(|node_id| !names.contains(&node_id.name()))
            .cloned()
            .collect();
        let mut result: Vec<Cmd> = Vec::new();
        for name in names.iter() {
            if let Some(info) = self.network_knowledge.get_section_member(name) {
                let info = info.leave()?;
                if let Ok(cmds) = self.send_node_off_proposal(elders.clone(), info) {
                    result.extend(cmds);
                }
            }
        }
        Ok(result)
    }
}
