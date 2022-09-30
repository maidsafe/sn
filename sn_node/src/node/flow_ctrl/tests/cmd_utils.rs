use crate::node::{
    flow_ctrl::cmds::Cmd::HandleValidClientMsg,
    flow_ctrl::dispatcher::Dispatcher,
    messaging::{OutgoingMsg, Peers},
    Cmd,
};
use assert_matches::assert_matches;
use eyre::eyre;
use eyre::Result;
#[cfg(feature = "traceroute")]
use sn_interface::messaging::Traceroute;
use sn_interface::{
    messaging::{
        data::{ClientMsg, Error as MessagingDataError},
        serialisation::WireMsg,
        system::{JoinResponse, MembershipState, NodeCmd, NodeMsg, RelocateDetails},
        AuthorityProof, ClientAuth, MsgId, MsgType,
    },
    network_knowledge::{test_utils::*, NodeState, SectionAuthorityProvider},
    types::{Keypair, Peer, ReplicatedData, SecretKeySet},
};
use std::collections::BTreeSet;

pub(crate) struct HandleOnlineStatus {
    pub(crate) node_approval_sent: bool,
    pub(crate) relocate_details: Option<RelocateDetails>,
}

pub(crate) async fn handle_online_cmd(
    peer: &Peer,
    sk_set: &bls::SecretKeySet,
    dispatcher: &Dispatcher,
    section_auth: &SectionAuthorityProvider,
) -> Result<HandleOnlineStatus> {
    let node_state = NodeState::joined(*peer, None);
    let membership_decision = section_decision(sk_set, node_state.to_msg())?;

    let all_cmds = run_and_collect_cmds(
        Cmd::HandleMembershipDecision(membership_decision),
        dispatcher,
    )
    .await?;

    let mut status = HandleOnlineStatus {
        node_approval_sent: false,
        relocate_details: None,
    };

    for cmd in all_cmds {
        let (msg, recipients) = match cmd {
            Cmd::SendMsg {
                recipients,
                msg: OutgoingMsg::Node(msg),
                ..
            } => (msg, recipients),
            _ => continue,
        };

        match msg {
            NodeMsg::JoinResponse(response) => {
                if let JoinResponse::Approved {
                    section_tree_update,
                    ..
                } = *response
                {
                    assert_eq!(
                        section_tree_update.section_auth,
                        section_auth.clone().to_msg()
                    );
                    assert_matches!(recipients, Peers::Multiple(peers) => {
                        assert_eq!(peers, BTreeSet::from([*peer]));
                    });
                    status.node_approval_sent = true;
                }
            }
            NodeMsg::Propose {
                proposal: sn_interface::messaging::system::Proposal::VoteNodeOffline(node_state),
                ..
            } => {
                if let MembershipState::Relocated(details) = node_state.state {
                    if details.previous_name != peer.name() {
                        continue;
                    }
                    status.relocate_details = Some(*details.clone());
                }
            }
            _ => continue,
        }
    }

    Ok(status)
}

pub(crate) async fn run_and_collect_cmds(
    cmd: Cmd,
    dispatcher: &Dispatcher,
) -> crate::node::error::Result<Vec<Cmd>> {
    let mut all_cmds = vec![];

    let mut cmds = dispatcher.process_cmd(cmd).await?;

    while !cmds.is_empty() {
        all_cmds.extend(cmds.clone());
        let mut new_cmds = vec![];
        for cmd in cmds {
            if !matches!(cmd, Cmd::SendMsg { .. }) {
                new_cmds.extend(dispatcher.process_cmd(cmd).await?);
            }
        }
        cmds = new_cmds;
    }

    Ok(all_cmds)
}

pub(crate) fn wrap_service_msg_for_handling(msg: ClientMsg, peer: Peer) -> Result<Cmd> {
    let payload = WireMsg::serialize_msg_payload(&msg)?;
    let src_client_keypair = Keypair::new_ed25519();
    let auth = ClientAuth {
        public_key: src_client_keypair.public_key(),
        signature: src_client_keypair.sign(&payload),
    };
    let auth_proof = AuthorityProof::verify(auth, &payload)?;
    Ok(HandleValidClientMsg {
        msg_id: MsgId::new(),
        msg,
        origin: peer,
        auth: auth_proof,
        #[cfg(feature = "traceroute")]
        traceroute: Traceroute(Vec::new()),
    })
}

/// Extend the `Cmd` enum with some utilities for testing.
///
/// Since this is in a module marked as #[test], this functionality will only be present in the
/// testing context.
impl Cmd {
    /// Get the recipients for a `SendMsg` command.
    pub(crate) fn recipients(&self) -> Result<BTreeSet<Peer>> {
        match self {
            Cmd::SendMsg { recipients, .. } => match recipients {
                Peers::Single(peer) => {
                    let mut set = BTreeSet::new();
                    let _ = set.insert(*peer);
                    Ok(set)
                }
                Peers::Multiple(peers) => Ok(peers.clone()),
            },
            _ => Err(eyre!("A Cmd::SendMsg variant was expected")),
        }
    }

    /// Get the replicated data from a `NodeCmd` message.
    pub(crate) fn get_replicated_data(&self) -> Result<ReplicatedData> {
        match self {
            Cmd::SendMsg { msg, .. } => match msg {
                OutgoingMsg::Node(sys_msg) => match sys_msg {
                    NodeMsg::NodeCmd(node_cmd) => match node_cmd {
                        NodeCmd::ReplicateData(data) => {
                            if data.len() != 1 {
                                return Err(eyre!("Only 1 replicated data instance is expected"));
                            }
                            Ok(data[0].clone())
                        }
                        _ => Err(eyre!("A NodeCmd::ReplicateData variant was expected")),
                    },
                    _ => Err(eyre!("An NodeMsg::NodeCmd variant was expected")),
                },
                _ => Err(eyre!("An OutgoingMsg::System variant was expected")),
            },
            _ => Err(eyre!("A Cmd::SendMsg variant was expected")),
        }
    }

    /// Get a `ClientMsg` from a `Cmd::SendMsg` enum variant.
    pub(crate) fn get_service_msg(&self) -> Result<ClientMsg> {
        match self {
            Cmd::SendMsg { msg, .. } => match msg {
                OutgoingMsg::Client(service_msg) => Ok(service_msg.clone()),
                _ => Err(eyre!("A OutgoingMsg::Service variant was expected")),
            },
            _ => Err(eyre!("A Cmd::SendMsg variant was expected")),
        }
    }

    /// Get a `sn_interface::messaging::data::Error` from a `Cmd::SendMsg` enum variant.
    pub(crate) fn get_error(&self) -> Result<MessagingDataError> {
        match self {
            Cmd::SendMsg { msg, .. } => match msg {
                OutgoingMsg::Client(service_msg) => match service_msg {
                    ClientMsg::CmdError { error, .. } => Ok(error.clone()),
                    _ => Err(eyre!("A ClientMsg::CmdError variant was expected")),
                },
                _ => Err(eyre!("A OutgoingMsg::Service variant was expected")),
            },
            _ => Err(eyre!("A Cmd::SendMsg variant was expected")),
        }
    }
}
