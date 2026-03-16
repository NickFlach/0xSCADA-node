use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::block::*;
use crate::crypto::*;
use crate::merkle::SparseMerkleTree;
use crate::storage::*;
use crate::transaction::*;

#[derive(Debug, thiserror::Error)]
pub enum BlockManagerError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("invalid block: {0}")]
    InvalidBlock(String),
    #[error("invalid genesis: {0}")]
    InvalidGenesis(String),
}

pub struct BlockManager {
    storage: Storage,
    state_tree: SparseMerkleTree,
    validator_set: Vec<[u8; 32]>,
    latest_height: u64,
    latest_hash: [u8; 32],
}

impl BlockManager {
    pub fn new(storage_path: &Path, validator_set: Vec<[u8; 32]>) -> Result<Self, BlockManagerError> {
        let storage = Storage::open(storage_path)?;
        let mut mgr = BlockManager {
            storage,
            state_tree: SparseMerkleTree::new(),
            validator_set,
            latest_height: 0,
            latest_hash: [0u8; 32],
        };

        // Load existing state if any
        if let Some(height) = mgr.storage.get_latest_height()? {
            let block = mgr.storage.get_block_by_height(height)?;
            mgr.latest_height = height;
            mgr.latest_hash = block.hash;
        }

        Ok(mgr)
    }

    pub fn create_genesis(&mut self, keypair: &NodeKeypair) -> Result<Block, BlockManagerError> {
        if self.latest_height > 0 || self.latest_hash != [0u8; 32] {
            return Err(BlockManagerError::InvalidGenesis(
                "genesis already exists".to_string(),
            ));
        }

        let header = BlockHeader {
            version: 1,
            height: 0,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            prev_hash: [0u8; 32],
            tx_root: [0u8; 32],
            state_root: self.state_tree.root(),
            proposer: keypair.public_key().0,
        };

        let header_bytes = header.serialize();
        let sig = keypair.sign(&header_bytes);

        let block = Block {
            header,
            header_signature: sig.0,
            transactions: vec![],
            attestations: vec![],
        };

        self.apply_block(&block)?;
        Ok(block)
    }

    pub fn validate_block(&self, block: &Block) -> Result<(), BlockManagerError> {
        let is_genesis = block.header.height == 0 && block.header.prev_hash == [0u8; 32];

        if is_genesis {
            // Genesis: just verify proposer signature
            let header_bytes = block.header.serialize();
            let pk = PublicKey(block.header.proposer);
            let sig = Signature(block.header_signature);
            if !verify(&pk, &header_bytes, &sig) {
                return Err(BlockManagerError::InvalidBlock(
                    "invalid proposer signature".to_string(),
                ));
            }
            return Ok(());
        }

        // Height check
        if block.header.height != self.latest_height + 1 {
            return Err(BlockManagerError::InvalidBlock(format!(
                "expected height {}, got {}",
                self.latest_height + 1,
                block.header.height
            )));
        }

        // Prev hash check
        if block.header.prev_hash != self.latest_hash {
            return Err(BlockManagerError::InvalidBlock(
                "prev_hash mismatch".to_string(),
            ));
        }

        // Proposer signature
        let header_bytes = block.header.serialize();
        let pk = PublicKey(block.header.proposer);
        let sig = Signature(block.header_signature);
        if !verify(&pk, &header_bytes, &sig) {
            return Err(BlockManagerError::InvalidBlock(
                "invalid proposer signature".to_string(),
            ));
        }

        // Attestation threshold
        let threshold = self.attestation_threshold();
        if block.attestations.len() < threshold {
            return Err(BlockManagerError::InvalidBlock(format!(
                "insufficient attestations: got {}, need {}",
                block.attestations.len(),
                threshold
            )));
        }

        // Verify all attestation signatures
        let block_hash = block.header.hash();
        for att in &block.attestations {
            let pk = PublicKey(att.validator);
            let sig = Signature(att.signature);
            if !verify(&pk, &block_hash, &sig) {
                return Err(BlockManagerError::InvalidBlock(
                    "invalid attestation signature".to_string(),
                ));
            }
        }

        Ok(())
    }

    pub fn apply_block(&mut self, block: &Block) -> Result<(), BlockManagerError> {
        // Execute transactions
        for tx in &block.transactions {
            match &tx.body {
                TransactionBody::AnchorBatch {
                    merkle_root,
                    event_count,
                    site_id,
                    metadata: _,
                } => {
                    // value = CBOR(site_id, event_count, timestamp)
                    let val_tuple = (site_id.clone(), *event_count, block.header.timestamp);
                    let mut buf = Vec::new();
                    ciborium::into_writer(&val_tuple, &mut buf).unwrap();
                    self.state_tree.insert(merkle_root, &buf);
                }
                TransactionBody::ValidatorUpdate {
                    action,
                    pubkey,
                    proof: _,
                } => match action {
                    ValidatorAction::Add => {
                        if !self.validator_set.contains(pubkey) {
                            self.validator_set.push(*pubkey);
                        }
                    }
                    ValidatorAction::Remove => {
                        self.validator_set.retain(|k| k != pubkey);
                    }
                },
                TransactionBody::Extension {
                    module_id,
                    payload,
                } => {
                    self.state_tree.insert(module_id, payload);
                }
            }
        }

        // Convert to StorableBlock and store
        let storable = StorableBlock {
            hash: block.header.hash(),
            height: block.header.height,
            prev_hash: block.header.prev_hash,
            timestamp: block.header.timestamp,
            tx_hashes: block.transactions.iter().map(|tx| tx.hash).collect(),
            data: block.serialize(),
        };

        self.storage.store_block(&storable)?;
        self.storage
            .store_state_root(block.header.height, &self.state_tree.root())?;

        self.latest_height = block.header.height;
        self.latest_hash = block.header.hash();

        Ok(())
    }

    pub fn build_block(
        &self,
        transactions: Vec<Transaction>,
        keypair: &NodeKeypair,
    ) -> Result<Block, BlockManagerError> {
        // Compute tx_root from transaction hashes
        let tx_root = if transactions.is_empty() {
            [0u8; 32]
        } else {
            let combined: Vec<u8> = transactions.iter().flat_map(|tx| tx.hash).collect();
            hash(&combined)
        };

        let header = BlockHeader {
            version: 1,
            height: self.latest_height + 1,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            prev_hash: self.latest_hash,
            tx_root,
            state_root: self.state_tree.root(),
            proposer: keypair.public_key().0,
        };

        let header_bytes = header.serialize();
        let sig = keypair.sign(&header_bytes);

        Ok(Block {
            header,
            header_signature: sig.0,
            transactions,
            attestations: vec![],
        })
    }

    pub fn attest_block(
        &self,
        block: &Block,
        keypair: &NodeKeypair,
    ) -> Result<Attestation, BlockManagerError> {
        let block_hash = block.header.hash();
        let sig = keypair.sign(&block_hash);
        Ok(Attestation {
            validator: keypair.public_key().0,
            signature: sig.0,
        })
    }

    fn attestation_threshold(&self) -> usize {
        let n = self.validator_set.len();
        if n == 0 {
            return 1;
        }
        let t = (n + 1) / 2 + 1; // ceil(n/2) + 1
        t.min(n) // can't require more attestations than validators
    }

    pub fn latest_height(&self) -> u64 {
        self.latest_height
    }

    pub fn latest_hash(&self) -> [u8; 32] {
        self.latest_hash
    }

    pub fn validator_set(&self) -> &[[u8; 32]] {
        &self.validator_set
    }

    pub fn state_tree(&self) -> &SparseMerkleTree {
        &self.state_tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, NodeKeypair) {
        let dir = tempfile::tempdir().unwrap();
        let kp = NodeKeypair::generate();
        (dir, kp)
    }

    fn make_manager(dir: &Path, kp: &NodeKeypair) -> BlockManager {
        let validators = vec![kp.public_key().0];
        BlockManager::new(dir, validators).unwrap()
    }

    fn make_anchor_tx(kp: &NodeKeypair) -> Transaction {
        let body = TransactionBody::AnchorBatch {
            merkle_root: [0x42; 32],
            event_count: 10,
            site_id: "site-1".to_string(),
            metadata: vec![],
        };
        let body_bytes = body.serialize();
        let sig = kp.sign(&body_bytes);
        Transaction {
            hash: body.hash(),
            body,
            signature: sig.0,
            signer: kp.public_key().0,
        }
    }

    #[test]
    fn test_create_genesis() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        let genesis = mgr.create_genesis(&kp).unwrap();
        assert_eq!(genesis.header.height, 0);
        assert_eq!(genesis.header.prev_hash, [0u8; 32]);
        assert_eq!(mgr.latest_height(), 0);
        // Stored in storage
        let stored = mgr.storage.get_block_by_height(0).unwrap();
        assert_eq!(stored.height, 0);
    }

    #[test]
    fn test_genesis_is_valid() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        let genesis = mgr.create_genesis(&kp).unwrap();
        // Validate genesis on a fresh manager
        let dir2 = tempfile::tempdir().unwrap();
        let mgr2 = make_manager(dir2.path(), &kp);
        mgr2.validate_block(&genesis).unwrap();
    }

    #[test]
    fn test_build_and_apply_block() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let tx = make_anchor_tx(&kp);
        let mut block = mgr.build_block(vec![tx], &kp).unwrap();
        assert_eq!(block.header.height, 1);

        // Add attestations to pass threshold
        let att = mgr.attest_block(&block, &kp).unwrap();
        block.attestations.push(att);

        mgr.validate_block(&block).unwrap();
        mgr.apply_block(&block).unwrap();
        assert_eq!(mgr.latest_height(), 1);
    }

    #[test]
    fn test_validate_wrong_height() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let mut block = mgr.build_block(vec![], &kp).unwrap();
        block.header.height = 5; // wrong
        // Re-sign with wrong height
        let sig = kp.sign(&block.header.serialize());
        block.header_signature = sig.0;

        let err = mgr.validate_block(&block).unwrap_err();
        assert!(err.to_string().contains("expected height"));
    }

    #[test]
    fn test_validate_wrong_prev_hash() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let mut block = mgr.build_block(vec![], &kp).unwrap();
        block.header.prev_hash = [0xFF; 32]; // wrong
        let sig = kp.sign(&block.header.serialize());
        block.header_signature = sig.0;

        let err = mgr.validate_block(&block).unwrap_err();
        assert!(err.to_string().contains("prev_hash"));
    }

    #[test]
    fn test_validate_bad_signature() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let mut block = mgr.build_block(vec![], &kp).unwrap();
        block.header_signature = [0xAA; 64]; // invalid

        let err = mgr.validate_block(&block).unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[test]
    fn test_validate_insufficient_attestations() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let block = mgr.build_block(vec![], &kp).unwrap();
        // No attestations added
        let err = mgr.validate_block(&block).unwrap_err();
        assert!(err.to_string().contains("insufficient attestations"));
    }

    #[test]
    fn test_attest_block() {
        let (dir, kp) = setup();
        let kp2 = NodeKeypair::generate();
        let validators = vec![kp.public_key().0, kp2.public_key().0];
        let mut mgr = BlockManager::new(dir.path(), validators).unwrap();
        mgr.create_genesis(&kp).unwrap();

        let mut block = mgr.build_block(vec![], &kp).unwrap();
        // threshold for 2 validators: ceil(2/2)+1 = 2
        let att1 = mgr.attest_block(&block, &kp).unwrap();
        let att2 = mgr.attest_block(&block, &kp2).unwrap();
        block.attestations.push(att1);
        block.attestations.push(att2);

        mgr.validate_block(&block).unwrap();
    }

    #[test]
    fn test_anchor_batch_updates_state() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let tx = make_anchor_tx(&kp);
        let merkle_root = [0x42; 32]; // matches make_anchor_tx
        let mut block = mgr.build_block(vec![tx], &kp).unwrap();
        let att = mgr.attest_block(&block, &kp).unwrap();
        block.attestations.push(att);

        mgr.apply_block(&block).unwrap();

        // State tree should contain the anchor batch entry
        let val = mgr.state_tree().get(&merkle_root);
        assert!(val.is_some());
    }

    #[test]
    fn test_validator_update_add() {
        let (dir, kp) = setup();
        let mut mgr = make_manager(dir.path(), &kp);
        mgr.create_genesis(&kp).unwrap();

        let new_validator = [0x99; 32];
        let body = TransactionBody::ValidatorUpdate {
            action: ValidatorAction::Add,
            pubkey: new_validator,
            proof: vec![],
        };
        let sig = kp.sign(&body.serialize());
        let tx = Transaction {
            hash: body.hash(),
            body,
            signature: sig.0,
            signer: kp.public_key().0,
        };

        let mut block = mgr.build_block(vec![tx], &kp).unwrap();
        let att = mgr.attest_block(&block, &kp).unwrap();
        block.attestations.push(att);

        mgr.apply_block(&block).unwrap();
        assert!(mgr.validator_set().contains(&new_validator));
        assert_eq!(mgr.validator_set().len(), 2);
    }
}
