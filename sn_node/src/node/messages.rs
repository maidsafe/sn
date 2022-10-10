// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{Error, Result};

use sn_interface::{
    messaging::{system::NodeMsg, AuthKind, Dst, MsgId, NodeSig, SectionSigShare, WireMsg},
    network_knowledge::{MyNodeInfo, SectionKeyShare},
};

use bls::PublicKey as BlsPublicKey;

// Utilities for WireMsg.
pub(crate) trait WireMsgUtils {
    /// Creates a message signed using a BLS `KeyShare` for destination accumulation
    fn for_dst_accumulation(
        key_share: &SectionKeyShare,
        dst: Dst,
        node_msg: NodeMsg,
    ) -> Result<WireMsg, Error>;

    /// Creates a signed message from single node.
    fn single_src(
        node: &MyNodeInfo,
        dst: Dst,
        node_msg: NodeMsg,
        src_section_pk: BlsPublicKey,
    ) -> Result<WireMsg>;
}

impl WireMsgUtils for WireMsg {
    /// Creates a message signed using a BLS `KeyShare` for destination accumulation
    fn for_dst_accumulation(
        key_share: &SectionKeyShare,
        dst: Dst,
        msg: NodeMsg,
    ) -> Result<WireMsg, Error> {
        let msg_payload =
            WireMsg::serialize_msg_payload(&msg).map_err(|_| Error::InvalidMessage)?;

        let auth = AuthKind::SectionShare(SectionSigShare {
            public_key_set: key_share.public_key_set.clone(),
            index: key_share.index,
            signature_share: key_share.secret_key_share.sign(&msg_payload),
        });

        let wire_msg = WireMsg::new_msg(MsgId::new(), msg_payload, auth, dst);

        #[cfg(feature = "test-utils")]
        let wire_msg = wire_msg.set_payload_debug(msg);

        Ok(wire_msg)
    }

    /// Creates a signed message from single node.
    fn single_src(
        node: &MyNodeInfo,
        dst: Dst,
        msg: NodeMsg,
        src_section_pk: BlsPublicKey,
    ) -> Result<WireMsg> {
        let msg_payload =
            WireMsg::serialize_msg_payload(&msg).map_err(|_| Error::InvalidMessage)?;

        let auth = AuthKind::Node(
            NodeSig::authorize(src_section_pk, &node.keypair, &msg_payload).into_inner(),
        );

        let wire_msg = WireMsg::new_msg(MsgId::new(), msg_payload, auth, dst);

        #[cfg(feature = "test-utils")]
        let wire_msg = wire_msg.set_payload_debug(msg);

        Ok(wire_msg)
    }
}
