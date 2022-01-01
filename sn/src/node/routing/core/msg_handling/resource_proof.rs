// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::system::{JoinResponse, ResourceProofResponse, SystemMsg};
use crate::node::{
    error::{Error, Result},
    routing::{
        api::command::Command,
        core::{Core, RESOURCE_PROOF_DATA_SIZE, RESOURCE_PROOF_DIFFICULTY},
        ed25519,
    },
};
use crate::peer::Peer;
use crate::types::log_markers::LogMarker;

use ed25519_dalek::Verifier;
use xor_name::XorName;

// Resource signed
impl Core {
    pub(crate) async fn validate_resource_proof_response(
        &self,
        peer_name: &XorName,
        response: ResourceProofResponse,
    ) -> bool {
        let serialized = if let Ok(serialized) = bincode::serialize(&(peer_name, &response.nonce)) {
            serialized
        } else {
            return false;
        };

        if self
            .node
            .read()
            .await
            .keypair
            .public
            .verify(&serialized, &response.nonce_signature)
            .is_err()
        {
            return false;
        }

        self.resource_proof
            .validate_all(&response.nonce, &response.data, response.solution)
    }

    pub(crate) async fn send_resource_proof_challenge(&self, peer: Peer) -> Result<Command> {
        let nonce: [u8; 32] = rand::random();
        let serialized =
            bincode::serialize(&(peer.name(), &nonce)).map_err(|_| Error::InvalidMessage)?;
        let response = SystemMsg::JoinResponse(Box::new(JoinResponse::ResourceChallenge {
            data_size: RESOURCE_PROOF_DATA_SIZE,
            difficulty: RESOURCE_PROOF_DIFFICULTY,
            nonce,
            nonce_signature: ed25519::sign(&serialized, &self.node.read().await.keypair),
        }));

        trace!("{}", LogMarker::SendResourceProofChallenge);
        self.send_direct_message(peer, response, self.network_knowledge.section_key().await)
            .await
    }
}
