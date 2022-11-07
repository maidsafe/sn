// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::node::{flow_ctrl::cmds::Cmd, MyNode, Proposal, Result};
use sn_interface::{
    messaging::system::{SectionSig, SectionSigned},
    network_knowledge::{
        NodeState, SapCandidate, SectionAuthUtils, SectionAuthorityProvider, SectionTreeUpdate,
    },
    types::log_markers::LogMarker,
};
use std::collections::BTreeSet;

// Agreement
impl MyNode {
    #[instrument(skip(self), level = "trace")]
    pub(crate) fn handle_general_agreements(
        &mut self,
        proposal: Proposal,
        sig: SectionSig,
    ) -> Result<Vec<Cmd>> {
        debug!("{:?} {:?}", LogMarker::ProposalAgreed, proposal);
        let mut cmds = Vec::new();
        match proposal {
            Proposal::VoteNodeOffline(node_state) => {
                cmds.extend(self.handle_offline_agreement(node_state, sig))
            }
            Proposal::SectionInfo(sap) => {
                cmds.extend(self.handle_section_info_agreement(sap, sig)?)
            }
            Proposal::NewElders(_) => {
                error!("Elders agreement should be handled in a separate blocking fashion");
            }
            Proposal::JoinsAllowed(joins_allowed) => {
                self.joins_allowed = joins_allowed;
            }
        }
        Ok(cmds)
    }

    #[instrument(skip(self))]
    fn handle_offline_agreement(&mut self, node_state: NodeState, sig: SectionSig) -> Option<Cmd> {
        info!(
            "Agreement - proposing membership change with node offline: {}",
            node_state.peer()
        );
        self.propose_membership_change(node_state)
    }

    #[instrument(skip(self), level = "trace")]
    fn handle_section_info_agreement(
        &mut self,
        sap: SectionAuthorityProvider,
        sig: SectionSig,
    ) -> Result<Vec<Cmd>> {
        // check if section matches our prefix
        let equal_prefix = sap.prefix() == self.network_knowledge.prefix();
        let is_extension_prefix = sap
            .prefix()
            .is_extension_of(&self.network_knowledge.prefix());
        if !equal_prefix && !is_extension_prefix {
            // Other section. We shouln't be receiving or updating a SAP for
            // a remote section here, that is done with a AE msg response.
            debug!(
                "Ignoring Proposal::SectionInfo since prefix doesn't match ours: {:?}",
                sap
            );
            return Ok(vec![]);
        }
        debug!("Handling section info with prefix: {:?}", sap.prefix());

        // check if at the given memberhip gen, the elders candidates are matching
        let membership_gen = sap.membership_gen();
        let signed_sap = SectionSigned::new(sap, sig);
        let dkg_sessions_info = self.best_elder_candidates_at_gen(membership_gen);

        let elder_candidates = BTreeSet::from_iter(signed_sap.names());
        if dkg_sessions_info
            .iter()
            .all(|session| !session.elder_names().eq(elder_candidates.iter().copied()))
        {
            error!("Elder candidates don't match best elder candidates at given gen in received section agreement, ignoring it.");
            return Ok(vec![]);
        };

        // handle regular elder handover (1 to 1)
        // trigger handover consensus among elders
        if equal_prefix {
            debug!("Propose elder handover to: {:?}", signed_sap.prefix());
            return self.propose_handover_consensus(SapCandidate::ElderHandover(signed_sap));
        }

        // add to pending split SAP candidates
        // those are stored in a mapping from Generation to BTreeSet so the order in the set is deterministic
        let section_candidates_for_gen = self
            .pending_split_sections
            .entry(membership_gen)
            .and_modify(|curr| {
                let _ = curr.insert(signed_sap.clone());
            })
            .or_insert_with(|| BTreeSet::from([signed_sap]));

        // if we have reached 2 split SAP candidates for this generation
        // handle section split (1 to 2)
        if let [sap1, sap2] = section_candidates_for_gen
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .as_slice()
        {
            debug!(
                "Propose section split handover to: {:?} {:?}",
                sap1.prefix(),
                sap2.prefix()
            );
            self.propose_handover_consensus(SapCandidate::SectionSplit(sap1.clone(), sap2.clone()))
        } else {
            debug!("Waiting for more split handover candidates");
            Ok(vec![])
        }
    }

    #[instrument(skip(self), level = "trace")]
    pub(crate) fn handle_new_elders_agreement(
        &mut self,
        signed_sap: SectionSigned<SectionAuthorityProvider>,
        section_sig: SectionSig,
    ) -> Result<Vec<Cmd>> {
        trace!("{}", LogMarker::HandlingNewEldersAgreement);
        let snapshot = self.state_snapshot();
        let mut section_chain = self.section_chain();
        let last_key = section_chain.last_key()?;

        let prefix = signed_sap.prefix();
        trace!("{}: for {:?}", LogMarker::NewSignedSap, prefix);

        info!("New SAP agreed for:{}", *signed_sap);

        // Let's update our network knowledge, including our
        // section SAP and chain if the new SAP's prefix matches our name
        // We need to generate the proof chain to connect our current chain to new SAP.
        match section_chain.insert(&last_key, signed_sap.section_key(), section_sig.signature) {
            Ok(()) => {
                let section_tree_update = SectionTreeUpdate::new(signed_sap, section_chain);
                match self.update_network_knowledge(section_tree_update, None) {
                    Ok(true) => {
                        info!("Updated our network knowledge for {:?}", prefix);
                        info!("Writing updated knowledge to disk");
                        self.write_section_tree()
                    }
                    Err(err) => {
                        error!("Error updating our network knowledge for {prefix:?}: {err:?}")
                    }

                    _ => {}
                }
            }
            Err(err) => error!("Failed to generate proof chain for a newly received SAP: {err:?}"),
        }

        info!(
            "Prefixes we know about: {:?}",
            self.network_knowledge.section_tree()
        );

        self.update_on_elder_change(&snapshot)
    }
}
