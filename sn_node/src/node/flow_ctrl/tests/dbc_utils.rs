// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use eyre::{eyre, Result};
use sn_dbc::{
    Dbc, DbcTransaction, Owner, OwnerOnce, PublicKey, SpentProof, SpentProofShare, Token,
    TransactionBuilder,
};
use sn_interface::{dbcs::gen_genesis_dbc, messaging::data::RegisterCmd, types::ReplicatedData};
use std::collections::BTreeSet;

/// Get the spent proof share that's packaged inside the data that's to be replicated to the adults
/// in the section.
pub(crate) fn get_spent_proof_share_from_replicated_data(
    replicated_data: ReplicatedData,
) -> Result<SpentProofShare> {
    match replicated_data {
        ReplicatedData::SpentbookWrite(reg_cmd) => match reg_cmd {
            RegisterCmd::Edit(signed_edit) => {
                let entry = signed_edit.op.edit.crdt_op.value;
                let spent_proof_share: SpentProofShare = rmp_serde::from_slice(&entry)?;
                Ok(spent_proof_share)
            }
            _ => Err(eyre!("A RegisterCmd::Edit variant was expected")),
        },
        _ => Err(eyre!(
            "A ReplicatedData::SpentbookWrite variant was expected"
        )),
    }
}

/// Returns the info necessary to populate the `SpentbookCmd::Spend` message to be handled.
///
/// The genesis DBC is used, but that doesn't really matter; for testing the code in the message
/// handler we could use any DBC.
///
/// The `gen_genesis_dbc` function returns the DBC itself. To put it through the spending message
/// handler, it needs to have a transaction, which is what we provide here before we return it
/// back for use in tests.
pub(crate) fn get_genesis_dbc_spend_info(
    sk_set: &bls::SecretKeySet,
) -> Result<(
    PublicKey,
    DbcTransaction,
    BTreeSet<SpentProof>,
    BTreeSet<DbcTransaction>,
)> {
    let genesis_dbc = gen_genesis_dbc(sk_set, &sk_set.secret_key())?;
    let dbc_owner = genesis_dbc.owner_base().clone();
    let output_owner = OwnerOnce::from_owner_base(dbc_owner, &mut rand::thread_rng());
    let tx_builder = TransactionBuilder::default().add_input_dbc_bearer(&genesis_dbc)?;
    let inputs_amount_sum = tx_builder.inputs_amount_sum();
    let dbc_builder = tx_builder
        .add_output_by_amount(inputs_amount_sum, output_owner)
        .build(rand::thread_rng())?;
    let (public_key, tx) = &dbc_builder.inputs()[0];
    Ok((
        *public_key,
        tx.clone(),
        genesis_dbc.inputs_spent_proofs.clone(),
        genesis_dbc.inputs_spent_transactions,
    ))
}

pub(crate) fn reissue_invalid_dbc_with_no_inputs(
    input: &Dbc,
    amount: u64,
    output_owner_sk: &bls::SecretKey,
) -> Result<Dbc> {
    let output_amount = Token::from_nano(amount);
    let input_amount = input.amount_secrets_bearer()?.amount();
    let change_amount = input_amount
        .checked_sub(output_amount)
        .ok_or_else(|| eyre!("The input amount minus the amount must evaluate to a valid value"))?;

    let mut rng = rand::thread_rng();
    let output_owner = Owner::from(output_owner_sk.clone());
    let dbc_builder = TransactionBuilder::default()
        .add_output_by_amount(
            output_amount,
            OwnerOnce::from_owner_base(output_owner, &mut rng),
        )
        .add_output_by_amount(
            change_amount,
            OwnerOnce::from_owner_base(input.owner_base().clone(), &mut rng),
        )
        .build(rng)?;
    let output_dbcs = dbc_builder.build_without_verifying()?;
    let (output_dbc, ..) = output_dbcs
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("At least one output DBC should have been generated"))?;
    Ok(output_dbc)
}
