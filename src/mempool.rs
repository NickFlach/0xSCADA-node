use crate::transaction::*;
use std::collections::HashMap;

pub const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Transaction mempool — holds pending transactions until included in a block
pub struct Mempool {
    transactions: HashMap<[u8; 32], Transaction>,
    order: Vec<[u8; 32]>,
}

impl Mempool {
    pub fn new() -> Self {
        Mempool {
            transactions: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Add a transaction to the mempool.
    /// Returns false if already exists, mempool is full, or validation fails.
    pub fn insert(&mut self, tx: Transaction) -> bool {
        if self.transactions.len() >= MAX_MEMPOOL_SIZE {
            return false;
        }
        if !tx.verify_hash() {
            return false;
        }
        if tx.validate_size().is_err() {
            return false;
        }
        if self.transactions.contains_key(&tx.hash) {
            return false;
        }
        let hash = tx.hash;
        self.order.push(hash);
        self.transactions.insert(hash, tx);
        true
    }

    /// Remove a transaction by hash
    pub fn remove(&mut self, hash: &[u8; 32]) -> Option<Transaction> {
        if let Some(tx) = self.transactions.remove(hash) {
            self.order.retain(|h| h != hash);
            Some(tx)
        } else {
            None
        }
    }

    /// Drain up to `max` transactions for block building, ordered by priority:
    /// ValidatorUpdate > AnchorBatch > Extension
    /// Within same priority: FIFO (insertion order)
    pub fn drain(&mut self, max: usize) -> Vec<Transaction> {
        // Build ordered list by priority then FIFO
        let mut result: Vec<[u8; 32]> = Vec::new();

        // Priority 0: ValidatorUpdate
        for h in &self.order {
            if let Some(tx) = self.transactions.get(h) {
                if matches!(tx.body, TransactionBody::ValidatorUpdate { .. }) {
                    result.push(*h);
                }
            }
        }
        // Priority 1: AnchorBatch
        for h in &self.order {
            if let Some(tx) = self.transactions.get(h) {
                if matches!(tx.body, TransactionBody::AnchorBatch { .. }) {
                    result.push(*h);
                }
            }
        }
        // Priority 2: Extension
        for h in &self.order {
            if let Some(tx) = self.transactions.get(h) {
                if matches!(tx.body, TransactionBody::Extension { .. }) {
                    result.push(*h);
                }
            }
        }

        result.truncate(max);

        let txs: Vec<Transaction> = result
            .iter()
            .map(|h| self.transactions.remove(h).unwrap())
            .collect();

        // Remove from order
        let drained: std::collections::HashSet<[u8; 32]> = result.into_iter().collect();
        self.order.retain(|h| !drained.contains(h));

        txs
    }

    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    pub fn contains(&self, hash: &[u8; 32]) -> bool {
        self.transactions.contains_key(hash)
    }

    /// Remove all transactions whose hashes are in the given set
    pub fn remove_committed(&mut self, tx_hashes: &[[u8; 32]]) {
        for hash in tx_hashes {
            self.transactions.remove(hash);
        }
        let set: std::collections::HashSet<[u8; 32]> = tx_hashes.iter().copied().collect();
        self.order.retain(|h| !set.contains(h));
    }

    pub fn clear(&mut self) {
        self.transactions.clear();
        self.order.clear();
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::NodeKeypair;

    fn make_tx(body: TransactionBody) -> Transaction {
        let kp = NodeKeypair::generate();
        Transaction::new(body, &kp)
    }

    fn anchor_body(id: u8) -> TransactionBody {
        TransactionBody::AnchorBatch {
            merkle_root: [id; 32],
            event_count: 1,
            site_id: format!("site-{}", id),
            metadata: vec![id],
        }
    }

    fn validator_body(id: u8) -> TransactionBody {
        TransactionBody::ValidatorUpdate {
            action: ValidatorAction::Add,
            pubkey: [id; 32],
            proof: vec![id],
        }
    }

    fn extension_body(id: u8) -> TransactionBody {
        TransactionBody::Extension {
            module_id: [id; 32],
            payload: vec![id],
        }
    }

    #[test]
    fn test_insert_and_len() {
        let mut pool = Mempool::new();
        assert!(pool.insert(make_tx(anchor_body(1))));
        assert!(pool.insert(make_tx(anchor_body(2))));
        assert!(pool.insert(make_tx(anchor_body(3))));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn test_insert_dedup() {
        let mut pool = Mempool::new();
        let tx = make_tx(anchor_body(1));
        assert!(pool.insert(tx.clone()));
        assert!(!pool.insert(tx));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_insert_rejects_invalid_hash() {
        let mut pool = Mempool::new();
        let mut tx = make_tx(anchor_body(1));
        tx.hash = [0xFF; 32]; // corrupt hash
        assert!(!pool.insert(tx));
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_insert_rejects_oversized() {
        let mut pool = Mempool::new();
        let body = TransactionBody::Extension {
            module_id: [1; 32],
            payload: vec![0u8; MAX_PAYLOAD_SIZE + 1],
        };
        let tx = make_tx(body);
        assert!(!pool.insert(tx));
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_mempool_full() {
        let mut pool = Mempool::new();
        for i in 0..MAX_MEMPOOL_SIZE {
            let body = TransactionBody::Extension {
                module_id: [0; 32],
                payload: (i as u32).to_le_bytes().to_vec(),
            };
            assert!(pool.insert(make_tx(body)));
        }
        assert_eq!(pool.len(), MAX_MEMPOOL_SIZE);
        assert!(!pool.insert(make_tx(extension_body(99))));
    }

    #[test]
    fn test_drain_priority_order() {
        let mut pool = Mempool::new();
        // Insert in mixed order
        let ext = make_tx(extension_body(1));
        let anchor = make_tx(anchor_body(1));
        let val = make_tx(validator_body(1));
        pool.insert(ext.clone());
        pool.insert(anchor.clone());
        pool.insert(val.clone());

        let drained = pool.drain(10);
        assert_eq!(drained.len(), 3);
        assert!(matches!(drained[0].body, TransactionBody::ValidatorUpdate { .. }));
        assert!(matches!(drained[1].body, TransactionBody::AnchorBatch { .. }));
        assert!(matches!(drained[2].body, TransactionBody::Extension { .. }));
        assert!(pool.is_empty());
    }

    #[test]
    fn test_drain_fifo_within_priority() {
        let mut pool = Mempool::new();
        let tx1 = make_tx(anchor_body(1));
        let tx2 = make_tx(anchor_body(2));
        let tx3 = make_tx(anchor_body(3));
        let h1 = tx1.hash;
        let h2 = tx2.hash;
        let h3 = tx3.hash;
        pool.insert(tx1);
        pool.insert(tx2);
        pool.insert(tx3);

        let drained = pool.drain(10);
        assert_eq!(drained[0].hash, h1);
        assert_eq!(drained[1].hash, h2);
        assert_eq!(drained[2].hash, h3);
    }

    #[test]
    fn test_drain_max_limit() {
        let mut pool = Mempool::new();
        pool.insert(make_tx(anchor_body(1)));
        pool.insert(make_tx(anchor_body(2)));
        pool.insert(make_tx(anchor_body(3)));

        let drained = pool.drain(2);
        assert_eq!(drained.len(), 2);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_remove_committed() {
        let mut pool = Mempool::new();
        let tx1 = make_tx(anchor_body(1));
        let tx2 = make_tx(anchor_body(2));
        let tx3 = make_tx(anchor_body(3));
        let h1 = tx1.hash;
        let h3 = tx3.hash;
        pool.insert(tx1);
        pool.insert(tx2);
        pool.insert(tx3);

        pool.remove_committed(&[h1, h3]);
        assert_eq!(pool.len(), 1);
        assert!(!pool.contains(&h1));
        assert!(!pool.contains(&h3));
    }

    #[test]
    fn test_contains() {
        let mut pool = Mempool::new();
        let tx = make_tx(anchor_body(1));
        let hash = tx.hash;
        assert!(!pool.contains(&hash));
        pool.insert(tx);
        assert!(pool.contains(&hash));
    }

    #[test]
    fn test_clear() {
        let mut pool = Mempool::new();
        pool.insert(make_tx(anchor_body(1)));
        pool.insert(make_tx(anchor_body(2)));
        assert_eq!(pool.len(), 2);
        pool.clear();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }
}
