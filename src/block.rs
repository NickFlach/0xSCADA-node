// PROTOCOL NOTE: Field ordering is part of the wire format. Do not reorder fields.

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::crypto::{self, PublicKey, Signature};
use crate::transaction::Transaction;

/// Maximum transactions per block.
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 1000;
/// Maximum attestations per block.
pub const MAX_ATTESTATIONS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u8,
    pub height: u64,
    pub timestamp: u64,
    pub prev_hash: [u8; 32],
    pub tx_root: [u8; 32],
    pub state_root: [u8; 32],
    pub proposer: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    pub validator: [u8; 32],
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    #[serde(with = "BigArray")]
    pub header_signature: [u8; 64],
    pub transactions: Vec<Transaction>,
    pub attestations: Vec<Attestation>,
}

impl BlockHeader {
    // SAFETY: CBOR serialization of fixed-size primitives cannot fail (no IO).
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).unwrap();
        buf
    }

    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.serialize()).as_bytes()
    }
}

impl Attestation {
    /// Verify this attestation's signature against the given block hash.
    pub fn verify(&self, block_hash: &[u8; 32]) -> bool {
        let pk = PublicKey(self.validator);
        let sig = Signature(self.signature);
        crypto::verify(&pk, block_hash, &sig)
    }
}

impl Block {
    pub fn hash(&self) -> [u8; 32] {
        self.header.hash()
    }

    // SAFETY: see BlockHeader::serialize()
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).unwrap();
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(data)
    }

    /// Verify the proposer's signature over the block header.
    pub fn verify_proposer_signature(&self) -> bool {
        let pk = PublicKey(self.header.proposer);
        let sig = Signature(self.header_signature);
        crypto::verify(&pk, &self.header.serialize(), &sig)
    }

    /// Validate that block dimensions are within protocol limits.
    pub fn validate_size(&self) -> Result<(), String> {
        if self.transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
            return Err(format!(
                "too many transactions: {} > {}",
                self.transactions.len(),
                MAX_TRANSACTIONS_PER_BLOCK
            ));
        }
        if self.attestations.len() > MAX_ATTESTATIONS {
            return Err(format!(
                "too many attestations: {} > {}",
                self.attestations.len(),
                MAX_ATTESTATIONS
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::NodeKeypair;
    use crate::transaction::TransactionBody;

    fn sample_header(height: u64) -> BlockHeader {
        BlockHeader {
            version: 1,
            height,
            timestamp: 1_700_000_000_000,
            prev_hash: [0u8; 32],
            tx_root: [1u8; 32],
            state_root: [2u8; 32],
            proposer: [3u8; 32],
        }
    }

    fn sample_block() -> Block {
        let body = TransactionBody::AnchorBatch {
            merkle_root: [10u8; 32],
            event_count: 5,
            site_id: "site-1".to_string(),
            metadata: vec![0x01],
        };
        let tx = Transaction {
            hash: body.hash(),
            body,
            signature: [0xAA; 64],
            signer: [0xBB; 32],
        };
        Block {
            header: sample_header(1),
            header_signature: [0xCC; 64],
            transactions: vec![tx],
            attestations: vec![Attestation {
                validator: [0xDD; 32],
                signature: [0xEE; 64],
            }],
        }
    }

    #[test]
    fn test_block_header_serialize_roundtrip() {
        let h = sample_header(42);
        let bytes = h.serialize();
        let h2: BlockHeader = ciborium::from_reader(&bytes[..]).unwrap();
        assert_eq!(h.version, h2.version);
        assert_eq!(h.height, h2.height);
        assert_eq!(h.timestamp, h2.timestamp);
        assert_eq!(h.prev_hash, h2.prev_hash);
        assert_eq!(h.tx_root, h2.tx_root);
        assert_eq!(h.state_root, h2.state_root);
        assert_eq!(h.proposer, h2.proposer);
    }

    #[test]
    fn test_block_hash_determinism() {
        let h = sample_header(1);
        assert_eq!(h.hash(), h.hash());
        let h2 = sample_header(1);
        assert_eq!(h.hash(), h2.hash());
    }

    #[test]
    fn test_block_hash_changes_with_content() {
        let h1 = sample_header(1);
        let h2 = sample_header(2);
        assert_ne!(h1.hash(), h2.hash());
    }

    #[test]
    fn test_full_block_serialize_roundtrip() {
        let block = sample_block();
        let bytes = block.serialize();
        let block2 = Block::deserialize(&bytes).unwrap();
        assert_eq!(block.header.height, block2.header.height);
        assert_eq!(block.header_signature, block2.header_signature);
        assert_eq!(block.transactions.len(), block2.transactions.len());
        assert_eq!(block.transactions[0].hash, block2.transactions[0].hash);
        assert_eq!(block.attestations.len(), block2.attestations.len());
        assert_eq!(block.attestations[0].validator, block2.attestations[0].validator);
        assert_eq!(block.hash(), block2.hash());
    }

    #[test]
    fn test_block_signature_verification() {
        let kp = NodeKeypair::generate();
        let header = BlockHeader {
            version: 1,
            height: 1,
            timestamp: 1_700_000_000_000,
            prev_hash: [0u8; 32],
            tx_root: [0u8; 32],
            state_root: [0u8; 32],
            proposer: kp.public_key().0,
        };
        let sig = kp.sign(&header.serialize());
        let block = Block {
            header,
            header_signature: sig.into(),
            transactions: vec![],
            attestations: vec![],
        };
        assert!(block.verify_proposer_signature());

        // Tamper with signature
        let mut bad_block = block.clone();
        bad_block.header_signature = [0xAA; 64];
        assert!(!bad_block.verify_proposer_signature());
    }

    #[test]
    fn test_attestation_verification() {
        let kp = NodeKeypair::generate();
        let block_hash = [0x42u8; 32];
        let sig = kp.sign(&block_hash);
        let att = Attestation {
            validator: kp.public_key().0,
            signature: sig.into(),
        };
        assert!(att.verify(&block_hash));
        assert!(!att.verify(&[0xFF; 32])); // wrong hash
    }

    #[test]
    fn test_block_validate_size() {
        let block = sample_block();
        assert!(block.validate_size().is_ok());

        // Too many attestations
        let mut big = block.clone();
        big.attestations = (0..MAX_ATTESTATIONS + 1)
            .map(|_| Attestation { validator: [0u8; 32], signature: [0u8; 64] })
            .collect();
        assert!(big.validate_size().is_err());
    }
}
