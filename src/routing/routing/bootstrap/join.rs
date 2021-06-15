// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::verify_message;
use crate::messaging::{
    node::{
        JoinRejectionReason, JoinRequest, JoinResponse, ResourceProofResponse, RoutingMsg, Section,
        Variant,
    },
    DstInfo, DstLocation, MessageType, WireMsg,
};
use crate::routing::{
    ed25519,
    error::{Error, Result},
    messages::RoutingMsgUtils,
    node::Node,
    peer::PeerUtils,
    routing::comm::{Comm, ConnectionEvent, SendStatus},
    section::{SectionAuthorityProviderUtils, SectionUtils},
    FIRST_SECTION_MAX_AGE, FIRST_SECTION_MIN_AGE,
};
use futures::future;
use rand::seq::IteratorRandom;
use resource_proof::ResourceProof;
use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddr,
};
use tokio::sync::mpsc;
use tracing::Instrument;
use xor_name::{Prefix, XorName};

const BACKLOG_CAPACITY: usize = 100;

/// Join the network as new node.
///
/// NOTE: It's not guaranteed this function ever returns. This can happen due to messages being
/// lost in transit or other reasons. It's the responsibility of the caller to handle this case,
/// for example by using a timeout.
pub(crate) async fn join(
    node: Node,
    comm: &Comm,
    incoming_conns: &mut mpsc::Receiver<ConnectionEvent>,
    bootstrap_addr: SocketAddr,
) -> Result<(Node, Section, Vec<(RoutingMsg, SocketAddr, DstInfo)>)> {
    let (send_tx, send_rx) = mpsc::channel(1);

    let span = trace_span!("bootstrap", name = %node.name());

    let state = Join::new(node, send_tx, incoming_conns);

    future::join(state.run(bootstrap_addr), send_messages(send_rx, comm))
        .instrument(span)
        .await
        .0
}

struct Join<'a> {
    // Sender for outgoing messages.
    send_tx: mpsc::Sender<(MessageType, Vec<(XorName, SocketAddr)>)>,
    // Receiver for incoming messages.
    recv_rx: &'a mut mpsc::Receiver<ConnectionEvent>,
    node: Node,
    // Backlog for unknown messages
    backlog: VecDeque<(RoutingMsg, SocketAddr, DstInfo)>,
}

impl<'a> Join<'a> {
    fn new(
        node: Node,
        send_tx: mpsc::Sender<(MessageType, Vec<(XorName, SocketAddr)>)>,
        recv_rx: &'a mut mpsc::Receiver<ConnectionEvent>,
    ) -> Self {
        Self {
            send_tx,
            recv_rx,
            node,
            backlog: VecDeque::with_capacity(BACKLOG_CAPACITY),
        }
    }

    // Send `JoinRequest` and wait for the response. If the response is:
    // - `Retry`: repeat with the new info.
    // - `Redirect`: repeat with the new set of addresses.
    // - `ResourceChallenge`: carry out resource proof calculation.
    // - `Approval`: returns the initial `Section` value to use by this node,
    //    completing the bootstrap.
    async fn run(
        self,
        bootstrap_addr: SocketAddr,
    ) -> Result<(Node, Section, Vec<(RoutingMsg, SocketAddr, DstInfo)>)> {
        // Use our XorName as we do not know their name or section key yet.
        let section_key = bls::SecretKey::random().public_key();
        let dst_xorname = self.node.name();

        let recipients = vec![(dst_xorname, bootstrap_addr)];

        self.join(section_key, recipients).await
    }

    async fn join(
        mut self,
        mut section_key: bls::PublicKey,
        mut recipients: Vec<(XorName, SocketAddr)>,
    ) -> Result<(Node, Section, Vec<(RoutingMsg, SocketAddr, DstInfo)>)> {
        // We send a first join request to obtain the resource challenge, which
        // we will then use to generate the challenge proof and send the
        // `JoinRequest` again with it.
        let join_request = JoinRequest {
            section_key,
            resource_proof_response: None,
        };

        self.send_join_requests(join_request, &recipients, section_key)
            .await?;

        // Avoid sending more than one request to the same peer.
        let mut used_recipient = HashSet::<SocketAddr>::new();

        loop {
            used_recipient.extend(recipients.iter().map(|(_, addr)| addr));

            let (response, sender, dst_info) = self.receive_join_response().await?;

            match response {
                JoinResponse::Rejected(JoinRejectionReason::NodeNotReachable(addr)) => {
                    error!(
                        "Node cannot join the network since it is not externally reachable: {}",
                        addr
                    );
                    return Err(Error::NodeNotReachable(addr));
                }
                JoinResponse::Rejected(JoinRejectionReason::JoinsDisallowed) => {
                    error!("Network is set to not taking any new joining node, try join later.");
                    return Err(Error::TryJoinLater);
                }
                JoinResponse::Approval {
                    section_auth,
                    genesis_key,
                    section_chain,
                    ..
                } => {
                    return Ok((
                        self.node,
                        Section::new(genesis_key, section_chain, section_auth)?,
                        self.backlog.into_iter().collect(),
                    ));
                }
                JoinResponse::Retry(section_auth) => {
                    if section_auth.section_key() == section_key {
                        debug!("Ignoring JoinResponse::Retry with invalid section authority provider key");
                        continue;
                    }

                    let new_recipients: Vec<(XorName, SocketAddr)> = section_auth
                        .elders
                        .iter()
                        .map(|(name, addr)| (*name, *addr))
                        .collect();

                    let prefix = section_auth.prefix;

                    // For the first section, using age random among 6 to 100 to avoid
                    // relocating too many nodes at the same time.
                    if prefix.is_empty() && self.node.age() < FIRST_SECTION_MIN_AGE {
                        let age: u8 = (FIRST_SECTION_MIN_AGE..FIRST_SECTION_MAX_AGE)
                            .choose(&mut rand::thread_rng())
                            .unwrap_or(FIRST_SECTION_MAX_AGE);

                        let new_keypair =
                            ed25519::gen_keypair(&Prefix::default().range_inclusive(), age);
                        let new_name = ed25519::name(&new_keypair.public);

                        info!("Setting Node name to {}", new_name);
                        self.node = Node::new(new_keypair, self.node.addr);
                    }

                    if prefix.matches(&self.node.name()) {
                        info!(
                            "Newer Join response for our prefix {:?} from {:?}",
                            section_auth, sender
                        );
                        section_key = section_auth.section_key();
                        let join_request = JoinRequest {
                            section_key,
                            resource_proof_response: None,
                        };

                        recipients = new_recipients;
                        self.send_join_requests(join_request, &recipients, section_key)
                            .await?;
                    } else {
                        warn!(
                            "Newer Join response not for our prefix {:?} from {:?}",
                            section_auth, sender,
                        );
                    }
                }
                JoinResponse::Redirect(section_auth) => {
                    if section_auth.section_key() == section_key {
                        continue;
                    }

                    // Ignore already used recipients
                    let new_recipients: Vec<(XorName, SocketAddr)> = section_auth
                        .elders
                        .iter()
                        .filter(|(_, addr)| !used_recipient.contains(addr))
                        .map(|(name, addr)| (*name, *addr))
                        .collect();

                    if new_recipients.is_empty() {
                        debug!("Joining redirected to the same set of peers we already contacted - ignoring response");
                        continue;
                    } else {
                        info!(
                            "Joining redirected to another set of peers: {:?}",
                            new_recipients,
                        );
                    }

                    if section_auth.prefix.matches(&self.node.name()) {
                        info!(
                            "Newer Join response for our prefix {:?} from {:?}",
                            section_auth, sender
                        );
                        section_key = section_auth.section_key();
                        let join_request = JoinRequest {
                            section_key,
                            resource_proof_response: None,
                        };

                        recipients = new_recipients;
                        self.send_join_requests(join_request, &recipients, section_key)
                            .await?;
                    } else {
                        warn!(
                            "Newer Join response not for our prefix {:?} from {:?}",
                            section_auth, sender,
                        );
                    }
                }
                JoinResponse::ResourceChallenge {
                    data_size,
                    difficulty,
                    nonce,
                    nonce_signature,
                } => {
                    let rp = ResourceProof::new(data_size, difficulty);
                    let data = rp.create_proof_data(&nonce);
                    let mut prover = rp.create_prover(data.clone());
                    let solution = prover.solve();

                    let join_request = JoinRequest {
                        section_key,
                        resource_proof_response: Some(ResourceProofResponse {
                            solution,
                            data,
                            nonce,
                            nonce_signature,
                        }),
                    };
                    let recipients = &[(dst_info.dst, sender)];
                    self.send_join_requests(join_request, recipients, section_key)
                        .await?;
                }
            }
        }
    }

    async fn send_join_requests(
        &mut self,
        join_request: JoinRequest,
        recipients: &[(XorName, SocketAddr)],
        section_key: bls::PublicKey,
    ) -> Result<()> {
        info!("Sending {:?} to {:?}", join_request, recipients);

        let variant = Variant::JoinRequest(Box::new(join_request));
        let message = RoutingMsg::single_src(
            &self.node,
            DstLocation::DirectAndUnrouted,
            variant,
            section_key,
        )?;

        let _ = self
            .send_tx
            .send((
                MessageType::Routing {
                    msg: message,
                    dst_info: DstInfo {
                        dst: recipients[0].0,
                        dst_section_pk: section_key,
                    },
                },
                recipients.to_vec(),
            ))
            .await;

        Ok(())
    }

    async fn receive_join_response(&mut self) -> Result<(JoinResponse, SocketAddr, DstInfo)> {
        let dst = self.node.name();

        while let Some(event) = self.recv_rx.recv().await {
            // we are interested only in `JoinResponse` type of messages
            let (routing_msg, dst_info, join_response, sender) = match event {
                ConnectionEvent::Received((sender, bytes)) => match WireMsg::deserialize(bytes) {
                    Ok(MessageType::Node { .. })
                    | Ok(MessageType::Client { .. })
                    | Ok(MessageType::SectionInfo { .. }) => continue,
                    Ok(MessageType::Routing { msg, dst_info }) => {
                        if let Variant::JoinResponse(resp) = &msg.variant {
                            let join_response = resp.clone();
                            (msg, dst_info, *join_response, sender)
                        } else {
                            self.backlog_message(msg, sender, dst_info);
                            continue;
                        }
                    }
                    Err(error) => {
                        debug!("Failed to deserialize message: {}", error);
                        continue;
                    }
                },
                ConnectionEvent::Disconnected(_) => continue,
            };

            match join_response {
                JoinResponse::Rejected(JoinRejectionReason::NodeNotReachable(_))
                | JoinResponse::Rejected(JoinRejectionReason::JoinsDisallowed) => {
                    return Ok((join_response, sender, dst_info));
                }
                JoinResponse::Retry(ref section_auth)
                | JoinResponse::Redirect(ref section_auth) => {
                    if !section_auth.prefix.matches(&dst) {
                        error!("Invalid JoinResponse bad prefix: {:?}", join_response);
                        continue;
                    }

                    if section_auth.elders.is_empty() {
                        error!(
                            "Invalid JoinResponse, empty list of Elders: {:?}",
                            join_response
                        );
                        continue;
                    }

                    if !verify_message(&routing_msg, None) {
                        continue;
                    }

                    return Ok((join_response, sender, dst_info));
                }
                JoinResponse::ResourceChallenge { .. } => {
                    if !verify_message(&routing_msg, None) {
                        continue;
                    }

                    return Ok((join_response, sender, dst_info));
                }
                JoinResponse::Approval {
                    ref section_auth,
                    ref node_state,
                    ..
                } => {
                    if node_state.value.peer.name() != &self.node.name() {
                        trace!("Ignore NodeApproval not for us");
                        continue;
                    }

                    if !verify_message(&routing_msg, None) {
                        continue;
                    }

                    trace!(
                        "This node has been approved to join the network at {:?}!",
                        section_auth.value.prefix,
                    );

                    return Ok((join_response, sender, dst_info));
                }
            }
        }

        error!("RoutingMsg sender unexpectedly closed");
        // TODO: consider more specific error here (e.g. `BootstrapInterrupted`)
        Err(Error::InvalidState)
    }

    fn backlog_message(&mut self, message: RoutingMsg, sender: SocketAddr, dst_info: DstInfo) {
        while self.backlog.len() >= BACKLOG_CAPACITY {
            let _ = self.backlog.pop_front();
        }

        self.backlog.push_back((message, sender, dst_info))
    }
}

// Keep reading messages from `rx` and send them using `comm`.
async fn send_messages(
    mut rx: mpsc::Receiver<(MessageType, Vec<(XorName, SocketAddr)>)>,
    comm: &Comm,
) -> Result<()> {
    while let Some((message, recipients)) = rx.recv().await {
        match comm
            .send(&recipients, recipients.len(), message.clone())
            .await
        {
            Ok(SendStatus::AllRecipients) | Ok(SendStatus::MinDeliveryGroupSizeReached(_)) => {}
            Ok(SendStatus::MinDeliveryGroupSizeFailed(recipients)) => {
                error!("Failed to send message {:?} to {:?}", message, recipients)
            }
            Err(err) => {
                error!(
                    "Failed to send message {:?} to {:?}: {:?}",
                    message, recipients, err
                )
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{node::NodeState, SectionAuthorityProvider};
    use crate::routing::{
        dkg::test_utils::*,
        error::Error as RoutingError,
        messages::RoutingMsgUtils,
        section::test_utils::*,
        section::{NodeStateUtils, SectionAuthorityProviderUtils},
        ELDER_SIZE, MIN_ADULT_AGE, MIN_AGE,
    };
    use anyhow::{anyhow, Error, Result};
    use assert_matches::assert_matches;
    use futures::{
        future::{self, Either},
        pin_mut,
    };
    use secured_linked_list::SecuredLinkedList;
    use std::collections::BTreeMap;
    use tokio::task;

    #[tokio::test]
    async fn join_as_adult() -> Result<()> {
        let (send_tx, mut send_rx) = mpsc::channel(1);
        let (recv_tx, mut recv_rx) = mpsc::channel(1);

        let (section_auth, mut nodes, sk_set) =
            gen_section_authority_provider(Prefix::default(), ELDER_SIZE);
        let bootstrap_node = nodes.remove(0);
        let bootstrap_addr = bootstrap_node.addr;

        let sk = sk_set.secret_key();
        let pk = sk.public_key();

        // Node in first section has to have an age higher than MIN_ADULT_AGE
        // Otherwise during the bootstrap process, node will change its id and age.
        let node_age = MIN_AGE + 2;
        let node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), node_age),
            gen_addr(),
        );
        let peer = node.peer();
        let state = Join::new(node, send_tx, &mut recv_rx);

        // Create the bootstrap task, but don't run it yet.
        let bootstrap = async move { state.run(bootstrap_addr).await.map_err(Error::from) };

        // Create the task that executes the body of the test, but don't run it either.
        let others = async {
            // Receive JoinRequest
            let (message, recipients) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            let bootstrap_addrs: Vec<SocketAddr> =
                recipients.iter().map(|(_name, addr)| *addr).collect();
            assert_eq!(bootstrap_addrs, [bootstrap_addr]);

            let (message, dst_info) = assert_matches!(message, MessageType::Routing { msg, dst_info } =>
                (msg, dst_info));

            assert_eq!(dst_info.dst, *peer.name());
            assert_matches!(message.variant, Variant::JoinRequest(request) => {
                assert!(request.resource_proof_response.is_none());
            });

            // Send JoinResponse::Retry with section auth provider info
            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Retry(section_auth.clone()))),
                &bootstrap_node,
                section_auth.section_key(),
                *peer.name(),
            )?;

            // Receive the second JoinRequest with correct section info
            let (message, recipients) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;
            let (message, dst_info) = assert_matches!(message, MessageType::Routing { msg, dst_info } =>
                (msg, dst_info));

            assert_eq!(dst_info.dst_section_pk, pk);
            itertools::assert_equal(
                recipients,
                section_auth
                    .elders()
                    .iter()
                    .map(|(name, addr)| (*name, *addr))
                    .collect::<Vec<_>>(),
            );
            assert_matches!(message.variant, Variant::JoinRequest(request) => {
                assert_eq!(request.section_key, pk);
            });

            // Send JoinResponse::Approval
            let section_auth = section_signed(sk, section_auth.clone())?;
            let node_state = section_signed(sk, NodeState::joined(peer))?;
            let proof_chain = SecuredLinkedList::new(pk);
            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Approval {
                    genesis_key: pk,
                    section_auth: section_auth.clone(),
                    node_state,
                    section_chain: proof_chain,
                })),
                &bootstrap_node,
                section_auth.value.section_key(),
                *peer.name(),
            )?;

            Ok(())
        };

        // Drive both tasks to completion concurrently (but on the same thread).
        let ((node, section, _backlog), _) = future::try_join(bootstrap, others).await?;

        assert_eq!(*section.authority_provider(), section_auth);
        assert_eq!(*section.chain().last_key(), pk);
        assert_eq!(node.age(), node_age);

        Ok(())
    }

    #[tokio::test]
    async fn join_receive_redirect_response() -> Result<()> {
        let (send_tx, mut send_rx) = mpsc::channel(1);
        let (recv_tx, mut recv_rx) = mpsc::channel(1);

        let (section_auth, mut nodes, sk_set) =
            gen_section_authority_provider(Prefix::default(), ELDER_SIZE);
        let bootstrap_node = nodes.remove(0);
        let pk_set = sk_set.public_keys();

        let node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE),
            gen_addr(),
        );
        let name = node.name();
        let state = Join::new(node, send_tx, &mut recv_rx);

        let bootstrap_task = state.run(bootstrap_node.addr);
        let test_task = async move {
            // Receive JoinRequest
            let (message, recipients) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            assert_eq!(
                recipients
                    .into_iter()
                    .map(|peer| peer.1)
                    .collect::<Vec<_>>(),
                vec![bootstrap_node.addr]
            );

            assert_matches!(message, MessageType::Routing { msg, .. } =>
                assert_matches!(msg.variant, Variant::JoinRequest{..}));

            // Send JoinResponse::Redirect
            let new_bootstrap_addrs: BTreeMap<_, _> = (0..ELDER_SIZE)
                .map(|_| (XorName::random(), gen_addr()))
                .collect();

            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Redirect(SectionAuthorityProvider {
                    prefix: Prefix::default(),
                    public_key_set: pk_set.clone(),
                    elders: new_bootstrap_addrs.clone(),
                }))),
                &bootstrap_node,
                section_auth.section_key(),
                name,
            )?;
            task::yield_now().await;

            // Receive new JoinRequest with redirected bootstrap contacts
            let (message, recipients) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            assert_eq!(
                recipients
                    .into_iter()
                    .map(|peer| peer.1)
                    .collect::<Vec<_>>(),
                new_bootstrap_addrs
                    .iter()
                    .map(|(_, addr)| *addr)
                    .collect::<Vec<_>>()
            );

            let (message, dst_info) = assert_matches!(message, MessageType::Routing { msg, dst_info } =>
                (msg, dst_info));

            assert_eq!(dst_info.dst_section_pk, pk_set.public_key());
            assert_matches!(message.variant, Variant::JoinRequest(req) => {
                assert_eq!(req.section_key, pk_set.public_key());
            });

            Ok(())
        };

        pin_mut!(bootstrap_task);
        pin_mut!(test_task);

        match future::select(bootstrap_task, test_task).await {
            Either::Left(_) => unreachable!(),
            Either::Right((output, _)) => output,
        }
    }

    #[tokio::test]
    async fn join_invalid_redirect_response() -> Result<()> {
        let (send_tx, mut send_rx) = mpsc::channel(1);
        let (recv_tx, mut recv_rx) = mpsc::channel(1);

        let (section_auth, mut nodes, sk_set) =
            gen_section_authority_provider(Prefix::default(), ELDER_SIZE);
        let bootstrap_node = nodes.remove(0);
        let pk_set = sk_set.public_keys();

        let node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE),
            gen_addr(),
        );
        let node_name = node.name();
        let state = Join::new(node, send_tx, &mut recv_rx);

        let bootstrap_task = state.run(bootstrap_node.addr);
        let test_task = async {
            let (message, _) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            assert_matches!(message, MessageType::Routing { msg, .. } =>
                    assert_matches!(msg.variant, Variant::JoinRequest{..}));

            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Redirect(SectionAuthorityProvider {
                    prefix: Prefix::default(),
                    public_key_set: pk_set.clone(),
                    elders: BTreeMap::new(),
                }))),
                &bootstrap_node,
                section_auth.section_key(),
                node_name,
            )?;
            task::yield_now().await;

            let addrs = (0..ELDER_SIZE)
                .map(|_| (XorName::random(), gen_addr()))
                .collect();

            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Redirect(SectionAuthorityProvider {
                    prefix: Prefix::default(),
                    public_key_set: pk_set.clone(),
                    elders: addrs,
                }))),
                &bootstrap_node,
                section_auth.section_key(),
                node_name,
            )?;
            task::yield_now().await;

            let (message, _) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            assert_matches!(message, MessageType::Routing { msg, .. } =>
                        assert_matches!(msg.variant, Variant::JoinRequest{..}));

            Ok(())
        };

        pin_mut!(bootstrap_task);
        pin_mut!(test_task);

        match future::select(bootstrap_task, test_task).await {
            Either::Left(_) => unreachable!(),
            Either::Right((output, _)) => output,
        }
    }

    #[tokio::test]
    async fn join_disallowed_response() -> Result<()> {
        let (send_tx, mut send_rx) = mpsc::channel(1);
        let (recv_tx, mut recv_rx) = mpsc::channel(1);

        let (section_auth, mut nodes, _) =
            gen_section_authority_provider(Prefix::default(), ELDER_SIZE);
        let bootstrap_node = nodes.remove(0);

        let node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE),
            gen_addr(),
        );

        let node_name = node.name();
        let state = Join::new(node, send_tx, &mut recv_rx);

        let bootstrap_task = state.run(bootstrap_node.addr);
        let test_task = async {
            let (message, _) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("JoinRequest was not received"))?;

            assert_matches!(message, MessageType::Routing { msg, .. } =>
                            assert_matches!(msg.variant, Variant::JoinRequest{..}));

            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Rejected(
                    JoinRejectionReason::JoinsDisallowed,
                ))),
                &bootstrap_node,
                section_auth.section_key(),
                node_name,
            )?;

            Ok(())
        };

        let (join_result, test_result) = future::join(bootstrap_task, test_task).await;

        if let Err(RoutingError::TryJoinLater) = join_result {
        } else {
            return Err(anyhow!("Not getting an execpted network rejection."));
        }

        test_result
    }

    #[tokio::test]
    async fn join_invalid_retry_prefix_response() -> Result<()> {
        let (send_tx, mut send_rx) = mpsc::channel(1);
        let (recv_tx, mut recv_rx) = mpsc::channel(1);

        let bootstrap_node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE),
            gen_addr(),
        );

        let node = Node::new(
            ed25519::gen_keypair(&Prefix::default().range_inclusive(), MIN_ADULT_AGE),
            gen_addr(),
        );
        let node_name = node.name();

        let (good_prefix, bad_prefix) = {
            let p0 = Prefix::default().pushed(false);
            let p1 = Prefix::default().pushed(true);

            if node.name().bit(0) {
                (p1, p0)
            } else {
                (p0, p1)
            }
        };

        let state = Join::new(node, send_tx, &mut recv_rx);

        let section_key = bls::SecretKey::random().public_key();
        let elders = (0..ELDER_SIZE)
            .map(|_| (good_prefix.substituted_in(rand::random()), gen_addr()))
            .collect();
        let join_task = state.join(section_key, elders);

        let test_task = async {
            let (message, _) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("RoutingMsg was not received"))?;

            let message = assert_matches!(message, MessageType::Routing{ msg, .. } => msg);
            assert_matches!(message.variant, Variant::JoinRequest(_));

            // Send `Retry` with bad prefix
            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Retry(
                    gen_section_authority_provider(bad_prefix, ELDER_SIZE).0,
                ))),
                &bootstrap_node,
                section_key,
                node_name,
            )?;
            task::yield_now().await;

            // Send `Retry` with good prefix
            send_response(
                &recv_tx,
                Variant::JoinResponse(Box::new(JoinResponse::Retry(
                    gen_section_authority_provider(good_prefix, ELDER_SIZE).0,
                ))),
                &bootstrap_node,
                section_key,
                node_name,
            )?;

            let (message, _) = send_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("RoutingMsg was not received"))?;

            let message = assert_matches!(message, MessageType::Routing{ msg, .. } => msg);
            assert_matches!(message.variant, Variant::JoinRequest(_));

            Ok(())
        };

        pin_mut!(join_task);
        pin_mut!(test_task);

        match future::select(join_task, test_task).await {
            Either::Left(_) => unreachable!(),
            Either::Right((output, _)) => output,
        }
    }

    // test helper
    fn send_response(
        recv_tx: &mpsc::Sender<ConnectionEvent>,
        variant: Variant,
        bootstrap_node: &Node,
        section_key: bls::PublicKey,
        node_name: XorName,
    ) -> Result<()> {
        let message = RoutingMsg::single_src(
            bootstrap_node,
            DstLocation::DirectAndUnrouted,
            variant,
            section_key,
        )?;

        recv_tx.try_send(ConnectionEvent::Received((
            bootstrap_node.addr,
            MessageType::Routing {
                msg: message,
                dst_info: DstInfo {
                    dst: node_name,
                    dst_section_pk: section_key,
                },
            }
            .serialize()?,
        )))?;

        Ok(())
    }
}
