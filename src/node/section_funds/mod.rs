// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

pub mod elder_signing;
mod reward_calc;
pub mod reward_process;
pub mod reward_stage;
pub mod reward_wallets;

use self::{reward_process::RewardProcess, reward_wallets::RewardWallets};
use crate::node::{Error, Result};
use crate::routing::{Prefix, XorName};
use crate::types::{CreditAgreementProof, CreditId, NodeAge, PublicKey, Token};
use dashmap::DashMap;
use log::info;
use std::collections::BTreeMap;

/// The management of section funds,
/// via the usage of a distributed AT2 Actor.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum SectionFunds {
    KeepingNodeWallets(RewardWallets),
    Churning {
        process: RewardProcess,
        wallets: RewardWallets,
    },
}

impl SectionFunds {
    pub fn as_churning_mut(&mut self) -> Result<(&mut RewardProcess, &mut RewardWallets)> {
        match self {
            Self::Churning { process, wallets } => Ok((process, wallets)),
            _ => Err(Error::NotChurningFunds),
        }
    }

    /// Returns registered wallet key of a node.
    pub fn get_node_wallet(&self, node_name: &XorName) -> Option<PublicKey> {
        match &self {
            Self::Churning { wallets, .. } | Self::KeepingNodeWallets(wallets) => {
                let (_, key) = wallets.get(node_name)?;
                Some(key)
            }
        }
    }

    /// Returns node wallet keys of registered nodes.
    pub fn node_wallets(&self) -> BTreeMap<XorName, (NodeAge, PublicKey)> {
        match &self {
            Self::Churning { wallets, .. } | Self::KeepingNodeWallets(wallets) => {
                wallets.node_wallets()
            }
        }
    }

    /// Nodes register/updates wallets for future reward payouts.
    pub fn set_node_wallet(&self, node_id: XorName, wallet: PublicKey, age: u8) {
        match &self {
            Self::Churning { wallets, .. } | Self::KeepingNodeWallets(wallets) => {
                wallets.set_node_wallet(node_id, age, wallet)
            }
        }
    }

    /// When the section becomes aware that a node has left,
    /// its reward key is removed.
    pub fn remove_node_wallet(&self, node_name: XorName) {
        info!("Removing node wallet");
        match &self {
            Self::Churning { wallets, .. } | Self::KeepingNodeWallets(wallets) => {
                wallets.remove_wallet(node_name)
            }
        }
    }

    /// When the section becomes aware that a node has left,
    /// its reward key is removed.
    pub fn keep_wallets_of(&self, prefix: Prefix) {
        match &self {
            Self::Churning { wallets, .. } | Self::KeepingNodeWallets(wallets) => {
                wallets.keep_wallets_of(prefix)
            }
        }
    }
}

type Payments = DashMap<CreditId, CreditAgreementProof>;
type Rewards = BTreeMap<CreditId, CreditAgreementProof>;

pub trait Credits {
    fn sum(&self) -> Token;
}

impl Credits for Payments {
    fn sum(&self) -> Token {
        Token::from_nano(self.iter().map(|c| (*c).amount().as_nano()).sum())
    }
}

impl Credits for Rewards {
    fn sum(&self) -> Token {
        Token::from_nano(self.iter().map(|(_, c)| c.amount().as_nano()).sum())
    }
}
