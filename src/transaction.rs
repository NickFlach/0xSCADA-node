use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: [u8; 32],
    pub body: TransactionBody,
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
    pub signer: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ValidatorAction {
    Add,
    Remove,
}

impl Transaction {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).expect("CBOR serialize failed");
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader(data)
    }
}

impl TransactionBody {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).expect("CBOR serialize failed");
        buf
    }

    pub fn hash(&self) -> [u8; 32] {
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
}
