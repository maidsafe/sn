// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::messaging::MessageId;
use crate::node::{
    network::Network,
    node_api::interaction::push_state,
    node_api::role::ElderRole,
    node_ops::NodeDuties,
    section_funds::{self, SectionFunds},
    transfers::get_replicas::replica_info,
    Error, Node, Result,
};
use crate::routing::{Prefix, XorName};
use log::debug;
use section_funds::{
    elder_signing::ElderSigning,
    reward_process::{OurSection, RewardProcess},
    reward_wallets::RewardWallets,
};
use sn_data_types::{NodeAge, PublicKey, Token};
use std::collections::{BTreeMap, BTreeSet};

impl Node {
    /// Called on split reported from routing layer.
    pub(crate) async fn begin_split_as_newbie(
        &mut self,
        our_key: PublicKey,
        our_prefix: Prefix,
    ) -> Result<()> {
        let section_key = self.network_api.section_public_key().await?;
        if our_key != section_key {
            return Err(Error::Logic(format!(
                "Some failure.. our_key: {}, section_key: {}",
                our_key, section_key
            )));
        }

        self.level_up().await?;

        let section = OurSection {
            our_prefix,
            our_key,
        };

        let elder = self.role.as_elder_mut()?;

        let process =
            RewardProcess::new(section, ElderSigning::new(self.network_api.clone()).await?);

        let wallets = RewardWallets::new(BTreeMap::<XorName, (NodeAge, PublicKey)>::new());

        *elder.section_funds.write().await = SectionFunds::Churning { process, wallets };

        Ok(())
    }

    /// Called on split reported from routing layer.
    pub(crate) async fn begin_split_as_oldie(
        elder: &ElderRole,
        network_api: &Network,
        our_prefix: Prefix,
        our_key: PublicKey,
        sibling_key: PublicKey,
        our_new_elders: BTreeSet<XorName>,
        their_new_elders: BTreeSet<XorName>,
    ) -> Result<NodeDuties> {
        // get payments before updating replica info
        let payments = elder.transfers.read().await.payments().await?;

        let info = replica_info(network_api).await?;
        elder.transfers.write().await.update_replica_info(info);

        let wallets = match &*elder.section_funds.read().await {
            SectionFunds::KeepingNodeWallets(wallets) | SectionFunds::Churning { wallets, .. } => {
                wallets.clone()
            }
        };

        let sibling_prefix = our_prefix.sibling();
        let mut ops = vec![];

        if payments > Token::zero() {
            let section_managed = elder.transfers.read().await.managed_amount().await?;

            // payments made since last churn
            debug!("Payments: {}", payments);
            // total amount in wallets
            debug!("Managed amount: {}", section_managed);

            // generate reward and minting proposal
            let mut process = RewardProcess::new(
                OurSection {
                    our_prefix,
                    our_key,
                },
                ElderSigning::new(network_api.clone()).await?,
            );

            ops.push(
                process
                    .reward_and_mint(payments, section_managed, wallets.node_wallets())
                    .await?,
            );

            *elder.section_funds.write().await = SectionFunds::Churning {
                process,
                wallets: wallets.clone(),
            };
        } else {
            debug!("Not paying out rewards, as no payments have been received since last split.");
        }

        // replicate state to our new elders
        let msg_id = MessageId::combine(&[our_prefix.name().0, XorName::from(our_key).0]);
        ops.push(push_state(elder, our_prefix, msg_id, our_new_elders).await?);

        // replicate state to our neighbour's new elders
        let msg_id = MessageId::combine(&[sibling_prefix.name().0, XorName::from(sibling_key).0]);
        ops.push(push_state(elder, sibling_prefix, msg_id, their_new_elders).await?);

        let our_adults = network_api.our_adults().await;
        // drop metadata state
        elder
            .meta_data
            .write()
            .await
            .retain_members_only(our_adults)
            .await?;

        // drop transfers state
        elder
            .transfers
            .write()
            .await
            .keep_keys_of(our_prefix)
            .await?;

        // drop reward wallets state
        elder.section_funds.read().await.keep_wallets_of(our_prefix);

        Ok(ops)
    }
}
