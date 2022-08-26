// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

//! Cryptographic primitives.

pub use ed25519_dalek::{Keypair, PublicKey, Signature, Verifier};

use ed25519_dalek::ExpandedSecretKey;
use std::ops::RangeInclusive;
use xor_name::{XorName, XOR_NAME_LEN};

/// SHA3-256 hash digest.
pub type Digest256 = [u8; 32];

pub fn sign(msg: &[u8], keypair: &Keypair) -> Signature {
    let expanded_secret_key = ExpandedSecretKey::from(&keypair.secret);
    expanded_secret_key.sign(msg, &keypair.public)
}

pub fn pub_key(name: &XorName) -> Result<PublicKey, ed25519_dalek::SignatureError> {
    PublicKey::from_bytes(&name.0)
}

pub fn name(public_key: &PublicKey) -> XorName {
    XorName(public_key.to_bytes())
}

/// Construct a random `XorName` whose last byte represents the targeted age.
pub fn gen_name_with_age(age: u8) -> XorName {
    loop {
        let name: XorName = xor_name::rand::random();
        if age == name[XOR_NAME_LEN - 1] {
            return name;
        }
    }
}

/// Construct a `Keypair` whose name is in the interval [start, end] (both endpoints inclusive).
/// And the last byte equals to the targeted age.
pub fn gen_keypair(range: &RangeInclusive<XorName>, age: u8) -> Keypair {
    let mut rng = rand_07::thread_rng();

    loop {
        let keypair = Keypair::generate(&mut rng);
        let new_name = XorName::from(crate::types::PublicKey::Ed25519(keypair.public));
        if range.contains(&new_name) && age == new_name[XOR_NAME_LEN - 1] {
            return keypair;
        }
    }
}

#[cfg(feature = "proptest")]
#[allow(clippy::unwrap_used)]
pub mod proptesting {

    pub use ed25519_dalek::{Keypair, PublicKey, Signature, Verifier};

    // we're in test feat territory here so we dont need to pull proptest
    // into the main crate deps
    use ed25519_dalek::{SecretKey, SECRET_KEY_LENGTH};
    use proptest::prelude::*;

    pub fn arbitrary_keypair() -> impl Strategy<Value = Keypair> {
        any::<[u8; SECRET_KEY_LENGTH]>().prop_map(|bytes| {
            // OK to unwrap because `from_bytes` returns error only if the input slice has incorrect
            // length. But here we only generate arrays of size `SECRET_KEY_LENGTH` which is the
            // correct one.
            // D.I. Letting this go, not for above reason but proptest uses unwrap due to
            // its structure.
            // nosemgrep
            let secret = SecretKey::from_bytes(&bytes[..]).unwrap();
            let public = PublicKey::from(&secret);

            Keypair { secret, public }
        })
    }
}
