use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::transaction::Transaction;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u8,
    pub height: u64,
    pub timestamp: u64,
    pub prev_hash: [u8; 32],
    pub tx_root: [u8; 32],
    pub state_root: [u8; 32],
    pub proposer: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attestation {
    pub validator: [u8; 32],
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    #[serde(with = "BigArray")]
    pub header_signature: [u8; 64],
    pub transactions: Vec<Transaction>,
    pub attestations: Vec<Attestation>,
}

impl BlockHeader {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).expect("CBOR serialize failed");
        buf
    }

    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.serialize()).as_bytes()
    }
}

impl Block {
    pub fn hash(&self) -> [u8; 32] {
        self.header.hash()
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).expect("CBOR serialize failed");
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
