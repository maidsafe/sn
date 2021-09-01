// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::NetworkPrefixMap;
use crate::messaging::{
    system::{Peer, Section},
    DstLocation,
};
use crate::routing::{
    error::{Error, Result},
    peer::PeerUtils,
    section::{SectionPeersUtils, SectionUtils},
    supermajority, SectionAuthorityProviderUtils, ELDER_SIZE,
};
use itertools::Itertools;
use std::{cmp, iter};
use xor_name::XorName;

/// Returns a set of nodes and their section PublicKey to which a message for the given
/// `DstLocation` could be sent onwards, sorted by priority, along with the number of targets the
/// message should be sent to. If the total number of targets returned is larger than this number,
/// the spare targets can be used if the message can't be delivered to some of the initial ones.
///
/// * If the destination is a `DstLocation::Section` OR `DstLocation::EndUser`:
///     - if our section is the closest on the network (i.e. our section's prefix is a prefix of
///       the dst), returns all other members of our section; otherwise
///     - returns the `N/3` closest members to the target
///
/// * If the destination is an individual node:
///     - if our name *is* the dst, returns an empty set; otherwise
///     - if the destination name is an entry in the routing table, returns it; otherwise
///     - returns the `N/3` closest members of the RT to the target
pub(crate) fn delivery_targets(
    dst: &DstLocation,
    our_name: &XorName,
    section: &Section,
    network: &NetworkPrefixMap,
) -> Result<(Vec<Peer>, usize)> {
    // Adult now having the knowledge of other adults within the own section.
    // Functions of `section_candidates` and `candidates` only take section elder into account.

    match dst {
        DstLocation::Section { name, .. } => section_candidates(name, our_name, section, network),
        DstLocation::EndUser(user) => section_candidates(&user.0, our_name, section, network),
        DstLocation::Node { name, .. } => {
            if name == our_name {
                return Ok((Vec::new(), 0));
            }
            if let Some(node) = get_peer(name, section, network) {
                return Ok((vec![node], 1));
            }

            if !section.is_elder(our_name) {
                // We are not Elder - return all the elders of our section,
                // so the message can be properly relayed through them.
                let targets: Vec<_> = section.authority_provider().peers().collect();
                let dg_size = targets.len();
                Ok((targets, dg_size))
            } else {
                candidates(name, our_name, section, network)
            }
        }
    }
}

fn section_candidates(
    target_name: &XorName,
    our_name: &XorName,
    section: &Section,
    network: &NetworkPrefixMap,
) -> Result<(Vec<Peer>, usize)> {
    // Find closest section to `target_name` out of the ones we know (including our own)
    let network_sections = network.all();
    let info = iter::once(section.authority_provider())
        .chain(network_sections.iter())
        .min_by(|lhs, rhs| lhs.prefix.cmp_distance(&rhs.prefix, target_name))
        .unwrap_or_else(|| section.authority_provider());

    if info.prefix == *section.prefix() {
        // Exclude our name since we don't need to send to ourself
        let chosen_section: Vec<_> = info
            .peers()
            .filter(|node| node.name() != our_name)
            .collect();
        let dg_size = chosen_section.len();
        return Ok((chosen_section, dg_size));
    }

    candidates(target_name, our_name, section, network)
}

// Obtain the delivery group candidates for this target
fn candidates(
    target_name: &XorName,
    our_name: &XorName,
    section: &Section,
    network: &NetworkPrefixMap,
) -> Result<(Vec<Peer>, usize)> {
    // All sections we know (including our own), sorted by distance to `target_name`.
    let network_sections = network.all();
    let sections = iter::once(section.authority_provider())
        .chain(network_sections.iter())
        .sorted_by(|lhs, rhs| lhs.prefix.cmp_distance(&rhs.prefix, target_name))
        .map(|info| (&info.prefix, info.elder_count(), info.peers()));

    // gives at least 1 honest target among recipients.
    let min_dg_size = 1 + ELDER_SIZE - supermajority(ELDER_SIZE);
    let mut dg_size = min_dg_size;
    let mut candidates = Vec::new();
    for (idx, (prefix, len, connected)) in sections.enumerate() {
        candidates.extend(connected);
        if prefix.matches(target_name) {
            // If we are last hop before final dst, send to all candidates.
            dg_size = len;
        } else {
            // If we don't have enough contacts send to as many as possible
            // up to dg_size of Elders
            dg_size = cmp::min(len, dg_size);
        }
        if len < min_dg_size {
            warn!(
                "Delivery group only {:?} when it should be {:?}",
                len, min_dg_size
            )
        }

        if prefix == section.prefix() {
            // Send to all connected targets so they can forward the message
            candidates.retain(|node| node.name() != our_name);
            dg_size = candidates.len();
            break;
        }
        if idx == 0 && candidates.len() >= dg_size {
            // can deliver to enough of the closest section
            break;
        }
    }
    candidates.sort_by(|lhs, rhs| target_name.cmp_distance(lhs.name(), rhs.name()));

    if dg_size > 0 && candidates.len() >= dg_size {
        Ok((candidates, dg_size))
    } else {
        Err(Error::CannotRoute)
    }
}

// Returns a `Peer` for a known node.
fn get_peer(name: &XorName, section: &Section, network: &NetworkPrefixMap) -> Option<Peer> {
    match section.members().get(name) {
        Some(info) => Some(info.peer),
        None => network
            .section_by_name(name)
            .ok()?
            .get_addr(name)
            .map(|addr| {
                let mut peer = Peer::new(*name, addr);
                peer.set_reachable(true);
                peer
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{system::NodeState, SectionAuthorityProvider};
    use crate::routing::{
        dkg::test_utils::section_signed,
        ed25519,
        section::{
            test_utils::{gen_addr, gen_section_authority_provider},
            NodeStateUtils,
        },
        SectionAuthorityProviderUtils, MIN_ADULT_AGE,
    };
    use eyre::{ContextCompat, Result};
    use rand::seq::IteratorRandom;
    use secured_linked_list::SecuredLinkedList;
    use xor_name::Prefix;

    #[test]
    fn delivery_targets_elder_to_our_elder() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let dst_name = *section
            .authority_provider()
            .names()
            .iter()
            .filter(|&&name| name != our_name)
            .choose(&mut rand::thread_rng())
            .context("too few elders")?;

        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send only to the dst node.
        assert_eq!(dg_size, 1);
        assert_eq!(recipients[0].name(), &dst_name);

        Ok(())
    }

    #[test]
    fn delivery_targets_elder_to_our_adult() -> Result<()> {
        let (our_name, mut section, network, sk) = setup_elder()?;

        let name = ed25519::gen_name_with_age(MIN_ADULT_AGE);
        let dst_name = section.prefix().substituted_in(name);
        let peer = Peer::new(dst_name, gen_addr());
        let node_state = NodeState::joined(peer, None);
        let node_state = section_signed(&sk, node_state)?;
        assert!(section.update_member(node_state));

        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send only to the dst node.
        assert_eq!(dg_size, 1);
        assert_eq!(recipients[0].name(), &dst_name);

        Ok(())
    }

    #[test]
    fn delivery_targets_elder_to_our_section() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let dst_name = section.prefix().substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Section {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all our elders except us.
        let expected_recipients = section
            .authority_provider()
            .peers()
            .filter(|peer| peer.name() != &our_name);
        assert_eq!(dg_size, expected_recipients.count());

        let expected_recipients = section
            .authority_provider()
            .peers()
            .filter(|peer| peer.name() != &our_name);
        itertools::assert_equal(recipients, expected_recipients);

        Ok(())
    }

    #[test]
    fn delivery_targets_elder_to_known_remote_peer() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let section_auth1 = network
            .get(&Prefix::default().pushed(true))
            .context("unknown section")?;

        let dst_name = choose_elder_name(&section_auth1)?;
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send only to the dst node.
        assert_eq!(dg_size, 1);
        assert_eq!(recipients[0].name(), &dst_name);

        Ok(())
    }

    #[test]
    fn delivery_targets_elder_to_final_hop_unknown_remote_peer() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let section_auth1 = network
            .get(&Prefix::default().pushed(true))
            .context("unknown section")?;

        let dst_name = section_auth1.prefix.substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders in the dst section
        let expected_recipients = section_auth1
            .peers()
            .sorted_by(|lhs, rhs| dst_name.cmp_distance(lhs.name(), rhs.name()));
        assert_eq!(dg_size, section_auth1.elder_count());
        itertools::assert_equal(recipients, expected_recipients);

        Ok(())
    }

    #[test]
    #[ignore = "Need to setup network so that we do not locate final dst, as to trigger correct outcome."]
    fn delivery_targets_elder_to_intermediary_hop_unknown_remote_peer() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let elders_info1 = network
            .get(&Prefix::default().pushed(true))
            .context("unknown section")?;

        let dst_name = elders_info1
            .prefix
            .pushed(false)
            .substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders in the dst section
        let expected_recipients = elders_info1
            .peers()
            .sorted_by(|lhs, rhs| dst_name.cmp_distance(lhs.name(), rhs.name()));
        let min_dg_size =
            1 + elders_info1.elder_count() - supermajority(elders_info1.elder_count());
        assert_eq!(dg_size, min_dg_size);
        itertools::assert_equal(recipients, expected_recipients);

        Ok(())
    }

    #[test]
    fn delivery_targets_elder_final_hop_to_remote_section() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let section_auth1 = network
            .get(&Prefix::default().pushed(true))
            .context("unknown section")?;

        let dst_name = section_auth1.prefix.substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Section {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders in the final dst section
        let expected_recipients = section_auth1
            .peers()
            .sorted_by(|lhs, rhs| dst_name.cmp_distance(lhs.name(), rhs.name()));
        assert_eq!(dg_size, section_auth1.elder_count());
        itertools::assert_equal(recipients, expected_recipients);

        Ok(())
    }

    #[test]
    #[ignore = "Need to setup network so that we do not locate final dst, as to trigger correct outcome."]
    fn delivery_targets_elder_intermediary_hop_to_remote_section() -> Result<()> {
        let (our_name, section, network, _) = setup_elder()?;

        let elders_info1 = network
            .get(&Prefix::default().pushed(true))
            .context("unknown section")?;

        let dst_name = elders_info1
            .prefix
            .pushed(false)
            .substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Section {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to a subset of elders in the intermediary dst section
        let min_dg_size =
            1 + elders_info1.elder_count() - supermajority(elders_info1.elder_count());
        let expected_recipients = elders_info1
            .peers()
            .sorted_by(|lhs, rhs| dst_name.cmp_distance(lhs.name(), rhs.name()))
            .take(min_dg_size);

        assert_eq!(dg_size, min_dg_size);
        itertools::assert_equal(recipients, expected_recipients);

        Ok(())
    }

    #[test]
    fn delivery_targets_adult_to_our_elder() -> Result<()> {
        let (our_name, section, network) = setup_adult()?;

        let dst_name = choose_elder_name(section.authority_provider())?;
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders
        assert_eq!(dg_size, section.authority_provider().elder_count());
        itertools::assert_equal(recipients, section.authority_provider().peers());

        Ok(())
    }

    #[test]
    fn delivery_targets_adult_to_our_adult() -> Result<()> {
        let (our_name, section, network) = setup_adult()?;

        let dst_name = section.prefix().substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders
        assert_eq!(dg_size, section.authority_provider().elder_count());
        itertools::assert_equal(recipients, section.authority_provider().peers());

        Ok(())
    }

    #[test]
    fn delivery_targets_adult_to_our_section() -> Result<()> {
        let (our_name, section, network) = setup_adult()?;

        let dst_name = section.prefix().substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Section {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders
        assert_eq!(dg_size, section.authority_provider().elder_count());
        itertools::assert_equal(recipients, section.authority_provider().peers());

        Ok(())
    }

    #[test]
    fn delivery_targets_adult_to_remote_peer() -> Result<()> {
        let (our_name, section, network) = setup_adult()?;

        let dst_name = Prefix::default()
            .pushed(true)
            .substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Node {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders
        assert_eq!(dg_size, section.authority_provider().elder_count());
        itertools::assert_equal(recipients, section.authority_provider().peers());

        Ok(())
    }

    #[test]
    fn delivery_targets_adult_to_remote_section() -> Result<()> {
        let (our_name, section, network) = setup_adult()?;

        let dst_name = Prefix::default()
            .pushed(true)
            .substituted_in(rand::random());
        let section_pk = section.authority_provider().section_key();
        let dst = DstLocation::Section {
            name: dst_name,
            section_pk,
        };
        let (recipients, dg_size) = delivery_targets(&dst, &our_name, &section, &network)?;

        // Send to all elders
        assert_eq!(dg_size, section.authority_provider().elder_count());
        itertools::assert_equal(recipients, section.authority_provider().peers());

        Ok(())
    }

    fn setup_elder() -> Result<(XorName, Section, NetworkPrefixMap, bls::SecretKey)> {
        let prefix0 = Prefix::default().pushed(false);
        let prefix1 = Prefix::default().pushed(true);

        let (section_auth0, _, secret_key_set) =
            gen_section_authority_provider(prefix0, ELDER_SIZE);
        let genesis_sk = secret_key_set.secret_key();
        let genesis_pk = genesis_sk.public_key();

        let elders0: Vec<_> = section_auth0.peers().collect();
        let section_auth0 = section_signed(genesis_sk, section_auth0)?;

        let chain = SecuredLinkedList::new(genesis_pk);

        let mut section = Section::new(genesis_pk, chain, section_auth0)?;

        for peer in elders0 {
            let node_state = NodeState::joined(peer, None);
            let node_state = section_signed(genesis_sk, node_state)?;
            assert!(section.update_member(node_state));
        }

        let network = NetworkPrefixMap::new();

        let (section_auth1, _, secret_key_set) =
            gen_section_authority_provider(prefix1, ELDER_SIZE);
        let sk1 = secret_key_set.secret_key();
        let pk1 = sk1.public_key();

        let section_auth1 = section_signed(sk1, section_auth1)?;

        // create a section chain branched out from same genesis pk
        let mut proof_chain = SecuredLinkedList::new(genesis_pk);
        // second key is the PK derived from SAP's SK
        let sig1 = bincode::serialize(&pk1).map(|bytes| genesis_sk.sign(&bytes))?;
        proof_chain.insert(&genesis_pk, pk1, sig1)?;

        // 3rd key is the section key in SAP
        let pk2 = section_auth1.value.public_key_set.public_key();
        let sig2 = bincode::serialize(&pk2).map(|bytes| sk1.sign(&bytes))?;
        proof_chain.insert(&pk1, pk2, sig2)?;

        assert!(network
            .update_remote_section_sap(section_auth1, &proof_chain, section.chain())
            .is_ok(),);

        let our_name = choose_elder_name(section.authority_provider())?;

        Ok((our_name, section, network, genesis_sk.clone()))
    }

    fn setup_adult() -> Result<(XorName, Section, NetworkPrefixMap)> {
        let prefix0 = Prefix::default().pushed(false);

        let (section_auth, _, secret_key_set) = gen_section_authority_provider(prefix0, ELDER_SIZE);
        let genesis_sk = secret_key_set.secret_key();
        let genesis_pk = genesis_sk.public_key();
        let section_auth = section_signed(genesis_sk, section_auth)?;
        let chain = SecuredLinkedList::new(genesis_pk);
        let section = Section::new(genesis_pk, chain, section_auth)?;

        let network = NetworkPrefixMap::new();
        let our_name = section.prefix().substituted_in(rand::random());

        Ok((our_name, section, network))
    }

    fn choose_elder_name(section_auth: &SectionAuthorityProvider) -> Result<XorName> {
        section_auth
            .elders()
            .keys()
            .choose(&mut rand::thread_rng())
            .copied()
            .context("no elders")
    }
}
