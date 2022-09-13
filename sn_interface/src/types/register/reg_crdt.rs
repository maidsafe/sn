// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::metadata::Entry;
use super::{
    super::{
        RegisterAddress, Signature, {Error, Result},
    },
    User,
};
use crdts::{
    merkle_reg::{MerkleReg, Node},
    CmRDT,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{self, Debug, Display, Formatter, Result as FmtResult},
    hash::Hash,
};

/// Hash of the register entry. Logging as the same format of `XorName`.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryHash(pub crdts::merkle_reg::Hash);

impl Debug for EntryHash {
    fn fmt(&self, formatter: &mut Formatter) -> FmtResult {
        write!(formatter, "{}", self)
    }
}

impl Display for EntryHash {
    fn fmt(&self, formatter: &mut Formatter) -> FmtResult {
        write!(
            formatter,
            "{:02x}{:02x}{:02x}..",
            self.0[0], self.0[1], self.0[2]
        )
    }
}

/// CRDT Data operation applicable to other Register replica.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrdtOperation<T> {
    /// Address of a Register object on the network.
    pub address: RegisterAddress,
    /// The data operation to apply.
    pub crdt_op: Node<T>,
    /// The PublicKey of the entity that generated the operation
    pub source: User,
    /// The signature of source on the crdt_top, required to apply the op
    pub signature: Option<Signature>,
}

/// Register data type as a CRDT with Access Control
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd)]
pub(crate) struct RegisterCrdt {
    /// Address on the network of this piece of data
    address: RegisterAddress,
    /// CRDT to store the actual data, i.e. the items of the Register.
    data: MerkleReg<Entry>,
}

impl Display for RegisterCrdt {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "(")?;
        for (i, entry) in self.data.read().values().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "<{:?}>", entry,)?;
        }
        write!(f, ")")
    }
}

impl RegisterCrdt {
    /// Constructs a new '`RegisterCrdt`'.
    pub(crate) fn new(address: RegisterAddress) -> Self {
        Self {
            address,
            data: MerkleReg::new(),
        }
    }

    /// Returns the address.
    pub(crate) fn address(&self) -> &RegisterAddress {
        &self.address
    }

    /// Returns total number of items in the register.
    pub(crate) fn size(&self) -> u64 {
        (self.data.num_nodes() + self.data.num_orphans()) as u64
    }

    /// Write a new entry to the `RegisterCrdt`, returning the hash
    /// of the entry and the CRDT operation without a signature
    pub(crate) fn write(
        &mut self,
        entry: Entry,
        children: BTreeSet<EntryHash>,
        source: User,
    ) -> Result<(EntryHash, CrdtOperation<Entry>)> {
        let address = *self.address();

        let children_array: BTreeSet<[u8; 32]> = children.iter().map(|itr| itr.0).collect();
        let crdt_op = self.data.write(entry, children_array);
        self.data.apply(crdt_op.clone());
        let hash = crdt_op.hash();

        // We return the operation as it may need to be broadcasted to other replicas
        let op = CrdtOperation {
            address,
            crdt_op,
            source,
            signature: None,
        };

        Ok((EntryHash(hash), op))
    }

    /// Apply a remote data CRDT operation to this replica of the `RegisterCrdt`.
    pub(crate) fn apply_op(&mut self, op: CrdtOperation<Entry>) -> Result<()> {
        // Let's first check the op is validly signed.
        // Note: Perms and valid sig for the op are checked at the upper Register layer.

        // Check the targetting address is correct
        if self.address != op.address {
            return Err(Error::CrdtWrongAddress(op.address));
        }

        // Apply the CRDT operation to the Register
        self.data.apply(op.crdt_op);

        Ok(())
    }

    /// Get the entry corresponding to the provided `hash` if it exists.
    pub(crate) fn get(&self, hash: EntryHash) -> Option<&Entry> {
        self.data.node(hash.0).map(|node| &node.value)
    }

    /// Read current entries (multiple entries occur on concurrent writes).
    pub(crate) fn read(&self) -> BTreeSet<(EntryHash, Entry)> {
        self.data
            .read()
            .hashes_and_nodes()
            .map(|(hash, node)| (EntryHash(hash), node.value.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use xor_name::XorName;

    #[test]
    fn creating_entry_hash() -> Result<()> {
        let mut rng = rand::thread_rng();
        let address_1 = RegisterAddress {
            name: XorName::random(&mut rng),
            tag: 0,
        };
        let address_2 = RegisterAddress {
            name: XorName::random(&mut rng),
            tag: 0,
        };

        let mut crdt_1 = RegisterCrdt::new(address_1);
        let mut crdt_2 = RegisterCrdt::new(address_2);
        let mut parents = BTreeSet::new();

        let entry_1 = vec![0x1, 0x1];
        // Different RegisterCrdt shall create same hashes for the same entry from root
        let (entry_hash_1, _) = crdt_1.write(entry_1.clone(), parents.clone(), User::Anyone)?;
        let (entry_hash_2, _) = crdt_2.write(entry_1, parents.clone(), User::Anyone)?;
        assert!(entry_hash_1 == entry_hash_2);

        let entry_2 = vec![0x2, 0x2];
        // RegisterCrdt shall create differnt hashes for different entries from root
        let (entry_hash_1_2, _) = crdt_1.write(entry_2, parents.clone(), User::Anyone)?;
        assert!(entry_hash_1 != entry_hash_1_2);

        let entry_3 = vec![0x3, 0x3];
        // Different RegisterCrdt shall create same hashes for the same entry from same parents
        let _ = parents.insert(entry_hash_1);
        let (entry_hash_1_3, _) = crdt_1.write(entry_3.clone(), parents.clone(), User::Anyone)?;
        let (entry_hash_2_3, _) = crdt_1.write(entry_3, parents, User::Anyone)?;
        assert!(entry_hash_1_3 == entry_hash_2_3);

        Ok(())
    }
}
