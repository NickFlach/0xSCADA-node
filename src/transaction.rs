// PROTOCOL NOTE: Field ordering is part of the wire format. Do not reorder fields.

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::crypto::NodeKeypair;

/// Maximum metadata size in bytes (64 KB).
pub const MAX_METADATA_SIZE: usize = 64 * 1024;
/// Maximum site_id length in bytes.
pub const MAX_SITE_ID_LEN: usize = 256;
/// Maximum payload size in bytes (256 KB).
pub const MAX_PAYLOAD_SIZE: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: [u8; 32],
    pub body: TransactionBody,
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
    pub signer: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionBody {
    AnchorBatch {
        merkle_root: [u8; 32],
        event_count: u32,
        site_id: String,
        metadata: Vec<u8>,
    },
    ValidatorUpdate {
        action: ValidatorAction,
        pubkey: [u8; 32],
        proof: Vec<u8>,
    },
    Extension {
        module_id: [u8; 32],
        payload: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidatorAction {
    Add,
    Remove,
}

impl Transaction {
    /// Create a new transaction with computed hash and signature.
    pub fn new(body: TransactionBody, keypair: &NodeKeypair) -> Transaction {
        let hash = body.compute_hash();
        let sig = keypair.sign(&hash);
        Transaction {
            hash,
            body,
            signature: sig.into(),
            signer: keypair.public_key().into(),
        }
    }

    // SAFETY: CBOR serialization of fixed-size primitives and bounded vecs cannot fail
    // (no IO, no fallible allocations). Size limits are enforced via validate_size().
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).unwrap();
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(data)
    }

    /// Verify that the stored hash matches the body's computed hash.
    pub fn verify_hash(&self) -> bool {
        self.hash == self.body.compute_hash()
    }

    /// Validate that all variable-length fields are within protocol size limits.
    pub fn validate_size(&self) -> Result<(), String> {
        match &self.body {
            TransactionBody::AnchorBatch { site_id, metadata, .. } => {
                if site_id.len() > MAX_SITE_ID_LEN {
                    return Err(format!("site_id too long: {} > {}", site_id.len(), MAX_SITE_ID_LEN));
                }
                if metadata.len() > MAX_METADATA_SIZE {
                    return Err(format!("metadata too large: {} > {}", metadata.len(), MAX_METADATA_SIZE));
                }
            }
            TransactionBody::ValidatorUpdate { proof, .. } => {
                if proof.len() > MAX_PAYLOAD_SIZE {
                    return Err(format!("proof too large: {} > {}", proof.len(), MAX_PAYLOAD_SIZE));
                }
            }
            TransactionBody::Extension { payload, .. } => {
                if payload.len() > MAX_PAYLOAD_SIZE {
                    return Err(format!("payload too large: {} > {}", payload.len(), MAX_PAYLOAD_SIZE));
                }
            }
        }
        Ok(())
    }
}

impl TransactionBody {
    // SAFETY: see Transaction::serialize()
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).unwrap();
        buf
    }

    pub fn hash(&self) -> [u8; 32] {
        self.compute_hash()
    }

    /// Compute the blake3 hash of the CBOR-serialized body.
    pub fn compute_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.serialize()).as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_anchor_batch() -> TransactionBody {
        TransactionBody::AnchorBatch {
            merkle_root: [1u8; 32],
            event_count: 42,
            site_id: "site-alpha".to_string(),
            metadata: vec![0xCA, 0xFE],
        }
    }

    fn sample_validator_update() -> TransactionBody {
        TransactionBody::ValidatorUpdate {
            action: ValidatorAction::Add,
            pubkey: [2u8; 32],
            proof: vec![0xDE, 0xAD],
        }
    }

    fn sample_extension() -> TransactionBody {
        TransactionBody::Extension {
            module_id: [3u8; 32],
            payload: vec![0xBE, 0xEF],
        }
    }

    fn make_tx(body: TransactionBody) -> Transaction {
        Transaction {
            hash: body.hash(),
            body,
            signature: [0xAA; 64],
            signer: [0xBB; 32],
        }
    }

    #[test]
    fn test_transaction_serialize_roundtrip() {
        let tx = make_tx(sample_anchor_batch());
        let bytes = tx.serialize();
        let tx2 = Transaction::deserialize(&bytes).unwrap();
        assert_eq!(tx.hash, tx2.hash);
        assert_eq!(tx.signature, tx2.signature);
        assert_eq!(tx.signer, tx2.signer);
    }

    #[test]
    fn test_transaction_body_variants() {
        for body in [sample_anchor_batch(), sample_validator_update(), sample_extension()] {
            let serialized = body.serialize();
            let deserialized: TransactionBody = ciborium::from_reader(&serialized[..]).unwrap();
            let reserialized = deserialized.serialize();
            assert_eq!(serialized, reserialized);
        }
    }

    #[test]
    fn test_transaction_hash_integrity() {
        let body = sample_anchor_batch();
        let tx = make_tx(body);
        assert!(tx.verify_hash());

        // Tamper with the hash
        let mut bad_tx = tx.clone();
        bad_tx.hash = [0xFF; 32];
        assert!(!bad_tx.verify_hash());
    }

    #[test]
    fn test_transaction_new_constructor() {
        let kp = crate::crypto::NodeKeypair::generate();
        let body = sample_anchor_batch();
        let tx = Transaction::new(body.clone(), &kp);
        assert!(tx.verify_hash());
        assert_eq!(tx.hash, body.compute_hash());
        // Verify signature
        let pk = crate::crypto::PublicKey(tx.signer);
        let sig = crate::crypto::Signature(tx.signature);
        assert!(crate::crypto::verify(&pk, &tx.hash, &sig));
    }

    #[test]
    fn test_oversized_metadata_rejected() {
        let tx = make_tx(TransactionBody::AnchorBatch {
            merkle_root: [0u8; 32],
            event_count: 1,
            site_id: "x".to_string(),
            metadata: vec![0u8; MAX_METADATA_SIZE + 1],
        });
        assert!(tx.validate_size().is_err());
    }

    #[test]
    fn test_oversized_site_id_rejected() {
        let tx = make_tx(TransactionBody::AnchorBatch {
            merkle_root: [0u8; 32],
            event_count: 1,
            site_id: "x".repeat(MAX_SITE_ID_LEN + 1),
            metadata: vec![],
        });
        assert!(tx.validate_size().is_err());
    }

    #[test]
    fn test_valid_size_passes() {
        let tx = make_tx(sample_anchor_batch());
        assert!(tx.validate_size().is_ok());
    }
}
