// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{Mapping, MsgContext};
use crate::messaging::{
    data::{DataMsg, ProcessMsg, ProcessingError},
    ClientSigned, EndUser, MessageId, SrcLocation,
};
use crate::node::{
    error::convert_to_error_message,
    node_ops::{MsgType, NodeDuty, OutgoingMsg},
    Error,
};
use tracing::warn;

pub(super) fn map_client_msg(
    msg_id: MessageId,
    msg: DataMsg,
    client_signed: ClientSigned,
    user: EndUser,
) -> Mapping {
    match &msg {
        DataMsg::Process(process_msg) => {
            // Signature has already been validated by the routing layer
            let op = map_client_process_msg(msg_id, process_msg.clone(), user, client_signed);

            let ctx = Some(MsgContext::Client {
                msg,
                src: SrcLocation::EndUser(user),
            });

            Mapping { op, ctx }
        }
        DataMsg::ProcessingError(error) => {
            warn!(
                "A node should never receive a DataMsg::ProcessingError {:?}",
                error
            );

            Mapping {
                op: NodeDuty::NoOp,
                ctx: None,
            }
        }
    }
}

fn map_client_process_msg(
    msg_id: MessageId,
    process_msg: ProcessMsg,
    origin: EndUser,
    client_signed: ClientSigned,
) -> NodeDuty {
    match process_msg {
        ProcessMsg::Query(query) => NodeDuty::ProcessRead {
            query,
            msg_id,
            client_signed,
            origin,
        },
        ProcessMsg::Cmd(cmd) => NodeDuty::ProcessWrite {
            cmd,
            msg_id,
            client_signed,
            origin,
        },
        _ => {
            let error_data = convert_to_error_message(Error::InvalidMessage(
                msg_id,
                format!("Unknown user msg: {:?}", process_msg),
            ));
            let src = SrcLocation::EndUser(origin);
            let id = MessageId::in_response_to(&msg_id);

            NodeDuty::Send(OutgoingMsg {
                id,
                msg: MsgType::Client(DataMsg::ProcessingError(ProcessingError {
                    reason: Some(error_data),
                    source_message: Some(process_msg),
                })),
                dst: src.to_dst(),
                aggregation: false,
            })
        }
    }
}
