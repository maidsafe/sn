// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

pub use sn_dbc::{self as dbc, Dbc, DbcTransaction, Token};
pub use sn_interface::dbcs::DbcReason;

use super::{helpers::parse_tokens_amount, register::EntryHash};
use crate::{
    safeurl::{ContentType, SafeUrl, XorUrl},
    Error, Result, Safe,
};

use sn_client::Client;
use sn_dbc::{
    rng, AmountSecrets, Error as DbcError, Hash, Owner, OwnerOnce, PublicKey, SpentProof,
    SpentProofShare, TransactionBuilder,
};
use sn_interface::{elder_count, network_knowledge::supermajority};

use bytes::Bytes;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use tracing::{debug, warn};

/// Type tag to use for the Wallet stored on Register
pub const WALLET_TYPE_TAG: u64 = 1_000;

/// Set of spendable DBCs mapped to their friendly name as defined/chosen by the user when
/// depositing DBCs into a wallet.
pub type WalletSpendableDbcs = BTreeMap<String, (Dbc, EntryHash)>;

// Number of attempts to make trying to spend inputs when reissuing DBCs
// As the spend and query cmds are cascaded closely, there is high chance
// that the first two query attempts could both be failed.
// Hence the max number of attempts set to a higher value.
const NUM_OF_DBC_REISSUE_ATTEMPTS: u8 = 5;

/// Verifier required by sn_dbc API to check a SpentProof
/// is signed by known sections keys.
struct SpentProofKeyVerifier<'a> {
    client: &'a Client,
}

impl sn_dbc::SpentProofKeyVerifier for SpentProofKeyVerifier<'_> {
    type Error = crate::Error;

    // Called by sn_dbc API when it needs to verify a SpentProof is signed by a known key,
    // we check if the key is any of the network sections keys we are aware of
    fn verify_known_key(&self, key: &PublicKey) -> Result<()> {
        if !futures::executor::block_on(self.client.is_known_section_key(key)) {
            Err(Error::DbcVerificationFailed(format!(
                "SpentProof key is an unknown section key: {}",
                key.to_hex()
            )))
        } else {
            Ok(())
        }
    }
}

impl Safe {
    /// Create an empty wallet and return its XOR-URL.
    ///
    /// A wallet is stored on a private register.
    pub async fn wallet_create(&self) -> Result<XorUrl> {
        let xorurl = self.multimap_create(None, WALLET_TYPE_TAG).await?;

        let mut safeurl = SafeUrl::from_url(&xorurl)?;
        safeurl.set_content_type(ContentType::Wallet)?;

        Ok(safeurl.to_string())
    }

    /// Deposit a DBC in a wallet to make it a spendable balance.
    ///
    /// A name can optionally be specified for the deposit. If it isn't,
    /// part of the hash of the DBC content will be used.
    /// Note this won't perform a verification on the network to check if the the DBC has
    /// been already spent, the user can call to `is_dbc_spent` API for that purpose beforehand.
    ///
    /// Returns the name that was set, along with the deposited amount.
    pub async fn wallet_deposit(
        &self,
        wallet_url: &str,
        spendable_name: Option<&str>,
        dbc: &Dbc,
        secret_key: Option<bls::SecretKey>,
    ) -> Result<(String, Token)> {
        let dbc_to_deposit = if dbc.is_bearer() {
            if secret_key.is_some() {
                return Err(Error::DbcDepositError(
                    "A secret key should not be supplied when depositing a bearer DBC".to_string(),
                ));
            }
            dbc.clone()
        } else if let Some(sk) = secret_key {
            let mut owned_dbc = dbc.clone();
            owned_dbc.to_bearer(&sk).map_err(|err| {
                if let DbcError::DbcBearerConversionFailed(_) = err {
                    Error::DbcDepositInvalidSecretKey
                } else {
                    Error::DbcDepositError(err.to_string())
                }
            })?;
            owned_dbc
        } else {
            return Err(Error::DbcDepositError(
                "A secret key must be provided to deposit an owned DBC".to_string(),
            ));
        };

        // Verify that the DBC to deposit is valid. This verifies there is a matching transaction
        // provided for each SpentProof, although this does not check if the DBC has been spent.
        let proof_key_verifier = SpentProofKeyVerifier {
            client: self.get_safe_client()?,
        };
        dbc_to_deposit.verify(
            &dbc_to_deposit.owner_base().secret_key()?,
            &proof_key_verifier,
        )?;

        let spendable_name = match spendable_name {
            Some(name) => name.to_string(),
            None => format!("dbc-{}", &hex::encode(dbc_to_deposit.hash())[0..8]),
        };

        let amount = dbc_to_deposit
            .amount_secrets_bearer()
            .map(|amount_secrets| amount_secrets.amount())?;

        let safeurl = self.parse_and_resolve_url(wallet_url).await?;
        self.insert_dbc_into_wallet(&safeurl, &dbc_to_deposit, spendable_name.clone())
            .await?;

        debug!(
            "A spendable DBC deposited (amount: {}) into wallet at {}, with name: {}",
            amount, safeurl, spendable_name
        );

        Ok((spendable_name, amount))
    }

    /// Verify if the provided DBC's public_key has been already spent on the network.
    pub async fn is_dbc_spent(&self, public_key: PublicKey) -> Result<bool> {
        let client = self.get_safe_client()?;
        let spent_proof_shares = client.spent_proof_shares(public_key).await?;

        // We obtain a set of unique spent transactions hash the shares belong to
        let spent_transactions: BTreeSet<Hash> = spent_proof_shares
            .iter()
            .map(|share| share.content.transaction_hash)
            .collect();

        let proof_key_verifier = SpentProofKeyVerifier { client };

        // Among all different proof shares that could have been signed for different
        // transactions, let's try to find one set of shares which can actually
        // be aggregated onto a valid proof signature for the provided DBC's public_key,
        // and which is signed by a known section key.
        let is_spent = spent_transactions.into_iter().any(|tx_hash| {
            let shares_for_current_tx = spent_proof_shares
                .iter()
                .cloned()
                .filter(|share| share.content.transaction_hash == tx_hash)
                .collect();

            verify_spent_proof_shares_for_tx(
                public_key,
                tx_hash,
                &shares_for_current_tx,
                &proof_key_verifier,
            )
            .is_ok()
        });

        Ok(is_spent)
    }

    /// Fetch a wallet from a Url performing all type of URL resolution required.
    /// Return the set of spendable DBCs found in the wallet.
    pub async fn wallet_get(&self, wallet_url: &str) -> Result<WalletSpendableDbcs> {
        let safeurl = self.parse_and_resolve_url(wallet_url).await?;
        debug!("Wallet URL was parsed and resolved to: {}", safeurl);
        self.fetch_wallet(&safeurl).await
    }

    /// Fetch a wallet from a `SafeUrl` without performing any type of URL resolution
    pub(crate) async fn fetch_wallet(&self, safeurl: &SafeUrl) -> Result<WalletSpendableDbcs> {
        let entries = match self.fetch_multimap(safeurl).await {
            Ok(entries) => entries,
            Err(Error::AccessDenied(_)) => {
                return Err(Error::AccessDenied(format!(
                    "Couldn't read wallet found at \"{safeurl}\"",
                )))
            }
            Err(Error::ContentNotFound(_)) => {
                return Err(Error::ContentNotFound(format!(
                    "No wallet found at {safeurl}",
                )))
            }
            Err(err) => {
                return Err(Error::ContentError(format!(
                    "Failed to read balances from wallet: {err}",
                )))
            }
        };

        let mut balances = WalletSpendableDbcs::default();
        for (entry_hash, (key, value)) in &entries {
            let xorurl_str = std::str::from_utf8(value)?;
            let dbc_xorurl = SafeUrl::from_xorurl(xorurl_str)?;
            let dbc_bytes = self.fetch_data(&dbc_xorurl, None).await?;

            let dbc: Dbc = match rmp_serde::from_slice(&dbc_bytes) {
                Ok(dbc) => dbc,
                Err(err) => {
                    warn!("Ignoring entry found in wallet since it cannot be deserialised as a valid DBC: {:?}", err);
                    continue;
                }
            };

            let spendable_name = std::str::from_utf8(key)?.to_string();
            balances.insert(spendable_name, (dbc, *entry_hash));
        }

        Ok(balances)
    }

    /// Check the total balance of a wallet found at a given XOR-URL
    pub async fn wallet_balance(&self, wallet_url: &str) -> Result<Token> {
        debug!("Finding total wallet balance for: {}", wallet_url);

        // Let's get the list of balances from the Wallet
        let balances = self.wallet_get(wallet_url).await?;
        debug!("Spendable balances to check: {:?}", balances);

        // Iterate through the DBCs adding up the amounts
        let mut total_balance = Token::from_nano(0);
        for (name, (dbc, _)) in &balances {
            debug!("Checking spendable balance named: {}", name);

            let balance = match dbc.amount_secrets_bearer() {
                Ok(amount_secrets) => amount_secrets.amount(),
                Err(err) => {
                    warn!("Ignoring amount from DBC found in wallet due to error in revealing secret amount: {:?}", err);
                    continue;
                }
            };
            debug!("Amount in spendable balance '{}': {}", name, balance);

            match total_balance.checked_add(balance) {
                None => {
                    return Err(Error::ContentError(format!(
                        "Failed to calculate total balance due to overflow when adding {balance} to {total_balance}",

                    )))
                }
                Some(new_total_balance) => total_balance = new_total_balance,
            }
        }

        Ok(total_balance)
    }

    /// Reissue a DBC from a wallet and return the output DBC.
    ///
    /// If you pass `None` for the `owner_public_key` argument, the output DBC will be a bearer. If
    /// the public key is specified, the output DBC will be owned by the person in possession of the
    /// secret key corresponding to the public key.
    ///
    /// If there is change from the transaction, the change DBC will be deposited in the source
    /// wallet.
    ///
    /// Spent DBCs are marked as removed from the source wallet, but since all entries are kept in
    /// the history, they can still be retrieved if desired by the user.
    pub async fn wallet_reissue(
        &self,
        wallet_url: &str,
        amount: &str,
        owner_public_key: Option<bls::PublicKey>,
        reason: DbcReason,
    ) -> Result<Dbc> {
        debug!(
            "Reissuing DBC from wallet at {} for an amount of {} tokens",
            wallet_url, amount
        );
        let dbcs = self
            .wallet_reissue_many(
                wallet_url,
                [(amount.to_string(), owner_public_key)]
                    .into_iter()
                    .collect(),
                reason,
            )
            .await?;

        dbcs.into_iter()
            .next()
            .ok_or_else(|| Error::DbcReissueError(
                "Unexpectedly failed to generate output DBC. No balance were removed from the wallet.".to_string(),
            ))
    }

    /// Reissue several DBCs from a wallet.
    ///
    /// This works exactly the same as `wallet_reissue` API with the only difference that
    /// this function allows to reissue from a single wallet several output DBCs instead
    /// of a single one. If there is change from the transaction, the change DBC will be
    /// deposited in the source wallet.
    pub async fn wallet_reissue_many(
        &self,
        wallet_url: &str,
        outputs: Vec<(String, Option<bls::PublicKey>)>,
        reason: DbcReason,
    ) -> Result<Vec<Dbc>> {
        let mut total_output_amount = Token::zero();
        let mut outputs_owners = Vec::<(Token, OwnerOnce)>::new();
        for (amount, owner_pk) in outputs {
            let output_amount = parse_tokens_amount(&amount)?;
            if output_amount.as_nano() == 0 {
                return Err(Error::InvalidAmount(
                    "Output amount to reissue needs to be larger than zero (0).".to_string(),
                ));
            }

            total_output_amount =
                total_output_amount
                    .checked_add(output_amount)
                    .ok_or_else(|| {
                        Error::DbcReissueError(
                        "Overflow occurred while calculating the total amount for the output DBC"
                            .to_string(),
                    )
                    })?;

            let output_owner = if let Some(pk) = owner_pk {
                let owner = Owner::from(pk);
                OwnerOnce::from_owner_base(owner, &mut rng::thread_rng())
            } else {
                let owner = Owner::from_random_secret_key(&mut rng::thread_rng());
                OwnerOnce::from_owner_base(owner, &mut rng::thread_rng())
            };

            outputs_owners.push((output_amount, output_owner));
        }

        let safeurl = self.parse_and_resolve_url(wallet_url).await?;
        let spendable_dbcs = self.fetch_wallet(&safeurl).await?;

        // From the spendable dbcs, we select the number required to cover the
        // amount going to the output dbcs.
        #[cfg(feature = "data-network")]
        let (input_dbcs_to_spend, input_dbcs_entries_hash, outputs_owners, change_amount) =
            Self::select_inputs(spendable_dbcs, total_output_amount, outputs_owners)?;
        #[cfg(not(feature = "data-network"))]
        let (input_dbcs_to_spend, input_dbcs_entries_hash, outputs_owners, change_amount) = {
            let client = self.get_safe_client()?;
            Self::select_inputs_with_fees(
                client,
                spendable_dbcs,
                total_output_amount,
                outputs_owners,
            )
            .await?
        };

        // We can now reissue the output DBCs
        let (output_dbcs, change_dbc) = self
            .reissue_dbcs(input_dbcs_to_spend, outputs_owners, reason, change_amount)
            .await?;

        if output_dbcs.is_empty() {
            return Err(Error::DbcReissueError(
                "Unexpectedly failed to generate output DBC. No balance were removed from the wallet.".to_string(),
            ));
        }

        if let Some(change_dbc) = change_dbc {
            self.insert_dbc_into_wallet(
                &safeurl,
                &change_dbc,
                format!("change-dbc-{}", &hex::encode(change_dbc.hash())[0..8]),
            )
            .await?;
        }

        // (virtually) remove input DBCs in the source wallet
        self.multimap_remove(&safeurl.to_string(), input_dbcs_entries_hash)
            .await?;

        Ok(output_dbcs.into_iter().map(|(dbc, _, _)| dbc).collect())
    }

    /// -------------------------------------------------
    ///  ------- Private helpers -------
    ///-------------------------------------------------

    ///
    #[cfg(feature = "data-network")]
    fn select_inputs(
        spendable_dbcs: WalletSpendableDbcs,
        total_output_amount: Token,
        outputs_owners: Vec<(Token, OwnerOnce)>,
    ) -> Result<SelectedInputs> {
        // We'll combine one or more input DBCs and reissue:
        // - one output DBC for the recipient,
        // - and a second DBC for the change, which will be stored in the source wallet.
        let mut input_dbcs_to_spend = Vec::<Dbc>::new();
        let mut input_dbcs_entries_hash = BTreeSet::<EntryHash>::new();
        let mut total_input_amount = Token::zero();
        let mut change_amount = total_output_amount;
        for (name, (dbc, entry_hash)) in spendable_dbcs {
            let dbc_balance = match dbc.amount_secrets_bearer() {
                Ok(amount_secrets) => amount_secrets.amount(),
                Err(err) => {
                    warn!("Ignoring input DBC found in wallet (entry: {}) due to error in revealing secret amount: {:?}", name, err);
                    continue;
                }
            };

            // Add this DBC as input to be spent.
            input_dbcs_to_spend.push(dbc);
            input_dbcs_entries_hash.insert(entry_hash);
            // Input amount increases with the amount of the dbc.
            total_input_amount = total_input_amount.checked_add(dbc_balance)
                .ok_or_else(|| {
                    Error::DbcReissueError(
                        "Overflow occurred while increasing total input amount while trying to cover the output DBCs."
                        .to_string(),
                )
                })?;

            // If we've already combined input DBCs for the total output amount, then stop.
            match change_amount.checked_sub(dbc_balance) {
                Some(pending_output) => {
                    change_amount = pending_output;
                    if change_amount.as_nano() == 0 {
                        break;
                    }
                }
                None => {
                    change_amount =
                        Token::from_nano(dbc_balance.as_nano() - change_amount.as_nano());
                    break;
                }
            }
        }

        // If not enough spendable was found, this check will return an error.
        Self::verify_amounts(total_input_amount, total_output_amount)?;

        Ok((
            input_dbcs_to_spend,
            input_dbcs_entries_hash,
            outputs_owners,
            change_amount,
        ))
    }

    ///
    /// NB: Upper layer should validate *estimated* fees against client preferences.
    async fn select_inputs_with_fees(
        client: &Client,
        spendable_dbcs: WalletSpendableDbcs,
        mut total_output_amount: Token,
        mut outputs_owners: Vec<(Token, OwnerOnce)>,
    ) -> Result<SelectedInputs> {
        // We'll combine one or more input DBCs and reissue:
        // - one output DBC for the recipient,
        // - and a second DBC for the change, which will be stored in the source wallet.
        let mut input_dbcs_to_spend = Vec::<Dbc>::new();
        let mut input_dbcs_entries_hash = BTreeSet::<EntryHash>::new();
        let mut total_input_amount = Token::zero();
        let mut change_amount = total_output_amount;

        for (name, (dbc, entry_hash)) in spendable_dbcs {
            // TODO: Query the network, one section per input, for the current fee.
            // Right now, now fees are added, as a dummy is used instead.

            let input_key = dbc.as_revealed_input_bearer()?.public_key();
            // each mint will have elder_count() instances to pay individually (for now, later they will be more)
            let mint_fees: BTreeMap<PublicKey, Token> = client.get_mint_fees(input_key).await?;
            let required_num_mints = supermajority(elder_count());
            if required_num_mints > mint_fees.len() {
                warn!("Not enough mints contacted for the section to spend the input. Found: {}, needed: {required_num_mints}", mint_fees.len());
                continue;
            }

            // Total fee paid to all recipients in the section for this input.
            let fee_per_input = mint_fees
                .values()
                .fold(Some(Token::zero()), |total, fee| {
                    total.and_then(|t| t.checked_add(*fee))
                })
                .ok_or_else(|| Error::DbcReissueError(
                    "Overflow occurred while summing the individual Elder's fees in order to calculate the total amount for the output DBCs."
                        .to_string(),
                ))?;

            // Add mints to outputs.
            mint_fees.into_iter().for_each(|(pk, fee)| {
                let owner = Owner::from(pk);
                let owner_once = OwnerOnce::from_owner_base(owner, &mut rng::thread_rng());
                outputs_owners.push((fee, owner_once));
            });

            let dbc_balance = match dbc.amount_secrets_bearer() {
                Ok(amount_secrets) => amount_secrets.amount(),
                Err(err) => {
                    warn!("Ignoring input DBC found in wallet (entry: {}) due to error in revealing secret amount: {:?}", name, err);
                    continue;
                }
            };

            // Add this DBC as input to be spent.
            input_dbcs_to_spend.push(dbc);
            input_dbcs_entries_hash.insert(entry_hash);

            // Input amount increases with the amount of the dbc.
            total_input_amount = total_input_amount.checked_add(dbc_balance)
                .ok_or_else(|| {
                    Error::DbcReissueError(
                        "Overflow occurred while increasing total input amount while trying to cover the output DBCs."
                        .to_string(),
                )
                })?;

            // Output amount now increases a bit, as we have to cover the fee as well..
            total_output_amount = total_output_amount.checked_add(fee_per_input)
                .ok_or_else(|| {
                    Error::DbcReissueError(
                    "Overflow occurred while adding mint fee in order to calculate the total amount for the output DBCs."
                        .to_string(),
                )
                })?;
            // ..and so does `change_amount` (that we subtract from to know if we've covered `total_output_amount`).
            change_amount = change_amount.checked_add(fee_per_input)
                .ok_or_else(|| {
                    Error::DbcReissueError(
                    "Overflow occurred while adding mint fee in order to calculate the total amount for the output DBCs."
                        .to_string(),
                )
                })?;

            // If we've already combined input DBCs for the total output amount, then stop.
            match change_amount.checked_sub(dbc_balance) {
                Some(pending_output) => {
                    change_amount = pending_output;
                    if change_amount.as_nano() == 0 {
                        break;
                    }
                }
                None => {
                    change_amount =
                        Token::from_nano(dbc_balance.as_nano() - change_amount.as_nano());
                    break;
                }
            }
        }

        // If not enough spendable was found, this check will return an error.
        Self::verify_amounts(total_input_amount, total_output_amount)?;

        Ok((
            input_dbcs_to_spend,
            input_dbcs_entries_hash,
            outputs_owners,
            change_amount,
        ))
    }

    // Make sure total input amount gathered with input DBCs are enough for the output amount
    fn verify_amounts(total_input_amount: Token, total_output_amount: Token) -> Result<()> {
        if total_output_amount > total_input_amount {
            return Err(Error::NotEnoughBalance(total_input_amount.to_string()));
        }
        Ok(())
    }

    /// Insert a DBC into the wallet's underlying `Multimap`.
    async fn insert_dbc_into_wallet(
        &self,
        safeurl: &SafeUrl,
        dbc: &Dbc,
        spendable_name: String,
    ) -> Result<()> {
        if !dbc.is_bearer() {
            return Err(Error::InvalidInput("Only bearer DBC's are supported at this point by the wallet. Please deposit a bearer DBC's.".to_string()));
        }

        let dbc_bytes = Bytes::from(rmp_serde::to_vec_named(dbc).map_err(|err| {
            Error::Serialisation(format!(
                "Failed to serialise DBC to insert it into the wallet: {err:?}",
            ))
        })?);

        let dbc_xorurl = self.store_bytes(dbc_bytes, None).await?;

        let entry = (spendable_name.into_bytes(), dbc_xorurl.into_bytes());
        let _entry_hash = self
            .multimap_insert(&safeurl.to_string(), entry, BTreeSet::default())
            .await?;

        Ok(())
    }

    /// Reissue DBCs and log the spent input DBCs on the network. Return the output DBC and the
    /// change DBC if there is one.
    pub(super) async fn reissue_dbcs(
        &self,
        input_dbcs: Vec<Dbc>,
        outputs: Vec<(Token, OwnerOnce)>,
        reason: DbcReason,
        change_amount: Token,
    ) -> Result<(Vec<(Dbc, OwnerOnce, AmountSecrets)>, Option<Dbc>)> {
        let mut tx_builder = TransactionBuilder::default()
            .add_inputs_dbc_bearer(input_dbcs.iter())?
            .add_outputs_by_amount(outputs.into_iter().map(|(token, owner)| (token, owner)));

        let client = self.get_safe_client()?;
        let change_owneronce =
            OwnerOnce::from_owner_base(client.dbc_owner().clone(), &mut rng::thread_rng());
        if change_amount.as_nano() > 0 {
            tx_builder = tx_builder.add_output_by_amount(change_amount, change_owneronce.clone());
        }

        let inputs_spent_proofs: BTreeSet<SpentProof> = input_dbcs
            .iter()
            .flat_map(|dbc| dbc.inputs_spent_proofs.clone())
            .collect();

        let inputs_spent_transactions: BTreeSet<DbcTransaction> = input_dbcs
            .iter()
            .flat_map(|dbc| dbc.inputs_spent_transactions.clone())
            .collect();

        let proof_key_verifier = SpentProofKeyVerifier { client };

        // Let's build the output DBCs
        let mut dbc_builder = tx_builder.build(rng::thread_rng())?;

        // Spend all the input DBCs, collecting the spent proof shares for each of them
        for (public_key, tx) in dbc_builder.inputs() {
            let tx_hash = Hash::from(tx.hash());
            // TODO: spend DBCs concurrently spawning tasks
            let mut attempts = 0;
            loop {
                attempts += 1;
                client
                    .spend_dbc(
                        public_key,
                        tx.clone(),
                        reason,
                        inputs_spent_proofs.clone(),
                        inputs_spent_transactions.clone(),
                    )
                    .await?;

                let spent_proof_shares = client.spent_proof_shares(public_key).await?;

                // TODO: we temporarilly filter the spent proof shares which correspond to the TX we
                // are spending now. This is because current implementation of Spentbook allows
                // double spents, so we may be retrieving spent proof shares for others spent TXs.
                let shares_for_current_tx: HashSet<SpentProofShare> = spent_proof_shares
                    .into_iter()
                    .filter(|proof_share| proof_share.content.transaction_hash == tx_hash)
                    .collect();

                match verify_spent_proof_shares_for_tx(
                    public_key,
                    tx_hash,
                    &shares_for_current_tx,
                    &proof_key_verifier,
                ) {
                    Ok(()) => {
                        dbc_builder = dbc_builder
                            .add_spent_proof_shares(shares_for_current_tx.into_iter())
                            .add_spent_transaction(tx);

                        break;
                    }
                    Err(err) if attempts == NUM_OF_DBC_REISSUE_ATTEMPTS => {
                        return Err(Error::DbcReissueError(format!(
                            "Failed to spend input, {} proof shares obtained from spentbook: {}",
                            shares_for_current_tx.len(),
                            err
                        )));
                    }
                    Err(_) => {}
                }
            }
        }

        // Perform verifications of input TX and spentproofs,
        // as well as building the output DBCs.
        let mut output_dbcs = dbc_builder.build(&proof_key_verifier)?;

        let mut change_dbc = None;
        output_dbcs.retain(|(dbc, owneronce, _)| {
            if owneronce == &change_owneronce && change_amount.as_nano() > 0 {
                change_dbc = Some(dbc.clone());
                false
            } else {
                true
            }
        });

        Ok((output_dbcs, change_dbc))
    }
}

type SelectedInputs = (
    Vec<Dbc>,
    BTreeSet<EntryHash>,
    Vec<(Token, OwnerOnce)>,
    Token,
);

// Private helper to verify if a set of spent proof shares are valid for a given public_key and TX
fn verify_spent_proof_shares_for_tx(
    public_key: PublicKey,
    tx_hash: Hash,
    proof_shares: &HashSet<sn_dbc::SpentProofShare>,
    proof_key_verifier: &SpentProofKeyVerifier,
) -> Result<()> {
    SpentProof::try_from_proof_shares(public_key, tx_hash, proof_shares)
        .and_then(|spent_proof| spent_proof.verify(tx_hash, proof_key_verifier))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_helpers::{
        get_next_bearer_dbc, new_read_only_safe_instance, new_safe_instance,
        new_safe_instance_with_dbc, new_safe_instance_with_dbc_owner, GENESIS_DBC,
    };

    use sn_client::{Error as ClientError, ErrorMsg};
    use sn_dbc::{Error as DbcError, Owner};
    use sn_interface::network_knowledge::DEFAULT_ELDER_COUNT;

    use anyhow::{anyhow, Result};
    use xor_name::XorName;

    #[cfg(feature = "data-network")]
    const FEE_PER_INPUT: u64 = 0;
    #[cfg(not(feature = "data-network"))]
    const FEE_PER_INPUT: u64 = DEFAULT_ELDER_COUNT as u64;

    #[tokio::test]
    async fn test_wallet_create() -> Result<()> {
        let safe = new_safe_instance().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        assert!(wallet_xorurl.starts_with("safe://"));

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, Token::zero());

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_bearer_dbc() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        let (_, amount) = safe
            .wallet_deposit(&wallet_xorurl, None, &dbc, None)
            .await?;
        assert_eq!(amount, dbc_balance);

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;
        assert_eq!(wallet_balances.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_name() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let (name, amount) = safe
            .wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, None)
            .await?;
        assert_eq!(name, "my-dbc");
        assert_eq!(amount, dbc_balance);

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;
        assert!(wallet_balances.contains_key("my-dbc"));

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_no_name() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        let (name, amount) = safe
            .wallet_deposit(&wallet_xorurl, None, &dbc, None)
            .await?;
        assert_eq!(amount, dbc_balance);
        assert_eq!(name, format!("dbc-{}", &hex::encode(dbc.hash())[0..8]));

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;
        assert!(wallet_balances.contains_key(&name));

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_owned_dbc() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let sk = bls::SecretKey::random();

        safe.wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, None)
            .await?;
        let owned_dbc = safe
            .wallet_reissue(
                &wallet_xorurl,
                "2.35",
                Some(sk.public_key()),
                DbcReason::none(),
            )
            .await?;
        safe.wallet_deposit(
            &wallet_xorurl,
            Some("owned-dbc"),
            &owned_dbc,
            Some(sk.clone()),
        )
        .await?;

        let owner = Owner::from(sk);
        let balances = safe.wallet_get(&wallet_xorurl).await?;
        let (owned_dbc, _) = balances
            .get("owned-dbc")
            .ok_or_else(|| anyhow!("Couldn't read DBC from wallet"))?;
        assert_eq!(*owned_dbc.owner_base(), owner);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_owned_dbc_without_providing_secret_key() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let pk = bls::SecretKey::random().public_key();

        safe.wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, None)
            .await?;
        let owned_dbc = safe
            .wallet_reissue(&wallet_xorurl, "2.35", Some(pk), DbcReason::none())
            .await?;
        let result = safe
            .wallet_deposit(&wallet_xorurl, Some("owned-dbc"), &owned_dbc, None)
            .await;
        match result {
            Ok(_) => Err(anyhow!(
                "This test case should result in an error".to_string()
            )),
            Err(Error::DbcDepositError(e)) => {
                assert_eq!(e, "A secret key must be provided to deposit an owned DBC");
                Ok(())
            }
            Err(_) => Err(anyhow!("This test should use a DbcDepositError".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_owned_dbc_with_invalid_secret_key() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let sk = bls::SecretKey::random();
        let sk2 = bls::SecretKey::random();
        let pk = sk.public_key();

        safe.wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, None)
            .await?;
        let owned_dbc = safe
            .wallet_reissue(&wallet_xorurl, "2.35", Some(pk), DbcReason::none())
            .await?;
        let result = safe
            .wallet_deposit(&wallet_xorurl, Some("owned-dbc"), &owned_dbc, Some(sk2))
            .await;
        match result {
            Ok(_) => Err(anyhow!(
                "This test case should result in an error".to_string()
            )),
            Err(Error::DbcDepositInvalidSecretKey) => Ok(()),
            Err(_) => Err(anyhow!(
                "This test should use a DbcDepositInvalidSecretKey error".to_string()
            )),
        }
    }

    #[tokio::test]
    async fn test_wallet_deposit_with_bearer_dbc_and_secret_key() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let sk = bls::SecretKey::random();

        let result = safe
            .wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, Some(sk))
            .await;
        match result {
            Ok(_) => Err(anyhow!(
                "This test case should result in an error".to_string()
            )),
            Err(Error::DbcDepositError(e)) => {
                assert_eq!(
                    e,
                    "A secret key should not be supplied when depositing a bearer DBC"
                );
                Ok(())
            }
            Err(_) => Err(anyhow!("This test should use a DbcDepositError".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_deposited_owned_dbc() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;
        let wallet2_xorurl = safe.wallet_create().await?;
        let sk = bls::SecretKey::random();

        safe.wallet_deposit(&wallet_xorurl, Some("my-dbc"), &dbc, None)
            .await?;
        let owned_dbc = safe
            .wallet_reissue(
                &wallet_xorurl,
                "2.35",
                Some(sk.public_key()),
                DbcReason::none(),
            )
            .await?;
        // Deposit the owned DBC in another wallet because it's easier to ensure this owned DBC
        // will be used as an input in the next reissue rather than having to be precise about
        // balances.
        safe.wallet_deposit(
            &wallet2_xorurl,
            Some("owned-dbc"),
            &owned_dbc,
            Some(sk.clone()),
        )
        .await?;

        let result = safe
            .wallet_reissue(&wallet2_xorurl, "2", None, DbcReason::none())
            .await;
        match result {
            Ok(_) => {
                // For this case, we just want to make sure the reissue went through without an
                // error, which means the owned DBC was used as an input. There are other test
                // cases that verify balances are correct and so on, we don't need to do that again
                // here.
                Ok(())
            }
            Err(e) => Err(anyhow!(e)),
        }
    }

    #[tokio::test]
    async fn test_wallet_balance() -> Result<()> {
        let (safe, dbc1, dbc1_balance) = new_safe_instance_with_dbc().await?;
        let (dbc2, dbc2_balance) = get_next_bearer_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        // We deposit the first DBC
        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc1, None)
            .await?;

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, dbc1_balance);

        // ...and a second DBC
        safe.wallet_deposit(&wallet_xorurl, Some("my-second-dbc"), &dbc2, None)
            .await?;

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(
            current_balance.as_nano(),
            dbc1_balance.as_nano() + dbc2_balance.as_nano()
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_balance_overflow() -> Result<()> {
        let safe = new_safe_instance().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        for i in 0..5 {
            safe.wallet_deposit(
                &wallet_xorurl,
                Some(&format!("my-dbc-#{i}")),
                &GENESIS_DBC,
                None,
            )
            .await?;
        }

        let genesis_balance = 4_525_524_120_000_000_000;
        match safe.wallet_balance(&wallet_xorurl).await {
            Err(Error::ContentError(msg)) => {
                assert_eq!(
                    msg,
                    format!(
                        "Failed to calculate total balance due to overflow when adding {} to {}",
                        Token::from_nano(genesis_balance),
                        Token::from_nano(genesis_balance * 4)
                    )
                );
                Ok(())
            }
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(balance) => Err(anyhow!("Wallet balance obtained unexpectedly: {}", balance)),
        }
    }

    #[tokio::test]
    async fn test_wallet_get() -> Result<()> {
        let (safe, dbc1, dbc1_balance) = new_safe_instance_with_dbc().await?;
        let (dbc2, dbc2_balance) = get_next_bearer_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc1, None)
            .await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-second-dbc"), &dbc2, None)
            .await?;

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;

        let (dbc1_read, _) = wallet_balances
            .get("my-first-dbc")
            .ok_or_else(|| anyhow!("Couldn't read first DBC from fetched wallet"))?;
        assert_eq!(dbc1_read.owner_base(), dbc1.owner_base());
        let balance1 = dbc1_read
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from first DBC fetched: {:?}", err))?;
        assert_eq!(balance1.amount(), dbc1_balance);

        let (dbc2_read, _) = wallet_balances
            .get("my-second-dbc")
            .ok_or_else(|| anyhow!("Couldn't read second DBC from fetched wallet"))?;
        assert_eq!(dbc2_read.owner_base(), dbc2.owner_base());
        let balance2 = dbc2_read
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from second DBC fetched: {:?}", err))?;
        assert_eq!(balance2.amount(), dbc2_balance);

        Ok(())
    }

    /// Ignoring until we implement encryption support again.
    #[ignore]
    #[tokio::test]
    async fn test_wallet_get_not_owned_wallet() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc, None)
            .await?;

        // test it fails to get a not owned wallet
        let read_only_safe = new_read_only_safe_instance().await?;
        match read_only_safe.wallet_get(&wallet_xorurl).await {
            Err(Error::AccessDenied(msg)) => {
                assert_eq!(
                    msg,
                    format!("Couldn't read wallet found at \"{wallet_xorurl}\"")
                );
                Ok(())
            }
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(_) => Err(anyhow!("Wallet get succeeded unexpectedly".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_get_non_compatible_content() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc, None)
            .await?;

        // We insert an entry (to its underlying data type, i.e. the Multimap) which is
        // not a valid serialised DBC, thus making part of its content incompatible/corrupted.
        let corrupted_dbc_xorurl = safe.store_bytes(Bytes::from_static(b"bla"), None).await?;
        let entry = (b"corrupted-dbc".to_vec(), corrupted_dbc_xorurl.into_bytes());
        safe.multimap_insert(&wallet_xorurl, entry, BTreeSet::default())
            .await?;

        // Now check the Wallet can still be read and the corrupted entry is ignored
        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, dbc_balance);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_multiple_input_dbcs() -> Result<()> {
        let (safe, dbc1, dbc1_balance) = new_safe_instance_with_dbc().await?;
        let (dbc2, dbc2_balance) = get_next_bearer_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc1, None)
            .await?;
        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-2"), &dbc2, None)
            .await?;

        let change_plus_fees = 100;
        let expected_change = change_plus_fees - (2 * FEE_PER_INPUT); // 2 dbc inputs = 2 fees

        let amount_to_reissue =
            Token::from_nano(dbc1_balance.as_nano() + dbc2_balance.as_nano() - change_plus_fees);
        let output_dbc = safe
            .wallet_reissue(
                &wallet_xorurl,
                &amount_to_reissue.to_string(),
                None,
                DbcReason::none(),
            )
            .await?;

        let output_balance = output_dbc
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from output DBC: {:?}", err))?;
        assert_eq!(output_balance.amount(), amount_to_reissue);

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, Token::from_nano(expected_change));

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;

        assert_eq!(wallet_balances.len(), 1);

        let (_, (change_dbc_read, _)) = wallet_balances
            .iter()
            .next()
            .ok_or_else(|| anyhow!("Couldn't read change DBC from fetched wallet"))?;
        let change = change_dbc_read
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from change DBC fetched: {:?}", err))?;
        assert_eq!(change.amount(), Token::from_nano(expected_change));

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_single_input_dbc() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc, None)
            .await?;

        let output_dbc = safe
            .wallet_reissue(&wallet_xorurl, "1", None, DbcReason::none())
            .await?;

        let output_balance = output_dbc
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from output DBC: {:?}", err))?;
        assert_eq!(output_balance.amount(), Token::from_nano(1_000_000_000));

        let change_amount = Token::from_nano(dbc_balance.as_nano() - 1_000_000_000 - FEE_PER_INPUT); // 1 dbc input = 1 fee
        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;

        assert_eq!(current_balance, change_amount);

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;

        assert_eq!(wallet_balances.len(), 1);

        let (_, (change_dbc_read, _)) = wallet_balances
            .iter()
            .next()
            .ok_or_else(|| anyhow!("Couldn't read change DBC from fetched wallet"))?;
        let change = change_dbc_read
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from change DBC fetched: {:?}", err))?;
        assert_eq!(change.amount(), change_amount);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_persistent_dbc_owner() -> Result<()> {
        let (safe, dbc_owner) = new_safe_instance_with_dbc_owner(
            "3917ad935714cf1e71b9b5e2831684811e83acc6c10f030031fe886292152e83",
        )
        .await?;
        let wallet_xorurl = safe.wallet_create().await?;

        let (_safe, dbc, _) = new_safe_instance_with_dbc().await?;
        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc, None)
            .await?;

        let _ = safe
            .wallet_reissue(&wallet_xorurl, "1", None, DbcReason::none())
            .await?;
        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;

        let (_, (change_dbc_read, _)) = wallet_balances
            .iter()
            .next()
            .ok_or_else(|| anyhow!("Couldn't read change DBC from fetched wallet"))?;
        assert_eq!(*change_dbc_read.owner_base(), dbc_owner);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_owned_dbc() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc, None)
            .await?;

        let pk = bls::SecretKey::random().public_key();
        let owner = Owner::from(pk);
        let output_dbc = safe
            .wallet_reissue(&wallet_xorurl, "1", Some(pk), DbcReason::none())
            .await?;

        // We have verified transaction details in other tests. In this test, we're just concerned
        // with the owner being assigned correctly.
        assert_eq!(owner, *output_dbc.owner_base());

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_reason() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc, None)
            .await?;

        let pk = bls::SecretKey::random().public_key();
        let just_any_xorname = XorName::from_content(&[1, 2, 3, 4]);
        let any_reason = DbcReason::from(just_any_xorname);
        let output_dbc = safe
            .wallet_reissue(&wallet_xorurl, "1", Some(pk), any_reason)
            .await?;

        assert_eq!(Some(any_reason.into()), output_dbc.reason());

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_not_enough_balance() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc"), &dbc, None)
            .await?;

        match safe
            .wallet_reissue(
                &wallet_xorurl,
                &Token::from_nano(dbc_balance.as_nano() + 1).to_string(),
                None,
                DbcReason::none(),
            )
            .await
        {
            Err(Error::NotEnoughBalance(msg)) => {
                assert_eq!(msg, dbc_balance.to_string());
                Ok(())
            }
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(_) => Err(anyhow!("Wallet reissue succeeded unexpectedly".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_reissue_invalid_amount() -> Result<()> {
        let safe = new_safe_instance().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        match safe
            .wallet_reissue(&wallet_xorurl, "0", None, DbcReason::none())
            .await
        {
            Err(Error::InvalidAmount(msg)) => {
                assert_eq!(
                    msg,
                    "Output amount to reissue needs to be larger than zero (0)."
                );
                Ok(())
            }
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(_) => Err(anyhow!("Wallet reissue succeeded unexpectedly".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_reissue_with_non_compatible_content() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc, None)
            .await?;

        // We insert an entry (to its underlying data type, i.e. the Multimap) which is
        // not a valid serialised DBC, thus making part of its content incompatible/corrupted.
        let corrupted_dbc_xorurl = safe.store_bytes(Bytes::from_static(b"bla"), None).await?;
        let entry = (b"corrupted-dbc".to_vec(), corrupted_dbc_xorurl.into_bytes());
        safe.multimap_insert(&wallet_xorurl, entry, BTreeSet::default())
            .await?;

        // Now check we can still reissue from the wallet and the corrupted entry is ignored
        let _ = safe
            .wallet_reissue(&wallet_xorurl, "0.4", None, DbcReason::none())
            .await?;
        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(
            current_balance,
            Token::from_nano(dbc_balance.as_nano() - 400_000_000 - FEE_PER_INPUT) // 1 input dbc = 1 fee
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_all_balance() -> Result<()> {
        let (safe, dbc, dbc_balance) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("my-first-dbc"), &dbc, None)
            .await?;

        // Now check that after reissuing with the total balance,
        // there is no change deposited in the wallet, i.e. wallet is empty with 0 balance
        let _ = safe
            .wallet_reissue(
                &wallet_xorurl,
                &Token::from_nano(dbc_balance.as_nano() - FEE_PER_INPUT).to_string(), // send all, leave enough to pay the fee amount
                None,
                DbcReason::none(),
            )
            .await?;

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, Token::zero());

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;
        assert!(wallet_balances.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_reissued_dbc() -> Result<()> {
        let (safe, dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet1_xorurl = safe.wallet_create().await?;
        let wallet2_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet1_xorurl, Some("deposited-dbc"), &dbc, None)
            .await?;

        let output_dbc = safe
            .wallet_reissue(&wallet1_xorurl, "0.25", None, DbcReason::none())
            .await?;

        safe.wallet_deposit(&wallet2_xorurl, Some("reissued-dbc"), &output_dbc, None)
            .await?;

        let balance = safe.wallet_balance(&wallet2_xorurl).await?;
        assert_eq!(balance, Token::from_nano(250_000_000));

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_deposit_dbc_verification_fails() -> Result<()> {
        let (safe, mut dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        // let's corrupt the pub key of the SpentProofs
        let random_pk = bls::SecretKey::random().public_key();
        dbc.inputs_spent_proofs = dbc
            .inputs_spent_proofs
            .into_iter()
            .map(|mut proof| {
                proof.spentbook_pub_key = random_pk;
                proof
            })
            .collect();

        match safe
            .wallet_deposit(&wallet_xorurl, Some("deposited-dbc"), &dbc, None)
            .await
        {
            Err(Error::DbcError(DbcError::InvalidSpentProofSignature(_public_key))) => Ok(()),
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(_) => Err(anyhow!("Wallet deposit succeeded unexpectedly".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_reissue_dbc_verification_fails() -> Result<()> {
        let (safe, mut dbc, _) = new_safe_instance_with_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        // let's corrupt the pub key of the SpentProofs
        let random_pk = bls::SecretKey::random().public_key();
        dbc.inputs_spent_proofs = dbc
            .inputs_spent_proofs
            .into_iter()
            .map(|mut proof| {
                proof.spentbook_pub_key = random_pk;
                proof
            })
            .collect();

        // We insert a corrupted DBC (which contains invalid spent proofs) directly in the wallet,
        // thus Elders won't sign the new spent proof shares when trying to reissue from it
        safe.insert_dbc_into_wallet(
            &SafeUrl::from_url(&wallet_xorurl)?,
            &dbc,
            "corrupted_dbc".to_string(),
        )
        .await?;

        // It shall detect no spent proofs for this TX, thus fail to reissue
        match safe
            .wallet_reissue(&wallet_xorurl, "0.1", None, DbcReason::none())
            .await
        {
            Err(Error::ClientError(ClientError::CmdError {
                source: ErrorMsg::InvalidOperation(msg),
                ..
            })) => {
                assert_eq!(
                    msg,
                    format!(
                        "Failed to perform operation: SpentbookError(\"Spent proof \
                        signature {random_pk:?} is invalid\")",
                    )
                );
                Ok(())
            }
            Err(err) => Err(anyhow!("Error returned is not the expected: {:?}", err)),
            Ok(_) => Err(anyhow!("Wallet deposit succeeded unexpectedly".to_string())),
        }
    }

    #[tokio::test]
    async fn test_wallet_is_dbc_spent() -> Result<()> {
        let safe = new_safe_instance().await?;

        // the api shall confirm the genesis DBC's public_key has been spent
        let is_genesis_spent = safe.is_dbc_spent(GENESIS_DBC.public_key()).await?;
        assert!(is_genesis_spent);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_dbc_is_unspent() -> Result<()> {
        let (safe, unspent_dbc, _) = new_safe_instance_with_dbc().await?;

        // confirm the DBC's public_key has not been spent yet
        let is_unspent_dbc_spent = safe.is_dbc_spent(unspent_dbc.public_key()).await?;
        assert!(!is_unspent_dbc_spent);

        Ok(())
    }

    #[tokio::test]
    async fn test_wallet_reissue_multiple_output_dbcs() -> Result<()> {
        let (safe, dbc1, dbc1_balance) = new_safe_instance_with_dbc().await?;
        let (dbc2, dbc2_balance) = get_next_bearer_dbc().await?;
        let wallet_xorurl = safe.wallet_create().await?;

        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-1"), &dbc1, None)
            .await?;
        safe.wallet_deposit(&wallet_xorurl, Some("deposited-dbc-2"), &dbc2, None)
            .await?;

        let change_plus_fees = 1000;
        let expected_change = change_plus_fees - (2 * FEE_PER_INPUT); // 2 dbc inputs = 2 fees

        let amount_to_reissue =
            Token::from_nano(dbc1_balance.as_nano() + dbc2_balance.as_nano() - change_plus_fees);
        // let's partition the total amount to reissue in a few amounts
        let output_amounts = vec![
            dbc1_balance.as_nano() - 700,
            dbc2_balance.as_nano() - 700,
            150,
            100,
            60,
            90,
        ];
        assert_eq!(
            amount_to_reissue.as_nano(),
            output_amounts.iter().sum::<u64>()
        );

        let outputs_owners = output_amounts
            .iter()
            .map(|amount| (Token::from_nano(*amount).to_string(), None))
            .collect();

        let output_dbcs = safe
            .wallet_reissue_many(&wallet_xorurl, outputs_owners, DbcReason::none())
            .await?;

        assert_eq!(
            output_dbcs.len(),
            output_amounts.len() + (2 * FEE_PER_INPUT) as usize
        );

        let mut num_fee_outputs = 0;
        for dbc in output_dbcs {
            if let Ok(balance) = dbc.amount_secrets_bearer() {
                assert!(output_amounts.contains(&balance.amount().as_nano()));
            } else {
                num_fee_outputs += 1;
            }
        }
        assert_eq!(num_fee_outputs, 2 * FEE_PER_INPUT);

        let current_balance = safe.wallet_balance(&wallet_xorurl).await?;
        assert_eq!(current_balance, Token::from_nano(expected_change));

        let wallet_balances = safe.wallet_get(&wallet_xorurl).await?;

        assert_eq!(wallet_balances.len(), 1);

        let (_, (change_dbc_read, _)) = wallet_balances
            .iter()
            .next()
            .ok_or_else(|| anyhow!("Couldn't read change DBC from fetched wallet"))?;
        let change = change_dbc_read
            .amount_secrets_bearer()
            .map_err(|err| anyhow!("Couldn't read balance from change DBC fetched: {:?}", err))?;
        assert_eq!(change.amount(), Token::from_nano(expected_change));

        Ok(())
    }
}
