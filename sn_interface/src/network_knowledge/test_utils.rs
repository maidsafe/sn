use super::{
    NetworkKnowledge, NodeState, SectionKeysProvider, SectionTree, SectionTreeUpdate, SectionsDAG,
};
use crate::{
    messaging::system::SectionSigned,
    network_knowledge::{section_keys::build_spent_proof_share, Error, MyNodeInfo, MIN_ADULT_AGE},
    test_utils::{TestKeys, TestSAP},
    SectionAuthorityProvider,
};
use eyre::{eyre, Result};
use itertools::Itertools;
use sn_consensus::{Ballot, Consensus, Decision, Proposition, Vote, VoteResponse};
use sn_dbc::{
    get_public_commitments_from_transaction, Commitment, Dbc, Owner, OwnerOnce, RingCtTransaction,
    Token, TransactionBuilder,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::SocketAddr,
};
use xor_name::Prefix;

// Parse `Prefix` from string
pub fn prefix(s: &str) -> Prefix {
    s.parse().expect("Failed to parse prefix")
}

// Generate unique SocketAddr for testing purposes
pub fn gen_addr() -> SocketAddr {
    thread_local! {
        static NEXT_PORT: Cell<u16> = Cell::new(1000);
    }

    let port = NEXT_PORT.with(|cell| cell.replace(cell.get().wrapping_add(1)));

    ([192, 0, 2, 0], port).into()
}

// Create `count` Nodes sorted by their names.
// The `age_diff` flag is used to trigger nodes being generated with different age pattern.
// The test of `handle_agreement_on_online_of_elder_candidate` requires most nodes to be with
// age of MIN_AGE + 2 and one node with age of MIN_ADULT_AGE.
pub fn gen_sorted_nodes(prefix: &Prefix, count: usize, age_diff: bool) -> Vec<MyNodeInfo> {
    (0..count)
        .map(|index| {
            let age = if age_diff && index < count - 1 {
                MIN_ADULT_AGE + 1
            } else {
                MIN_ADULT_AGE
            };
            MyNodeInfo::new(
                crate::types::keys::ed25519::gen_keypair(&prefix.range_inclusive(), age),
                gen_addr(),
            )
        })
        .sorted_by_key(|node| node.name())
        .collect()
}

/// Generate a random `NetworkKnowledge` for testing.
///
/// Uses `random_sap_with_key` to generate SAP; section_peer list is updated with the `NodeState` as well
pub fn gen_network_knowledge_with_key(
    prefix: Prefix,
    elder_count: usize,
    adult_count: usize,
    sk_set: &bls::SecretKeySet,
) -> (NetworkKnowledge, Vec<MyNodeInfo>) {
    let pk_set = sk_set.public_keys();
    let (sap, node_infos) = TestSAP::random_sap_with_key(prefix, elder_count, adult_count, sk_set);
    let signed_sap = TestKeys::get_section_signed(&sk_set.secret_key(), sap);
    let section_tree_update =
        SectionTreeUpdate::new(signed_sap, SectionsDAG::new(pk_set.public_key()));
    let mut network_knowledge =
        NetworkKnowledge::new(SectionTree::new(pk_set.public_key()), section_tree_update)
            .expect("Failed to create NetworkKnowledge");

    // update the sap members
    for peer in network_knowledge.signed_sap.elders() {
        let node_state = NodeState::joined(*peer, None);
        let signed_state = TestKeys::get_section_signed(&sk_set.secret_key(), node_state);
        let _changed = network_knowledge.section_peers.update(signed_state);
    }
    (network_knowledge, node_infos)
}

/// Generate a `SectionTreeUpdate` where the SAP's section key is appended to the proof chain
pub fn gen_section_tree_update(
    sap: &SectionSigned<SectionAuthorityProvider>,
    proof_chain: &SectionsDAG,
    parent_sk: &bls::SecretKey,
) -> SectionTreeUpdate {
    let signed_key = TestKeys::get_section_signed(parent_sk, sap.section_key());
    let mut proof_chain = proof_chain.clone();
    proof_chain
        .insert(
            &parent_sk.public_key(),
            signed_key.value,
            signed_key.sig.signature,
        )
        .expect("Failed to insert into proof chain");
    SectionTreeUpdate::new(sap.clone(), proof_chain)
}

pub fn section_decision<P: Proposition>(
    secret_key_set: &bls::SecretKeySet,
    proposal: P,
) -> Decision<P> {
    let n = secret_key_set.threshold() + 1;
    let mut nodes = Vec::from_iter((1..=n).into_iter().map(|idx| {
        let secret = (idx as u8, secret_key_set.secret_key_share(idx));
        Consensus::from(secret, secret_key_set.public_keys(), n)
    }));

    let first_vote = nodes[0]
        .sign_vote(Vote {
            gen: 0,
            ballot: Ballot::Propose(proposal),
            faults: Default::default(),
        })
        .expect("Failed to sign first vote");

    let mut votes = vec![nodes[0].cast_vote(first_vote).expect("Failed to cast vote")];

    while let Some(vote) = votes.pop() {
        for node in &mut nodes {
            match node
                .handle_signed_vote(vote.clone())
                .expect("Failed to handle vote")
            {
                VoteResponse::WaitingForMoreVotes => (),
                VoteResponse::Broadcast(vote) => votes.push(vote),
            }
        }
    }

    // All nodes have agreed to the same proposal
    assert_eq!(
        BTreeSet::from_iter(nodes.iter().map(|n| {
            if let Some(d) = n.decision.clone() {
                d.proposals
            } else {
                BTreeMap::new()
            }
        }))
        .len(),
        1
    );

    nodes[0]
        .decision
        .clone()
        .expect("We should have seen a decision, this is a bug")
}

struct FakeProofKeyVerifier {}
impl sn_dbc::SpentProofKeyVerifier for FakeProofKeyVerifier {
    type Error = Error;

    fn verify_known_key(&self, _key: &bls::PublicKey) -> std::result::Result<(), Error> {
        Ok(())
    }
}

/// Reissue a new DBC (at a particular amount) from a given input DBC.
///
/// The change DBC will be discarded.
///
/// A spent proof share is generated for the input DBC, but it doesn't go through the complete
/// spending validation process. This should be OK for the testing process.
pub fn reissue_dbc(
    input: &Dbc,
    amount: u64,
    output_owner_sk: &bls::SecretKey,
    sap: &SectionAuthorityProvider,
    section_keys_provider: &SectionKeysProvider,
) -> Result<Dbc> {
    let output_amount = Token::from_nano(amount);
    let input_amount = input.amount_secrets_bearer()?.amount();
    let change_amount = input_amount
        .checked_sub(output_amount)
        .ok_or_else(|| eyre!("The input amount minus the amount must evaluate to a valid value"))?;

    let mut rng = rand::thread_rng();
    let output_owner = Owner::from(output_owner_sk.clone());
    let mut dbc_builder = TransactionBuilder::default()
        .set_decoys_per_input(0)
        .set_require_all_decoys(false)
        .add_input_dbc_bearer(input)?
        .add_output_by_amount(
            output_amount,
            OwnerOnce::from_owner_base(output_owner, &mut rng),
        )
        .add_output_by_amount(
            change_amount,
            OwnerOnce::from_owner_base(input.owner_base().clone(), &mut rng),
        )
        .build(rng)?;
    for (key_image, tx) in dbc_builder.inputs() {
        let public_commitments = get_public_commitments_from_transaction(
            &tx,
            &input.spent_proofs,
            &input.spent_transactions,
        )?;
        let public_commitments: Vec<Commitment> = public_commitments
            .into_iter()
            .flat_map(|(k, v)| if k == key_image { v } else { vec![] })
            .collect();
        let spent_proof_share = build_spent_proof_share(
            &key_image,
            &tx,
            sap,
            section_keys_provider,
            public_commitments,
        )?;
        dbc_builder = dbc_builder
            .add_spent_proof_share(spent_proof_share)
            .add_spent_transaction(tx);
    }
    let verifier = FakeProofKeyVerifier {};
    let output_dbcs = dbc_builder.build(&verifier)?;
    let (output_dbc, ..) = output_dbcs
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("At least one output DBC should have been generated"))?;
    Ok(output_dbc)
}

/// Gets a key image and a transaction that are ready to be used in a spend request.
pub fn get_input_dbc_spend_info(
    input: &Dbc,
    amount: u64,
    output_owner_sk: &bls::SecretKey,
) -> Result<(bls::PublicKey, RingCtTransaction)> {
    let output_amount = Token::from_nano(amount);
    let input_amount = input.amount_secrets_bearer()?.amount();
    let change_amount = input_amount
        .checked_sub(output_amount)
        .ok_or_else(|| eyre!("The input amount minus the amount must evaluate to a valid value"))?;

    let mut rng = rand::thread_rng();
    let output_owner = Owner::from(output_owner_sk.clone());
    let dbc_builder = TransactionBuilder::default()
        .set_decoys_per_input(0)
        .set_require_all_decoys(false)
        .add_input_dbc_bearer(input)?
        .add_output_by_amount(
            output_amount,
            OwnerOnce::from_owner_base(output_owner, &mut rng),
        )
        .add_output_by_amount(
            change_amount,
            OwnerOnce::from_owner_base(input.owner_base().clone(), &mut rng),
        )
        .build(rng)?;
    let inputs = dbc_builder.inputs();
    let first = inputs
        .first()
        .ok_or_else(|| eyre!("There must be at least one input on the transaction"))?;
    Ok(first.clone())
}

pub fn assert_lists<I, J, K>(a: I, b: J)
where
    K: fmt::Debug + Eq,
    I: IntoIterator<Item = K>,
    J: IntoIterator<Item = K>,
{
    let vec1: Vec<_> = a.into_iter().collect();
    let mut vec2: Vec<_> = b.into_iter().collect();

    assert_eq!(vec1.len(), vec2.len());

    for item1 in &vec1 {
        let idx2 = vec2
            .iter()
            .position(|item2| item1 == item2)
            .expect("Item not found in second list");

        vec2.swap_remove(idx2);
    }

    assert_eq!(vec2.len(), 0);
}
