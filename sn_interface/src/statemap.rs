// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! Instrumentation for statemaps: <https://github.com/TritonDataCenter/statemap>

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

/// Log the metadata for a statemap, this should be called as soon as the system starts up.
pub fn log_metadata() {
    let metadata = json!({
        "title": "The Various States of Safe Network",
        "start": [0, 0], // [seconds, nanoseconds] since unix epoch
        "states": State::metadata_json()
    });

    trace!("STATEMAP_METADATA: {metadata}");
}

/// Log a statemap entry, call this whenever you would like to log a state transition.
pub fn log_state(entity: String, state: State) {
    let time = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(t) => t,
        Err(e) => {
            error!("STATEMAP_ENTRY: failed to read system time: {e:?}");
            return;
        }
    };

    let entry = serde_json::json!({
        "time": time.as_nanos().to_string(),
        "entity": entity,
        "state": state as usize,
    });

    trace!("STATEMAP_ENTRY: {entry}")
}

/// States used for generating statemaps
pub enum State {
    Idle,
    Validation,
    Comms,
    Dysfunction,
    BackPressure,
    ServiceMsg,
    Dkg,
    Agreement,
    Membership,
    Handover,
    Replication,
    AntiEntropy,
    Relocate,
    Join,
    Propose,
    Node,
}

impl State {
    pub fn metadata_json() -> serde_json::Value {
        // Colors generated with https://mokole.com/palette.html
        serde_json::json!({
            "Idle": { "value": State::Idle as usize, "color": "#f9f9f9" },
            "Validation": { "value": State::Validation as usize, "color": "#7f0000" },
            "Comms": { "value": State::Comms as usize, "color": "#808000" },
            "Dysfunction": { "value": State::Dysfunction as usize, "color": "#000080" },
            "BackPressure": { "value": State::BackPressure as usize, "color": "#ff0000" },
            "ServiceMsg": { "value": State::ServiceMsg as usize, "color": "#00ced1" },
            "Dkg": { "value": State::Dkg as usize, "color": "#ffa500" },
            "Agreement": { "value": State::Agreement as usize, "color": "#7fff00" },
            "Membership": { "value": State::Membership as usize, "color": "#e9967a" },
            "Handover": { "value": State::Handover as usize, "color": "#0000ff" },
            "Replication": { "value": State::Replication as usize, "color": "#ff00ff" },
            "AntiEntropy": { "value": State::AntiEntropy as usize, "color": "#1e90ff" },
            "Relocate": { "value": State::Relocate as usize, "color": "#ffff54" },
            "Join": { "value": State::Join as usize, "color": "#dda0dd" },
            "Propose": { "value": State::Propose as usize, "color": "#ff1493" },
            "Node": { "value": State::Node as usize, "color": "#98fb98" },
        })
    }
}
