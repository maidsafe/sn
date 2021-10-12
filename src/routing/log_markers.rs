// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

#[cfg(test)]
use strum_macros::EnumIter;

// this gets us to_string easily enough
#[cfg(test)]
use strum_macros::Display as StrumDisplay;

/// Internal log marker, to be used in tests asserts.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[cfg_attr(test, derive(EnumIter, StrumDisplay))]
#[allow(missing_docs)]
pub(crate) enum LogMarker {
    ServiceMsgToBeHandled,
    SystemMsgToBeHandled,
    StoringChunk,
    ChunkStoreReceivedAtElder,
    StoredNewChunk,
    ChunkQueryResponseReceviedFromAdult,
    ChunkQueryReceviedAtElder,
    ChunkQueryReceviedAtAdult,
}
