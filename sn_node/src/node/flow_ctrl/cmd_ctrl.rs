// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::flow_ctrl::{cmds::Cmd, dispatcher::Dispatcher};

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::RwLock;
use xor_name::XorName;

/// Takes care of spawning a new task for the processing of a cmd,
/// collecting resulting cmds from it, and sending it back to the calling context,
/// all the while logging the correlation between incoming and resulting cmds.
pub(crate) struct CmdCtrl {
    pub(crate) dispatcher: Arc<Dispatcher>,
    #[allow(dead_code)]
    id_counter: Arc<AtomicUsize>,
}

impl CmdCtrl {
    pub(crate) fn new(dispatcher: Dispatcher) -> Self {
        #[cfg(feature = "statemap")]
        sn_interface::statemap::log_metadata();

        Self {
            dispatcher: Arc::new(dispatcher),
            id_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn node(&self) -> Arc<RwLock<crate::node::MyNode>> {
        self.dispatcher.node()
    }

    /// Processes the passed in cmd on a new task
    pub(crate) async fn process_cmd_job(
        &self,
        cmd: Cmd,
        mut id: Vec<usize>,
        node_identifier: XorName,
        cmd_process_api: tokio::sync::mpsc::Sender<(Cmd, Vec<usize>)>,
    ) {
        trace!("about to spawn task for processing cmd: {cmd:?}, id: {id:?}");
        if id.is_empty() {
            id.push(self.id_counter.fetch_add(1, Ordering::SeqCst));
        }
        let dispatcher = self.dispatcher.clone();
        let _ = tokio::task::spawn(async move {
            debug!("> spawned process for cmd {cmd:?}, id: {id:?}");

            #[cfg(feature = "statemap")]
            sn_interface::statemap::log_state(node_identifier.to_string(), cmd.statemap_state());

            match dispatcher.process_cmd(cmd).await {
                Ok(cmds) => {
                    for (child_nr, cmd) in cmds.into_iter().enumerate() {
                        // zero based, first child of first cmd => [0, 0], second child => [0, 1], first child of second child => [0, 1, 0]
                        let child_id = [id.clone(), [child_nr].to_vec()].concat();
                        match cmd_process_api.send((cmd, child_id)).await {
                            Ok(_) => (), // no issues
                            Err(error) => {
                                let child_id = [id.clone(), [child_nr].to_vec()].concat();
                                error!(
                                    "Could not enqueue child cmd with id: {child_id:?}: {error:?}",
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    debug!("Error when processing cmd: {:?}", error);
                }
            }

            #[cfg(feature = "statemap")]
            sn_interface::statemap::log_state(
                node_identifier.to_string(),
                sn_interface::statemap::State::Idle,
            );
        });
    }
}
