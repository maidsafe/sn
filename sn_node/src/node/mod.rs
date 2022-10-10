// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! Implementation of the "Node" node for the SAFE Network.

/// Node Configuration
pub mod cfg;

mod api;
mod bootstrap;
mod connectivity;
mod data;
mod dkg;
pub(crate) mod error;
mod flow_ctrl;
mod handover;
mod logging;
mod membership;
mod messages;
mod messaging;
mod node_starter;
mod node_test_api;
mod proposal;
mod relocation;
mod statemap;

use self::{
    bootstrap::join_network,
    core::{
        MyNode, StateSnapshot, DATA_QUERY_LIMIT, GENESIS_DBC_AMOUNT, MAX_WAITING_PEERS_PER_QUERY,
        RESOURCE_PROOF_DATA_SIZE, RESOURCE_PROOF_DIFFICULTY,
    },
    data::MIN_LEVEL_WHEN_FULL,
    flow_ctrl::{
        cmds::Cmd,
        event::{CmdProcessEvent, Elders},
    },
    node_starter::CmdChannel,
    proposal::Proposal,
};
pub use self::{
    cfg::config_handler::Config,
    error::{Error, Result},
    flow_ctrl::{
        event::{Event, MembershipEvent, MessagingEvent, NodeElderChange},
        event_channel::EventReceiver,
    },
    node_starter::{new_test_api, start_node},
    node_test_api::NodeTestApi,
};

pub use crate::storage::DataStorage;

// pub(crate) use self::monitoring::RateLimits;
#[cfg(test)]
pub(crate) use relocation::{check as relocation_check, ChurnId};

pub use sn_interface::network_knowledge::{
    FIRST_SECTION_MAX_AGE, FIRST_SECTION_MIN_AGE, MIN_ADULT_AGE,
};
use sn_interface::{messaging::system::NodeMsg, types::Peer};

pub use qp2p::{Config as NetworkConfig, SendStream};
pub use xor_name::{Prefix, XorName, XOR_NAME_LEN}; // TODO remove pub on API update

mod core {
    use crate::{
        node::{
            bootstrap::JoiningAsRelocated,
            data::Capacity,
            dkg::DkgVoter,
            flow_ctrl::{cmds::Cmd, event_channel::EventSender},
            handover::Handover,
            membership::{elder_candidates, try_split_dkg, Membership},
            messaging::Peers,
            DataStorage, Elders, Error, Event, MembershipEvent, NodeElderChange, Prefix, Proposal,
            Result, XorName,
        },
        UsedSpace,
    };
    use sn_dysfunction::{DysfunctionDetection, IssueType};
    #[cfg(feature = "traceroute")]
    use sn_interface::messaging::Entity;

    use sn_interface::{
        messaging::{
            data::OperationId,
            signature_aggregator::SignatureAggregator,
            system::{DkgSessionId, NodeState, SectionSigned},
            AuthorityProof, SectionAuthorityProvider, SectionSig,
        },
        network_knowledge::{
            supermajority, MyNodeInfo, NetworkKnowledge,
            SectionAuthorityProvider as SectionAuthProvider, SectionKeyShare, SectionKeysProvider,
        },
        types::{keys::ed25519::Digest256, log_markers::LogMarker, Cache, DataAddress, Peer},
    };

    use backoff::ExponentialBackoff;
    use ed25519_dalek::Keypair;
    use itertools::Itertools;
    use resource_proof::ResourceProof;
    use sn_consensus::Generation;
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        net::SocketAddr,
        path::PathBuf,
        sync::Arc,
        time::Duration,
    };
    use uluru::LRUCache;

    /// Amount of tokens to be owned by the Genesis DBC.
    /// At the inception of the Network a total supply of 4,525,524,120 whole tokens will be created.
    /// Each whole token can be subdivided 10^9 times,
    /// thus creating a total of 4,525,524,120,000,000,000 available units.
    pub(crate) const GENESIS_DBC_AMOUNT: u64 = 4_525_524_120 * u64::pow(10, 9);

    pub(crate) const RESOURCE_PROOF_DATA_SIZE: usize = 128;
    pub(crate) const RESOURCE_PROOF_DIFFICULTY: u8 = 10;

    // This prevents pending query limit unbound growth
    // One insert per OpId/Adult.
    pub(crate) const DATA_QUERY_LIMIT: usize = 10_000;
    // per query we can have this many peers, so the total peers waiting can be QUERY_LIMIT * MAX_WAITING_PEERS_PER_QUERY
    // It's worth noting that nodes clean up all connections every two mins, so this max can only last that long.
    // (and yes, some clients may unfortunately be disconnected quickly)
    pub(crate) const MAX_WAITING_PEERS_PER_QUERY: usize = 100;

    const BACKOFF_CACHE_LIMIT: usize = 100;

    // File name where to cache this node's section tree (stored at this node's set root storage dir)
    const SECTION_TREE_FILE_NAME: &str = "section_tree";

    // How long to hold on to correlated `Peer`s for data queries. Since data queries are forwarded
    // from elders (with whom the client is connected) to adults (who hold the data), the elder handling
    // the query cannot reply immediately. For now, they stash a reference to the client `Peer` in
    // `Core::pending_data_queries`, which is a cache with duration-based expiry.
    // TODO: The value chosen here is shorter than the default client timeout (see
    // `use sn_client::SN_CLIENT_QUERY_TIMEOUT`), but the timeout is configurable. Ideally this would be
    // based on liveness properties (e.g. the timeout should be dynamic based on the responsiveness of
    // the section).
    const DATA_QUERY_TIMEOUT: Duration = Duration::from_secs(15);

    #[derive(Debug, Clone)]
    pub(crate) struct DkgSessionInfo {
        pub(crate) session_id: DkgSessionId,
        pub(crate) authority: AuthorityProof<SectionSig>,
    }

    // Store up to 100 in use backoffs
    pub(crate) type AeBackoffCache = LRUCache<(Peer, ExponentialBackoff), BACKOFF_CACHE_LIMIT>;

    pub(crate) struct MyNode {
        pub(crate) addr: SocketAddr, // does this change? if so... when? only at node start atm?
        pub(crate) event_sender: EventSender,
        root_storage_dir: PathBuf,
        pub(crate) data_storage: DataStorage, // Adult only before cache
        pub(crate) keypair: Arc<Keypair>,
        /// queue up all batch data to be replicated (as a result of churn events atm)
        // TODO: This can probably be reworked into the general per peer msg queue, but as
        // we need to pull data first before we form the WireMsg, we won't do that just now
        pub(crate) pending_data_to_replicate_to_peers: BTreeMap<DataAddress, BTreeSet<Peer>>,
        pub(crate) resource_proof: ResourceProof,
        // Network resources
        pub(crate) section_keys_provider: SectionKeysProvider,
        pub(crate) network_knowledge: NetworkKnowledge,
        // Signature aggregators
        pub(crate) message_aggregator: SignatureAggregator,
        pub(crate) proposal_aggregator: SignatureAggregator,
        // DKG/Split/Churn modules
        pub(crate) dkg_sessions_info: HashMap<Digest256, DkgSessionInfo>,
        pub(crate) dkg_voter: DkgVoter,
        pub(crate) pending_split_sections:
            BTreeMap<Generation, BTreeSet<SectionSigned<SectionAuthProvider>>>,
        pub(crate) relocate_state: Option<Box<JoiningAsRelocated>>,
        // ======================== Elder only ========================
        pub(crate) membership: Option<Membership>,
        // Section handover consensus state (Some for Elders, None for others)
        pub(crate) handover_voting: Option<Handover>,
        pub(crate) joins_allowed: bool,
        // Trackers
        pub(crate) capacity: Capacity,
        pub(crate) dysfunction_tracking: DysfunctionDetection,
        /// Cache the request combo,  (OperationId -> An adult xorname), to waiting Clients peers for that combo
        pub(crate) pending_data_queries: Cache<(OperationId, XorName), BTreeSet<Peer>>,
        // Caches
        pub(crate) ae_backoff_cache: AeBackoffCache,
    }

    impl MyNode {
        #[allow(clippy::too_many_arguments)]
        pub(crate) async fn new(
            addr: SocketAddr,
            keypair: Arc<Keypair>,
            network_knowledge: NetworkKnowledge,
            section_key_share: Option<SectionKeyShare>,
            event_sender: EventSender,
            used_space: UsedSpace,
            root_storage_dir: PathBuf,
        ) -> Result<Self> {
            let membership = if let Some(key) = section_key_share.clone() {
                let n_elders = network_knowledge.signed_sap().elder_count();

                // TODO: the bootstrap members should come from handover
                let bootstrap_members = BTreeSet::from_iter(
                    network_knowledge
                        .section_signed_members()
                        .into_iter()
                        .map(|section_auth| section_auth.value.to_msg()),
                );

                Some(Membership::from(
                    (key.index as u8, key.secret_key_share),
                    key.public_key_set,
                    n_elders,
                    bootstrap_members,
                ))
            } else {
                None
            };

            let section_keys_provider = SectionKeysProvider::new(section_key_share.clone());

            let data_storage = DataStorage::new(&root_storage_dir, used_space.clone())?;

            info!("Creating DysfunctionDetection checks");
            let node_dysfunction_detector = DysfunctionDetection::new(
                network_knowledge
                    .members()
                    .iter()
                    .map(|peer| peer.name())
                    .collect::<Vec<XorName>>(),
            );
            info!(
                "DysfunctionDetection check: {:?}",
                node_dysfunction_detector
            );

            // create handover
            let handover = if let Some(key) = section_key_share {
                let secret_key = (key.index as u8, key.secret_key_share);
                let elders = key.public_key_set;
                let n_elders = network_knowledge.elders().len();

                let handover_data = Handover::from(secret_key, elders, n_elders);
                Some(handover_data)
            } else {
                None
            };

            let node = Self {
                addr,
                keypair,
                network_knowledge,
                section_keys_provider,
                root_storage_dir,
                dkg_sessions_info: HashMap::default(),
                proposal_aggregator: SignatureAggregator::default(),
                pending_split_sections: Default::default(),
                message_aggregator: SignatureAggregator::default(),
                dkg_voter: DkgVoter::default(),
                relocate_state: None,
                event_sender,
                handover_voting: handover,
                joins_allowed: true,
                resource_proof: ResourceProof::new(
                    RESOURCE_PROOF_DATA_SIZE,
                    RESOURCE_PROOF_DIFFICULTY,
                ),
                data_storage,
                capacity: Capacity::default(),
                dysfunction_tracking: node_dysfunction_detector,
                pending_data_queries: Cache::with_expiry_duration(DATA_QUERY_TIMEOUT),
                pending_data_to_replicate_to_peers: BTreeMap::new(),
                ae_backoff_cache: AeBackoffCache::default(),
                membership,
            };

            node.statemap_log_metadata();

            // Write the section tree to this node's root storage directory
            node.write_section_tree().await;

            Ok(node)
        }

        pub(crate) fn info(&self) -> MyNodeInfo {
            let keypair = self.keypair.clone();
            let addr = self.addr;
            MyNodeInfo { keypair, addr }
        }

        pub(crate) fn name(&self) -> XorName {
            self.info().name()
        }

        ////////////////////////////////////////////////////////////////////////////
        // Miscellaneous
        ////////////////////////////////////////////////////////////////////////////

        /// Generates a random AE probe for _anywhere_ on the network.
        pub(crate) fn generate_probe_msg(&self) -> Result<Cmd> {
            // Generate a random address not belonging to our Prefix
            let mut dst = xor_name::rand::random();

            // We don't probe ourselves
            while self.network_knowledge.prefix().matches(&dst) {
                dst = xor_name::rand::random();
            }

            let matching_section = self.network_knowledge.section_auth_by_name(&dst)?;
            let recipients = matching_section.elders_set();

            let probe = self.network_knowledge.anti_entropy_probe();

            info!("ProbeMsg target {:?}: {probe:?}", matching_section.prefix());

            Ok(self.send_system_msg(probe, Peers::Multiple(recipients)))
        }

        /// Generates a SectionProbeMsg with our current knowledge,
        /// targetting our section elders.
        /// Even if we're up to date, we expect a response.
        pub(crate) fn generate_section_probe_msg(&self) -> Cmd {
            let our_section = self.network_knowledge.section_auth();
            let recipients = our_section.elders_set();

            info!(
                "ProbeMsg target our section {:?} recipients {:?}",
                our_section.prefix(),
                recipients,
            );

            let probe = self.network_knowledge.anti_entropy_probe();
            self.send_system_msg(probe, Peers::Multiple(recipients))
        }

        /// returns names that are relatively dysfunctional
        pub(crate) fn get_dysfunctional_node_names(&mut self) -> Result<BTreeSet<XorName>> {
            self.dysfunction_tracking
                .get_dysfunctional_nodes()
                .map_err(Error::from)
        }

        /// Log an issue in dysfunction
        pub(crate) fn log_node_issue(&mut self, name: XorName, issue: IssueType) {
            trace!("Logging issue {issue:?} in dysfunction for {name}");
            self.dysfunction_tracking.track_issue(name, issue)
        }

        /// Log a communication problem
        pub(crate) fn log_comm_issue(&mut self, name: XorName) {
            trace!("Logging comms issue in dysfunction for {name}");
            self.dysfunction_tracking
                .track_issue(name, IssueType::Communication)
        }

        /// Log a dkg issue (ie, an initialised but unfinished dkg round for a given participant)
        pub(crate) fn log_dkg_issue(&mut self, name: XorName) {
            trace!("Logging Dkg issue in dysfunction for {name}");
            self.dysfunction_tracking.track_issue(name, IssueType::Dkg)
        }

        /// Log a dkg session as responded to
        pub(crate) fn log_dkg_session(&mut self, name: &XorName) {
            trace!("Logging Dkg session as responded to in dysfunction for {name}");
            self.dysfunction_tracking.dkg_ack_fulfilled(name);
        }

        pub(crate) fn state_snapshot(&self) -> StateSnapshot {
            StateSnapshot {
                is_elder: self.is_elder(),
                section_key: self.network_knowledge.section_key(),
                prefix: self.network_knowledge.prefix(),
                elders: self.network_knowledge().section_auth().names(),
                members: self
                    .network_knowledge()
                    .members()
                    .into_iter()
                    .map(|p| p.name())
                    .collect(),
            }
        }

        /// Generates section infos for the best elder candidate among the members at the given generation
        /// Returns a set of candidate `DkgSessionId`'s.
        pub(crate) fn best_elder_candidates_at_gen(
            &mut self,
            membership_gen: u64,
        ) -> Vec<DkgSessionId> {
            let sap = self.network_knowledge.section_auth();
            let chain_len = self.network_knowledge.section_chain_len();

            // get members for membership gen
            let members: BTreeMap<XorName, NodeState> = if let Some(m) = self.membership.as_ref() {
                m.section_members(membership_gen)
                    .unwrap_or_default()
                    .iter()
                    .map(|(n, s)| (*n, s.clone()))
                    .collect()
            } else {
                error!("Attempted to find best elder candidates when we don't have a membership instance");
                return vec![];
            };

            // Try splitting
            trace!("{}", LogMarker::SplitAttempt);
            if let Some((zero_dkg_id, one_dkg_id)) =
                try_split_dkg(&members, &sap, chain_len, membership_gen)
            {
                debug!(
                    "Upon section split attempt, section size: zero {:?}, one {:?}",
                    zero_dkg_id.bootstrap_members.len(),
                    one_dkg_id.bootstrap_members.len()
                );
                info!("Splitting {:?} {:?}", zero_dkg_id, one_dkg_id);

                // Lets track ongoing DKG sessions.
                // However we won't receive DKG messages from the other after-split section.
                // So, shall only track the side that we are in as well.
                if zero_dkg_id.elders.contains_key(&self.info().name()) {
                    for candidate in zero_dkg_id.elders.keys() {
                        self.log_dkg_issue(*candidate);
                    }
                } else if one_dkg_id.elders.contains_key(&self.info().name()) {
                    for candidate in one_dkg_id.elders.keys() {
                        self.log_dkg_issue(*candidate);
                    }
                }

                return vec![zero_dkg_id, one_dkg_id];
            }

            // Candidates for elders out of all the nodes in the section, even out of the
            // relocating nodes if there would not be enough instead.
            let sap = self.network_knowledge.section_auth();
            let elder_candidates = elder_candidates(members.values().cloned(), &sap);
            let current_elders = BTreeSet::from_iter(sap.elders().copied());

            info!(
                "ELDER CANDIDATES (current gen:{}) {}: {:?}",
                membership_gen,
                elder_candidates.len(),
                elder_candidates
            );

            if elder_candidates
                .iter()
                .map(NodeState::peer)
                .eq(current_elders.iter())
            {
                vec![]
            } else if elder_candidates.len() < supermajority(current_elders.len()) {
                warn!("ignore attempt to reduce the number of elders too much");
                vec![]
            } else if elder_candidates.len() < current_elders.len() {
                // TODO: this special case doesn't seem valid to me, what if the section shrinks to below the elder size.
                // Could be due to the newly promoted elder doesn't have enough knowledge of
                // existing members.
                warn!("Ignore attempt to shrink the elders");
                trace!("current_names  {:?}", current_elders);
                trace!("expected_names {:?}", elder_candidates);
                trace!("section_peers {:?}", members);
                vec![]
            } else {
                let chain_len = self.network_knowledge.section_chain_len();
                let session_id = DkgSessionId {
                    prefix: sap.prefix(),
                    elders: BTreeMap::from_iter(
                        elder_candidates
                            .into_iter()
                            .map(|node| (node.name, node.addr)),
                    ),
                    section_chain_len: chain_len,
                    bootstrap_members: BTreeSet::from_iter(members.into_values()),
                    membership_gen,
                };
                // track init of DKG
                for candidate in session_id.elders.keys() {
                    self.log_dkg_issue(*candidate);
                }

                vec![session_id]
            }
        }

        /// Generates section infos for the current best elder candidate among the current members
        /// Returns a set of candidate `DkgSessionId`'s.
        pub(crate) fn best_elder_candidates(&mut self) -> Vec<DkgSessionId> {
            match self.membership.as_ref() {
                Some(m) => self.best_elder_candidates_at_gen(m.generation()),
                None => {
                    error!("Attempted to find best elder candidates when we don't have a membership instance");
                    vec![]
                }
            }
        }

        fn initialize_membership(&mut self, sap: SectionAuthorityProvider) -> Result<()> {
            let key = self
                .section_keys_provider
                .key_share(&self.network_knowledge.section_key())?;

            self.membership = Some(Membership::from(
                (key.index as u8, key.secret_key_share),
                key.public_key_set,
                sap.elders.len(),
                BTreeSet::from_iter(sap.members.into_values()),
            ));

            Ok(())
        }

        fn initialize_handover(&mut self) -> Result<()> {
            let key = self
                .section_keys_provider
                .key_share(&self.network_knowledge.section_key())?;
            let n_elders = self.network_knowledge.section_auth().elder_count();

            self.handover_voting = Some(Handover::from(
                (key.index as u8, key.secret_key_share),
                key.public_key_set,
                n_elders,
            ));

            Ok(())
        }

        fn initialize_elder_state(&mut self) -> Result<()> {
            let sap = self.network_knowledge.signed_sap().value.to_msg();
            self.initialize_membership(sap)?;
            self.initialize_handover()?;
            Ok(())
        }

        /// Updates various state if elders changed.
        pub(crate) async fn update_on_elder_change(
            &mut self,
            old: &StateSnapshot,
        ) -> Result<Vec<Cmd>> {
            let new = self.state_snapshot();

            if new.section_key == old.section_key {
                // there was no change
                return Ok(vec![]);
            }

            let mut cmds = vec![];

            // clean up DKG sessions 2 generations older than current
            // `session_id.section_chain_len + 2 < current_chain_len`
            // we voluntarily keep the previous DKG rounds
            // so lagging elder candidates can still get responses to their gossip.
            // At generation+2, they are not going to be elders anymore so we can safely discard it
            let current_chain_len = self.network_knowledge.section_chain_len();
            let mut old_hashes = vec![];
            for (hash, session_info) in self.dkg_sessions_info.iter() {
                if session_info.session_id.section_chain_len + 5 < current_chain_len {
                    old_hashes.push(*hash);
                    debug!(
                        "Removing old DKG s{} of chain len {} when we are at {}",
                        session_info.session_id.sh(),
                        session_info.session_id.section_chain_len,
                        current_chain_len
                    );
                }
            }
            for hash in old_hashes {
                let _ = self.dkg_sessions_info.remove(&hash);
                self.dkg_voter.remove(&hash);
            }

            // clean up pending split sections
            self.pending_split_sections = Default::default();

            if new.is_elder {
                let sap = self.network_knowledge.section_auth();
                info!(
                    "Section updated: prefix: ({:b}), key: {:?}, elders: {}",
                    new.prefix,
                    new.section_key,
                    sap.elders().format(", ")
                );

                // It can happen that we recieve the SAP demonstrating that we've become elders
                // before our local DKG can update the section_keys_provider with our Elder key share.
                //
                // Eventually our local DKG instance will complete and add our key_share to the
                // `section_keys_provider` cache. Once that happens, this function will be called
                // again and we can complete our Elder state transition.
                let we_have_our_key_share_for_new_section_key = self
                    .section_keys_provider
                    .key_share(&new.section_key)
                    .is_ok();

                if we_have_our_key_share_for_new_section_key {
                    // The section-key has changed, we are now able to function as an elder.
                    self.initialize_elder_state()?;

                    cmds.extend(self.trigger_dkg()?);

                    // Whenever there is an elders change, casting a round of joins_allowed
                    // proposals to sync this particular state.
                    cmds.extend(self.propose(Proposal::JoinsAllowed(self.joins_allowed))?);
                }

                self.log_network_stats();
                self.log_section_stats();
            } else {
                // if not elder
                self.handover_voting = None;
            }

            if new.is_elder || old.is_elder {
                if let Some(cmd) = self.send_ae_update_to_our_section()? {
                    cmds.push(cmd);
                }
            }

            let current: BTreeSet<_> = self.network_knowledge.section_auth().names();
            let added = current.difference(&old.elders).copied().collect();
            let removed = old.elders.difference(&current).copied().collect();
            let remaining = old.elders.intersection(&current).copied().collect();

            let elders = Elders {
                prefix: new.prefix,
                key: new.section_key,
                remaining,
                added,
                removed,
            };

            let self_status_change = if !old.is_elder && new.is_elder {
                info!("{}: {:?}", LogMarker::PromotedToElder, new.prefix);
                NodeElderChange::Promoted
            } else if old.is_elder && !new.is_elder {
                info!("{}", LogMarker::DemotedFromElder);
                self.section_keys_provider.wipe();
                NodeElderChange::Demoted
            } else {
                NodeElderChange::None
            };

            let mut events = vec![];

            let new_elders = !elders.added.is_empty();
            let section_split = new.prefix != old.prefix;
            let elders_changed = !elders.added.is_empty() || !elders.removed.is_empty();

            if section_split && new.is_elder {
                info!("{}: {:?}", LogMarker::SplitSuccess, new.prefix);

                if old.is_elder {
                    info!("{}: {:?}", LogMarker::StillElderAfterSplit, new.prefix);
                }

                cmds.extend(self.send_updates_to_sibling_section(old)?);
                self.liveness_retain_only(
                    self.network_knowledge
                        .adults()
                        .iter()
                        .map(|peer| peer.name())
                        .collect(),
                )?;

                // During the split, sibling's SAP could be unknown to us yet.
                // Hence, fire the SectionSplit event whenever detect a prefix change.
                events.push(Event::Membership(MembershipEvent::SectionSplit {
                    elders: elders.clone(),
                    self_status_change,
                }))
            };

            if !section_split && elders_changed {
                events.push(Event::Membership(MembershipEvent::EldersChanged {
                    elders,
                    self_status_change,
                }))
            }

            // update new elders if we were an elder (regardless if still or not)
            if new_elders && old.is_elder {
                cmds.push(
                    self.send_metadata_updates(
                        self.network_knowledge
                            .section_auth()
                            .elders()
                            .filter(|peer| !old.elders.contains(&peer.name()))
                            .cloned()
                            .collect(),
                        &self.network_knowledge.prefix(),
                    ),
                );
            };

            for event in events {
                self.send_event(event).await
            }

            Ok(cmds)
        }

        #[allow(unused)]
        pub(crate) fn section_key_by_name(&self, name: &XorName) -> Result<bls::PublicKey> {
            if self.network_knowledge.prefix().matches(name) {
                Ok(self.network_knowledge.section_key())
            } else if let Ok(sap) = self.network_knowledge.section_auth_by_name(name) {
                Ok(sap.section_key())
            } else if self.network_knowledge.prefix().sibling().matches(name) {
                // For sibling with unknown key, use the previous key in our chain under the assumption
                // that it's the last key before the split and therefore the last key of theirs we know.
                // In case this assumption is not correct (because we already progressed more than one
                // key since the split) then this key would be unknown to them and they would send
                // us back their whole section chain. However, this situation should be rare.

                // section_chain contains a single leaf key
                let leaf_key = self.network_knowledge.section_chain().last_key()?;
                match self.section_chain().get_parent_key(&leaf_key) {
                    Ok(prev_pk) => Ok(prev_pk.unwrap_or(*self.section_chain().genesis_key())),
                    Err(_) => {
                        error!("SectionsDAG fields went out of sync");
                        Ok(leaf_key)
                    }
                }
            } else {
                Ok(*self.network_knowledge.genesis_key())
            }
        }

        pub(crate) fn log_network_stats(&self) {
            info!(
                "{}",
                self.network_knowledge
                    .section_tree()
                    .network_stats(&self.network_knowledge.section_auth())
            );
        }

        pub(crate) fn log_section_stats(&self) {
            if let Some(m) = self.membership.as_ref() {
                let adults = self.network_knowledge.adults().len();

                let elders = self.network_knowledge.section_auth().elder_count();

                let membership_adults = m.current_section_members().len() - elders;
                let prefix = self.network_knowledge.prefix();

                debug!("{prefix:?}: {elders} Elders, {adults}~{membership_adults} Adults.");
            } else {
                debug!("log_section_stats: No membership instance");
            };
        }

        pub(crate) async fn write_section_tree(&self) {
            let section_tree = self.network_knowledge.section_tree().clone();
            let path = self.root_storage_dir.clone().join(SECTION_TREE_FILE_NAME);

            let _ = tokio::spawn(async move {
                if let Err(err) = section_tree.write_to_disk(&path).await {
                    error!(
                        "Error writing SectionTree to `{}` dir: {:?}",
                        path.display(),
                        err
                    );
                }
            });
        }

        #[cfg(feature = "traceroute")]
        pub(crate) fn identity(&self) -> Entity {
            if self.is_elder() {
                Entity::Elder(self.info().public_key())
            } else {
                Entity::Adult(self.info().public_key())
            }
        }
    }

    #[derive(Clone)]
    pub(crate) struct StateSnapshot {
        pub(crate) is_elder: bool,
        pub(crate) section_key: bls::PublicKey,
        pub(crate) prefix: Prefix,
        pub(crate) elders: BTreeSet<XorName>,
        pub(crate) members: BTreeSet<XorName>,
    }
}
