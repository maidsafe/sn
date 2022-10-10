// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::MyNode;

use std::sync::Arc;
use tokio::sync::RwLock;
use xor_name::Prefix;

pub(crate) struct LogCtx {
    node: Arc<RwLock<MyNode>>,
}

impl LogCtx {
    pub(crate) fn new(node: Arc<RwLock<MyNode>>) -> Self {
        Self { node }
    }

    pub(crate) async fn prefix(&self) -> Prefix {
        self.node.read().await.network_knowledge().prefix()
    }
}
