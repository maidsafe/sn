// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{
    flow_ctrl::cmds::Cmd,
    messaging::{OutgoingMsg, Peers},
    Error, Event, MembershipEvent, MyNode, Result, StateSnapshot,
};

use bls::PublicKey as BlsPublicKey;
use qp2p::{SendStream, UsrMsgBytes};
#[cfg(feature = "traceroute")]
use sn_interface::messaging::Traceroute;
use sn_interface::{
    messaging::{
        system::{AntiEntropyKind, NodeCmd, NodeMsg, SectionSigned},
        MsgType, WireMsg,
    },
    network_knowledge::{NodeState, SectionTreeUpdate},
    types::{log_markers::LogMarker, Peer, PublicKey},
};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::Mutex;
use xor_name::{Prefix, XorName};

impl MyNode {
    /// Send `AntiEntropy` update message to all nodes in our own section.
    pub(crate) fn send_ae_update_to_our_section(&self) -> Result<Option<Cmd>> {
        let our_name = self.info().name();
        let recipients: BTreeSet<_> = self
            .network_knowledge
            .section_members()
            .into_iter()
            .filter(|info| info.name() != our_name)
            .map(|info| *info.peer())
            .collect();

        if recipients.is_empty() {
            warn!("No peers of our section found in our network knowledge to send AE-Update");
            return Ok(None);
        }

        let leaf = self.section_chain().last_key()?;
        // The previous PK which is likely what adults know
        match self.section_chain().get_parent_key(&leaf) {
            Ok(prev_pk) => {
                let prev_pk = prev_pk.unwrap_or(*self.section_chain().genesis_key());
                Ok(Some(self.send_ae_update_to_nodes(recipients, prev_pk)))
            }
            Err(_) => {
                error!("SectionsDAG fields went out of sync");
                Ok(None)
            }
        }
    }

    /// Send `AntiEntropy` update message to the specified nodes.
    pub(crate) fn send_ae_update_to_nodes(
        &self,
        recipients: BTreeSet<Peer>,
        section_pk: BlsPublicKey,
    ) -> Cmd {
        let members = self.network_knowledge.section_signed_members();

        let ae_msg = self.generate_ae_msg(Some(section_pk), AntiEntropyKind::Update { members });

        self.send_system_msg(ae_msg, Peers::Multiple(recipients))
    }

    /// Send `MetadataExchange` packet to the specified nodes
    pub(crate) fn send_metadata_updates(&self, recipients: BTreeSet<Peer>, prefix: &Prefix) -> Cmd {
        let metadata = self.get_metadata_of(prefix);
        self.send_system_msg(
            NodeMsg::NodeCmd(NodeCmd::ReceiveMetadata { metadata }),
            Peers::Multiple(recipients),
        )
    }

    #[instrument(skip_all)]
    /// Send AntiEntropy update message to the nodes in our sibling section.
    pub(crate) fn send_updates_to_sibling_section(
        &self,
        our_prev_state: &StateSnapshot,
    ) -> Result<Vec<Cmd>> {
        debug!("{}", LogMarker::AeSendUpdateToSiblings);
        let sibling_prefix = self.network_knowledge.prefix().sibling();
        if let Some(sibling_sap) = self
            .network_knowledge
            .section_tree()
            .get_signed(&sibling_prefix)
        {
            let promoted_sibling_elders: BTreeSet<_> = sibling_sap
                .elders()
                .filter(|peer| !our_prev_state.elders.contains(&peer.name()))
                .cloned()
                .collect();

            if promoted_sibling_elders.is_empty() {
                debug!("No promoted siblings found in our network knowledge to send AE-Update");
                return Ok(vec![]);
            }

            // Using previous_key as dst_section_key as newly promoted
            // sibling Elders shall still in the state of pre-split.
            let previous_section_key = our_prev_state.section_key;
            let sibling_prefix = sibling_sap.prefix();

            let mut cmds =
                vec![self.send_metadata_updates(promoted_sibling_elders.clone(), &sibling_prefix)];

            // Also send AE update to sibling section's new Elders
            cmds.push(self.send_ae_update_to_nodes(promoted_sibling_elders, previous_section_key));

            Ok(cmds)
        } else {
            error!("Failed to get sibling SAP during split.");
            Ok(vec![])
        }
    }

    // Private helper to generate AntiEntropy message to update
    // a peer abot our SAP, with proof_chain and members list.
    fn generate_ae_msg(
        &self,
        dst_section_key: Option<BlsPublicKey>,
        kind: AntiEntropyKind,
    ) -> NodeMsg {
        let signed_sap = self.network_knowledge.signed_sap();

        let proof_chain = dst_section_key
            .and_then(|key| {
                self.network_knowledge
                    .get_proof_chain_to_current_section(&key)
                    .ok()
            })
            .unwrap_or_else(|| self.network_knowledge.section_chain());

        let section_tree_update = SectionTreeUpdate::new(signed_sap, proof_chain);

        NodeMsg::AntiEntropy {
            section_tree_update,
            kind,
        }
    }

    // Update's Network Knowledge
    // returns
    //   Ok(true) if the update had new valid information
    //   Ok(false) if the update was valid but did not contain new information
    //   Err(_) if the update was invalid
    pub(crate) fn update_network_knowledge(
        &mut self,
        section_tree_update: SectionTreeUpdate,
        members: Option<BTreeSet<SectionSigned<NodeState>>>,
    ) -> Result<bool> {
        let our_name = self.info().name();
        let sap = section_tree_update.signed_sap.clone();

        let we_have_a_share_of_this_key = self
            .section_keys_provider
            .key_share(&sap.section_key())
            .is_ok();

        // check we should be _becoming_ an elder
        let we_should_become_an_elder = sap.contains_elder(&our_name);

        trace!("we_have_a_share_of_this_key: {we_have_a_share_of_this_key}, we_should_become_an_elder: {we_should_become_an_elder}");

        // This prevent us from updating our NetworkKnowledge based on an AE message where
        // we don't have the key share for the new SAP, making this node unable to sign section
        // messages and possibly being kicked out of the group of Elders.
        if we_should_become_an_elder && !we_have_a_share_of_this_key {
            warn!("We should be an elder, but we're missing the keyshare!, ignoring update to wait until we have our keyshare");
            return Ok(false);
        };

        Ok(self.network_knowledge.update_knowledge_if_valid(
            section_tree_update,
            members,
            &our_name,
        )?)
    }

    #[instrument(skip_all)]
    pub(crate) async fn handle_anti_entropy_msg(
        &mut self,
        section_tree_update: SectionTreeUpdate,
        kind: AntiEntropyKind,
        sender: Peer,
        #[cfg(feature = "traceroute")] traceroute: Traceroute,
    ) -> Result<Vec<Cmd>> {
        let snapshot = self.state_snapshot();
        let sap = section_tree_update.signed_sap.value.clone();

        let members = if let AntiEntropyKind::Update { members } = kind.clone() {
            Some(members)
        } else {
            None
        };

        let updated = self.update_network_knowledge(section_tree_update, members)?;

        // always run this, only changes will trigger events
        let mut cmds = self.update_on_elder_change(&snapshot)?;

        // Only trigger reorganize data when there is a membership change happens.
        if updated && self.is_not_elder() {
            // only done if adult, since as an elder we dont want to get any more
            // data for our name (elders will eventually be caching data in general)
            cmds.push(self.ask_for_any_new_data().await);
        }

        if updated {
            self.write_section_tree();
            let prefix = sap.prefix();
            info!("SectionTree written to disk with update for prefix {prefix:?}");

            // check if we've been kicked out of the section
            if snapshot.members.contains(&self.name())
                && !self.state_snapshot().members.contains(&self.name())
            {
                error!("Detected that we've been removed from the section");
                // move off thread to keep fn sync
                let event_sender = self.event_sender.clone();
                let _handle = tokio::spawn(async move {
                    event_sender
                        .send(Event::Membership(MembershipEvent::RemovedFromSection))
                        .await;
                });

                return Err(Error::RemovedFromSection);
            }
        }

        // Check if we need to resend any messsages and who should we send it to.
        let (bounced_msg, response_peer) = match kind {
            AntiEntropyKind::Update { .. } => {
                // log the msg as received. Elders track this for other elders in dysfunction
                self.dysfunction_tracking
                    .ae_update_msg_received(&sender.name());
                return Ok(cmds);
            } // Nope, bail early
            AntiEntropyKind::Retry { bounced_msg } => {
                trace!("{}", LogMarker::AeResendAfterRetry);
                (bounced_msg, sender)
            }
            AntiEntropyKind::Redirect { bounced_msg } => {
                // We choose the Elder closest to the dst section key,
                // just to pick one of them in an arbitrary but deterministic fashion.
                let target_name = XorName::from(PublicKey::Bls(sap.section_key()));

                let chosen_dst_elder = if let Some(dst) = sap
                    .elders()
                    .max_by(|lhs, rhs| target_name.cmp_distance(&lhs.name(), &rhs.name()))
                {
                    *dst
                } else {
                    error!("Failed to find closest Elder to resend msg upon AE-Redirect response.");
                    return Ok(cmds);
                };

                trace!("{}", LogMarker::AeResendAfterRedirect);

                (bounced_msg, chosen_dst_elder)
            }
        };

        let (msg_to_resend, msg_id, dst) = match WireMsg::deserialize(bounced_msg)? {
            MsgType::Node {
                msg, msg_id, dst, ..
            } => (msg, msg_id, dst),
            _ => {
                warn!("Non System MsgType received in AE response. We do not handle any other type in AE msgs yet.");
                return Ok(cmds);
            }
        };

        // If the new SAP's section key is the same as the section key set when the
        // bounced message was originally sent, we just drop it.
        if dst.section_key == sap.section_key() {
            error!("Dropping bounced msg ({sender:?}) received in AE-Retry from {msg_id:?} as suggested new dst section key is the same as previously sent: {:?}", sap.section_key());
            return Ok(cmds);
        }

        trace!("Resend Original {msg_id:?} to {response_peer:?} with {msg_to_resend:?}");

        if cfg!(feature = "traceroute") {
            cmds.push(self.trace_system_msg(
                msg_to_resend,
                Peers::Single(response_peer),
                #[cfg(feature = "traceroute")]
                traceroute,
            ))
        } else {
            cmds.push(self.send_system_msg(msg_to_resend, Peers::Single(response_peer)))
        }

        Ok(cmds)
    }

    // If entropy is found, determine the msg to send in order to
    // bring the sender's knowledge about us up to date.
    pub(crate) fn check_for_entropy(
        &self,
        wire_msg: &WireMsg,
        dst_section_key: &BlsPublicKey,
        dst_name: XorName,
        sender: &Peer,
        send_stream: Option<Arc<Mutex<SendStream>>>,
    ) -> Result<Option<Cmd>> {
        // Check if the message has reached the correct section,
        // if not, we'll need to respond with AE
        let our_prefix = self.network_knowledge.prefix();

        // Let's try to find a section closer to the destination, if it's not for us.
        if !self.network_knowledge.prefix().matches(&dst_name) {
            debug!(
                "AE: prefix not matching. We are: {:?}, they sent to: {:?}",
                our_prefix, dst_name
            );
            return match self.network_knowledge.closest_signed_sap(&dst_name) {
                Some((signed_sap, proof_chain)) => {
                    info!("Found a better matching prefix {:?}", signed_sap.prefix());
                    let bounced_msg = wire_msg.serialize()?;
                    let section_tree_update =
                        SectionTreeUpdate::new(signed_sap.clone(), proof_chain);
                    // Redirect to the closest section
                    let ae_msg = NodeMsg::AntiEntropy {
                        section_tree_update,
                        kind: AntiEntropyKind::Redirect { bounced_msg },
                    };

                    trace!(
                        "{} entropy found. {sender:?} should be updated",
                        LogMarker::AeSendRedirect
                    );

                    // client response, so send it over stream
                    if send_stream.is_some() {
                        return Ok(Some(Cmd::send_msg_via_response_stream(
                            OutgoingMsg::Node(ae_msg),
                            Peers::Single(*sender),
                            send_stream,
                        )));
                    } else {
                        return Ok(Some(Cmd::send_msg(
                            OutgoingMsg::Node(ae_msg),
                            Peers::Single(*sender),
                        )));
                    }
                }
                None => {
                    warn!("Our SectionTree is empty");
                    // TODO: instead of just dropping the message, don't we actually need
                    // to get up to date info from other Elders in our section as it may be
                    // a section key we are not aware of yet?
                    // ...and once we acquired new key/s we attempt AE check again?
                    warn!(
                        "Anti-Entropy: cannot reply with redirect msg for dst_name {:?} and key {:?} to a closest section.",
                        dst_name, dst_section_key
                    );

                    Err(Error::NoMatchingSection)
                }
            };
        }

        let section_key = self.network_knowledge.section_key();
        trace!(
            "Performing AE checks, provided pk was: {:?} ours is: {:?}",
            dst_section_key,
            section_key
        );

        if dst_section_key == &section_key {
            // Destination section key matches our current section key
            return Ok(None);
        }

        let bounced_msg = wire_msg.serialize()?;

        let ae_msg = self.generate_ae_msg(
            Some(*dst_section_key),
            AntiEntropyKind::Retry { bounced_msg },
        );

        trace!(
            "CMD of Sending AE message to {:?} with {:?}",
            sender,
            ae_msg
        );

        // client response, so send it over stream
        if send_stream.is_some() {
            Ok(Some(Cmd::send_msg_via_response_stream(
                OutgoingMsg::Node(ae_msg),
                Peers::Single(*sender),
                send_stream,
            )))
        } else {
            Ok(Some(Cmd::send_msg(
                OutgoingMsg::Node(ae_msg),
                Peers::Single(*sender),
            )))
        }
    }

    // Generate an AE redirect cmd for the given message
    pub(crate) fn ae_redirect_to_our_elders(
        &self,
        sender: Peer,
        bounced_msg: UsrMsgBytes,
    ) -> Result<Cmd> {
        trace!(
            "{} in ae_redirect to elders for {sender:?} ",
            LogMarker::AeSendRedirect
        );

        let ae_msg = self.generate_ae_msg(None, AntiEntropyKind::Redirect { bounced_msg });

        Ok(Cmd::send_msg(
            OutgoingMsg::Node(ae_msg),
            Peers::Single(sender),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::node::{
        cfg::create_test_max_capacity_and_root_storage,
        flow_ctrl::{event_channel, tests::network_utils::create_comm},
        MIN_ADULT_AGE,
    };
    use crate::UsedSpace;
    use sn_interface::{
        messaging::system::SectionSigned, network_knowledge::SectionAuthorityProvider,
    };

    use sn_interface::{
        elder_count,
        messaging::{Dst, MsgId, MsgKind},
        network_knowledge::{
            test_utils::{gen_addr, random_sap, section_signed},
            MyNodeInfo, SectionKeyShare, SectionKeysProvider, SectionsDAG,
        },
        types::keys::ed25519,
    };

    use assert_matches::assert_matches;
    use bls::SecretKey;
    use eyre::{Context, Result};
    use xor_name::Prefix;

    #[tokio::test]
    async fn ae_everything_up_to_date() -> Result<()> {
        // Construct a local task set that can run `!Send` futures.
        let local = tokio::task::LocalSet::new();

        // Run the local task set.
        local
            .run_until(async move {
                let env = Env::new().await?;
                let our_prefix = env.node.network_knowledge().prefix();
                let msg =
                    env.create_msg(&our_prefix, env.node.network_knowledge().section_key())?;
                let sender = env.node.info().peer();
                let dst_name = our_prefix.substituted_in(xor_name::rand::random());
                let dst_section_key = env.node.network_knowledge().section_key();

                let cmd =
                    env.node
                        .check_for_entropy(&msg, &dst_section_key, dst_name, &sender, None)?;

                assert!(cmd.is_none());
                Result::<()>::Ok(())
            })
            .await
    }

    #[tokio::test]
    async fn ae_redirect_to_other_section() -> Result<()> {
        // Construct a local task set that can run `!Send` futures.
        let local = tokio::task::LocalSet::new();

        // Run the local task set.
        local.run_until(async move {
            let mut env = Env::new().await?;

            let other_sk = bls::SecretKey::random();
            let other_pk = other_sk.public_key();

            let wire_msg = env.create_msg(&env.other_signed_sap.prefix(), other_pk)?;
            let sender = env.node.info().peer();

            // since it's not aware of the other prefix, it will redirect to self
            let dst_section_key = other_pk;
            let dst_name = env.other_signed_sap.prefix().name();

            let cmd = env
                .node
                .check_for_entropy(
                    &wire_msg,
                    &dst_section_key,
                    dst_name,
                    &sender,
                    None
                );

            let msg = assert_matches!(cmd, Ok(Some(Cmd::SendMsg { msg: OutgoingMsg::Node(msg), .. })) => {
                msg
            });

            assert_matches!(msg, NodeMsg::AntiEntropy { section_tree_update, kind: AntiEntropyKind::Redirect {..}, .. } => {
                assert_eq!(section_tree_update.signed_sap, env.node.network_knowledge().signed_sap());
            });

            // now let's insert the other SAP to make it aware of the other prefix
            let section_tree_update = SectionTreeUpdate::new(env.other_signed_sap.clone(), env.proof_chain);
            assert!(
                env.node
                    .update_network_knowledge(
                        section_tree_update,
                        None,
                    )?
            );

            // and it now shall give us an AE redirect msg
            // with the SAP we inserted for other prefix
            let cmd = env
                .node
                .check_for_entropy(
                    &wire_msg,
                    &dst_section_key,
                    dst_name,
                    &sender,
                    None
                );

            let msg = assert_matches!(cmd, Ok(Some(Cmd::SendMsg { msg: OutgoingMsg::Node(msg), .. })) => {
                msg
            });

            assert_matches!(msg, NodeMsg::AntiEntropy { section_tree_update, kind: AntiEntropyKind::Redirect {..}, .. } => {
                assert_eq!(section_tree_update.signed_sap, env.other_signed_sap);
            });
            Result::<()>::Ok(())
        }).await
    }

    #[tokio::test]
    async fn ae_outdated_dst_key_of_our_section() -> Result<()> {
        // Construct a local task set that can run `!Send` futures.
        let local = tokio::task::LocalSet::new();

        // Run the local task set.
        local.run_until(async move {


            let env = Env::new().await?;
            let our_prefix = env.node.network_knowledge().prefix();

            let msg = env.create_msg(
                &our_prefix,
                env.node.network_knowledge().section_key(),
            )?;
            let sender = env.node.info().peer();
            let dst_name = our_prefix.substituted_in(xor_name::rand::random());
            let dst_section_key = env.node.network_knowledge().genesis_key();

            let cmd = env
                .node
                .check_for_entropy(
                    &msg,
                    dst_section_key,
                    dst_name,
                    &sender,
                    None
                )?;

            let msg = assert_matches!(cmd, Some(Cmd::SendMsg { msg: OutgoingMsg::Node(msg), .. }) => {
                msg
            });

            assert_matches!(&msg, NodeMsg::AntiEntropy { section_tree_update, kind: AntiEntropyKind::Retry{..}, .. } => {
                assert_eq!(section_tree_update.signed_sap, env.node.network_knowledge().signed_sap());
                assert_eq!(section_tree_update.proof_chain, env.node.section_chain());
            });
            Ok(())
        }).await
    }

    #[tokio::test]
    async fn ae_wrong_dst_key_of_our_section_returns_retry() -> Result<()> {
        // Construct a local task set that can run `!Send` futures.
        let local = tokio::task::LocalSet::new();

        // Run the local task set.
        local.run_until(async move {

            let env = Env::new().await?;
            let our_prefix = env.node.network_knowledge().prefix();

            let msg = env.create_msg(
                &our_prefix,
                env.node.network_knowledge().section_key(),
            )?;
            let sender = env.node.info().peer();
            let dst_name = our_prefix.substituted_in(xor_name::rand::random());

            let bogus_env = Env::new().await?;
            let dst_section_key = bogus_env.node.network_knowledge().genesis_key();

            let cmd = env
                .node
                .check_for_entropy(
                    &msg,
                    dst_section_key,
                    dst_name,
                    &sender,
                    None
                )?;

            let msg = assert_matches!(cmd, Some(Cmd::SendMsg { msg: OutgoingMsg::Node(msg), .. }) => {
                msg
            });

            assert_matches!(&msg, NodeMsg::AntiEntropy { section_tree_update, kind: AntiEntropyKind::Retry {..}, .. } => {
                assert_eq!(section_tree_update.signed_sap, env.node.network_knowledge().signed_sap());
                assert_eq!(section_tree_update.proof_chain, env.node.section_chain());
            });
            Ok(())
        }).await
    }

    struct Env {
        node: MyNode,
        other_signed_sap: SectionSigned<SectionAuthorityProvider>,
        proof_chain: SectionsDAG,
    }

    impl Env {
        async fn new() -> Result<Self> {
            let prefix0 = Prefix::default().pushed(false);
            let prefix1 = Prefix::default().pushed(true);

            // generate a SAP for prefix0
            let (sap, mut nodes, secret_key_set) = random_sap(prefix0, elder_count(), 0, None);
            let info = nodes.remove(0);
            let sap_sk = secret_key_set.secret_key();
            let signed_sap = section_signed(&sap_sk, sap)?;

            let (proof_chain, genesis_sk_set) = create_proof_chain(signed_sap.section_key())
                .context("failed to create section chain")?;
            let genesis_pk = genesis_sk_set.public_keys().public_key();
            assert_eq!(genesis_pk, *proof_chain.genesis_key());

            let (max_capacity, root_storage_dir) = create_test_max_capacity_and_root_storage()?;
            let (mut node, _) = MyNode::first_node(
                create_comm().await?.socket_addr(),
                info.keypair.clone(),
                event_channel::new(1).0,
                UsedSpace::new(max_capacity),
                root_storage_dir,
                genesis_sk_set.clone(),
            )
            .await?;

            let section_key_share = SectionKeyShare {
                public_key_set: secret_key_set.public_keys(),
                index: 0,
                secret_key_share: secret_key_set.secret_key_share(0),
            };

            node.section_keys_provider = SectionKeysProvider::new(Some(section_key_share));

            // get our Node to now be in prefix(0)
            let section_tree_update = SectionTreeUpdate::new(signed_sap, proof_chain);
            let _ = node.update_network_knowledge(section_tree_update, None)?;

            // generate other SAP for prefix1
            let (other_sap, _, secret_key_set) = random_sap(prefix1, elder_count(), 0, None);
            let other_sap_sk = secret_key_set.secret_key();
            let other_sap = section_signed(&other_sap_sk, other_sap)?;
            // generate a proof chain for this other SAP
            let mut proof_chain = SectionsDAG::new(genesis_pk);
            let signature = bincode::serialize(&other_sap_sk.public_key())
                .map(|bytes| genesis_sk_set.secret_key().sign(&bytes))?;
            proof_chain.insert(&genesis_pk, other_sap_sk.public_key(), signature)?;

            Ok(Self {
                node,
                other_signed_sap: other_sap,
                proof_chain,
            })
        }

        fn create_msg(
            &self,
            src_section_prefix: &Prefix,
            src_section_pk: BlsPublicKey,
        ) -> Result<WireMsg> {
            let sender = MyNodeInfo::new(
                ed25519::gen_keypair(&src_section_prefix.range_inclusive(), MIN_ADULT_AGE),
                gen_addr(),
            );

            // just some message we can construct easily
            let payload_msg = NodeMsg::AntiEntropyProbe(src_section_pk);

            let payload = WireMsg::serialize_msg_payload(&payload_msg)?;

            let dst = Dst {
                name: xor_name::rand::random(),
                section_key: SecretKey::random().public_key(),
            };

            let msg_id = MsgId::new();

            Ok(WireMsg::new_msg(
                msg_id,
                payload,
                MsgKind::Node(sender.name()),
                dst,
            ))
        }
    }

    // Creates a proof chain with three blocks
    fn create_proof_chain(last_key: BlsPublicKey) -> Result<(SectionsDAG, bls::SecretKeySet)> {
        // create chain with random genesis key
        let genesis_sk_set = bls::SecretKeySet::random(0, &mut rand::thread_rng());
        let genesis_pk = genesis_sk_set.public_keys().public_key();
        let mut proof_chain = SectionsDAG::new(genesis_pk);

        // insert random second section key
        let second_sk_set = bls::SecretKeySet::random(0, &mut rand::thread_rng());
        let second_pk = second_sk_set.public_keys().public_key();
        let sig = genesis_sk_set
            .secret_key()
            .sign(&bincode::serialize(&second_pk)?);
        proof_chain.insert(&genesis_pk, second_pk, sig)?;

        // insert third key which is provided `last_key`
        let last_sig = second_sk_set
            .secret_key()
            .sign(&bincode::serialize(&last_key)?);
        proof_chain.insert(&second_pk, last_key, last_sig)?;

        Ok((proof_chain, genesis_sk_set))
    }
}
