use crate::block::*;
use crate::consensus::kuramoto::*;
use crate::crypto::*;
use crate::transaction::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Leader,
    Follower,
}

/// Tracks attestations for a proposed block
pub struct PendingBlock {
    pub block: Block,
    pub attestations: Vec<Attestation>,
    pub created_at: u64,
}

/// The consensus engine — manages local node's participation in Resonant Consensus
pub struct ConsensusEngine {
    local_keypair: NodeKeypair,
    local_id: [u8; 32],
    kuramoto: KuramotoEngine,
    role: NodeRole,
    pending: Option<PendingBlock>,
    attestation_threshold: usize,
    block_time_ms: u64,
    last_block_time: u64,
    last_tick: u64,
    latest_height: u64,
    latest_hash: [u8; 32],
}

impl ConsensusEngine {
    pub fn new(
        keypair: NodeKeypair,
        coupling: f64,
        chiral_eta: f64,
        block_time_ms: u64,
        validator_count: usize,
    ) -> Self {
        let local_id = keypair.public_key().0;
        let mut kuramoto = KuramotoEngine::new(coupling, chiral_eta);
        // Register ourselves
        kuramoto.update_peer(OscillatorState {
            node_id: local_id,
            phase: 0.0,
            natural_freq: 1.0,
            last_updated: 0,
        });
        let threshold = if validator_count == 0 {
            1
        } else {
            let t = (validator_count + 1) / 2 + 1;
            t.min(validator_count)
        };
        ConsensusEngine {
            local_keypair: keypair,
            local_id,
            kuramoto,
            role: NodeRole::Follower,
            pending: None,
            attestation_threshold: threshold,
            block_time_ms,
            last_block_time: 0,
            last_tick: 0,
            latest_height: 0,
            latest_hash: [0u8; 32],
        }
    }

    /// Update peer's phase from network message
    pub fn update_peer_phase(&mut self, state: OscillatorState) {
        self.kuramoto.update_peer(state);
    }

    /// Tick the consensus engine
    pub fn tick(&mut self, now_ms: u64, transactions: Vec<Transaction>) -> Option<Block> {
        // Advance Kuramoto
        if self.last_tick > 0 {
            let dt = (now_ms - self.last_tick) as f64 / 1000.0;
            self.kuramoto.step(&self.local_id, dt);
        }
        self.last_tick = now_ms;

        // Determine role
        self.role = match self.kuramoto.leader() {
            Some(leader_id) if leader_id == self.local_id => NodeRole::Leader,
            _ => NodeRole::Follower,
        };

        // If Leader and time to propose and have transactions
        if self.role == NodeRole::Leader
            && (now_ms - self.last_block_time >= self.block_time_ms)
            && !transactions.is_empty()
        {
            let block = self.build_block(now_ms, transactions);
            self.pending = Some(PendingBlock {
                block: block.clone(),
                attestations: Vec::new(),
                created_at: now_ms,
            });
            return Some(block);
        }

        None
    }

    /// Receive an attestation for our pending block
    pub fn receive_attestation(&mut self, attestation: Attestation) -> Option<Block> {
        let pending = self.pending.as_mut()?;

        // Verify attestation signature against our pending block hash
        let block_hash = pending.block.hash();
        if !attestation.verify(&block_hash) {
            return None;
        }

        pending.attestations.push(attestation);

        if pending.attestations.len() >= self.attestation_threshold {
            let mut finalized = pending.block.clone();
            finalized.attestations = pending.attestations.clone();
            self.latest_height = finalized.header.height;
            self.latest_hash = finalized.hash();
            self.last_block_time = pending.created_at;
            self.pending = None;
            Some(finalized)
        } else {
            None
        }
    }

    /// Receive a block proposal from another node (we're follower)
    pub fn receive_proposal(&mut self, block: &Block) -> Option<Attestation> {
        // Verify proposer signature
        if !block.verify_proposer_signature() {
            return None;
        }

        // Verify proposer is current leader
        if let Some(leader) = self.kuramoto.leader() {
            if block.header.proposer != leader {
                return None;
            }
        } else {
            return None;
        }

        // Verify chain continuity
        if block.header.height != self.latest_height + 1 {
            return None;
        }
        if block.header.prev_hash != self.latest_hash {
            return None;
        }

        // Create attestation
        let block_hash = block.hash();
        let sig = self.local_keypair.sign(&block_hash);
        Some(Attestation {
            validator: self.local_id,
            signature: sig.into(),
        })
    }

    pub fn role(&self) -> NodeRole {
        self.role
    }

    pub fn order_parameter(&self) -> OrderParameter {
        self.kuramoto.order_parameter()
    }

    pub fn current_leader(&self) -> Option<[u8; 32]> {
        self.kuramoto.leader()
    }

    pub fn local_phase(&self) -> f64 {
        self.kuramoto
            .peers()
            .iter()
            .find(|o| o.node_id == self.local_id)
            .map(|o| o.phase)
            .unwrap_or(0.0)
    }

    /// Set chain state (for testing or initialization)
    pub fn set_chain_state(&mut self, height: u64, hash: [u8; 32]) {
        self.latest_height = height;
        self.latest_hash = hash;
    }

    fn build_block(&self, now_ms: u64, transactions: Vec<Transaction>) -> Block {
        let tx_root = if transactions.is_empty() {
            [0u8; 32]
        } else {
            let combined: Vec<u8> = transactions.iter().flat_map(|tx| tx.hash).collect();
            hash(&combined)
        };

        let header = BlockHeader {
            version: 1,
            height: self.latest_height + 1,
            timestamp: now_ms,
            prev_hash: self.latest_hash,
            tx_root,
            state_root: [0u8; 32], // simplified — no state tree in engine
            proposer: self.local_id,
        };

        let header_bytes = header.serialize();
        let sig = self.local_keypair.sign(&header_bytes);

        Block {
            header,
            header_signature: sig.into(),
            transactions,
            attestations: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::TransactionBody;

    fn make_tx(kp: &NodeKeypair) -> Transaction {
        Transaction::new(
            TransactionBody::AnchorBatch {
                merkle_root: [0x42; 32],
                event_count: 10,
                site_id: "test-site".to_string(),
                metadata: vec![],
            },
            kp,
        )
    }

    fn make_id_from_keypair(kp: &NodeKeypair) -> [u8; 32] {
        kp.public_key().0
    }

    #[test]
    fn test_engine_creation() {
        let kp = NodeKeypair::generate();
        let engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 3);
        // Initially follower (only one node, but it should be leader after tick)
        // Before any tick, role is default Follower
        assert_eq!(engine.role(), NodeRole::Follower);
        assert_eq!(engine.attestation_threshold, 3); // (3+1)/2+1 = 3, min(3,3) = 3
    }

    #[test]
    fn test_leader_detection() {
        let kp = NodeKeypair::generate();
        let local_id = make_id_from_keypair(&kp);
        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 1);

        // Only node — must be leader after tick
        engine.tick(1000, vec![]);
        assert_eq!(engine.role(), NodeRole::Leader);
        assert_eq!(engine.current_leader(), Some(local_id));
    }

    #[test]
    fn test_follower_detection() {
        let kp = NodeKeypair::generate();
        let local_id = make_id_from_keypair(&kp);
        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 3);

        // Add a peer with phase exactly at mean phase (0.0), push local far away
        // First update local phase to be far from 0
        engine.kuramoto.update_peer(OscillatorState {
            node_id: local_id,
            phase: 3.0, // far from 0
            natural_freq: 1.0,
            last_updated: 0,
        });

        let mut peer_id = [0u8; 32];
        peer_id[0] = 0xFF;
        engine.kuramoto.update_peer(OscillatorState {
            node_id: peer_id,
            phase: 0.0, // at mean phase
            natural_freq: 1.0,
            last_updated: 0,
        });

        // Don't advance Kuramoto (last_tick == 0), just determine role
        engine.tick(0, vec![]);

        // The peer at phase 0 should be closer to mean phase
        assert_eq!(engine.role(), NodeRole::Follower);
    }

    #[test]
    fn test_tick_produces_block_when_leader() {
        let kp = NodeKeypair::generate();
        let kp2 = NodeKeypair::generate();
        let tx = make_tx(&kp2);

        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 1);

        // First tick to set last_tick
        engine.tick(0, vec![]);
        // Second tick after block_time_ms with transactions
        let result = engine.tick(1000, vec![tx]);
        assert!(result.is_some());
        let block = result.unwrap();
        assert_eq!(block.header.height, 1);
        assert_eq!(block.transactions.len(), 1);
        assert!(block.verify_proposer_signature());
    }

    #[test]
    fn test_tick_no_block_when_follower() {
        let kp = NodeKeypair::generate();
        let local_id = make_id_from_keypair(&kp);
        let kp2 = NodeKeypair::generate();
        let tx = make_tx(&kp2);

        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 3);

        // Make local a follower
        engine.kuramoto.update_peer(OscillatorState {
            node_id: local_id,
            phase: 3.0,
            natural_freq: 1.0,
            last_updated: 0,
        });
        let mut peer_id = [0u8; 32];
        peer_id[0] = 0xFF;
        engine.kuramoto.update_peer(OscillatorState {
            node_id: peer_id,
            phase: 0.0,
            natural_freq: 1.0,
            last_updated: 0,
        });

        engine.tick(0, vec![]);
        let result = engine.tick(1000, vec![tx]);
        assert!(result.is_none());
    }

    #[test]
    fn test_tick_no_block_when_no_transactions() {
        let kp = NodeKeypair::generate();
        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 1);

        engine.tick(0, vec![]);
        let result = engine.tick(1000, vec![]);
        assert!(result.is_none());
    }

    #[test]
    fn test_receive_attestation_finalizes() {
        let kp = NodeKeypair::generate();
        let kp2 = NodeKeypair::generate();
        let tx = make_tx(&kp2);

        let mut engine = ConsensusEngine::new(kp, 1.0, 0.0, 1000, 2);
        // threshold for 2 validators: ceil(2/2)+1 = 2, min(2,2) = 2

        engine.tick(0, vec![]);
        let block = engine.tick(1000, vec![tx]).unwrap();
        let block_hash = block.hash();

        // First attestation — not enough
        let att1_kp = NodeKeypair::generate();
        let sig1 = att1_kp.sign(&block_hash);
        let att1 = Attestation {
            validator: att1_kp.public_key().0,
            signature: sig1.into(),
        };
        assert!(engine.receive_attestation(att1).is_none());

        // Second attestation — should finalize
        let att2_kp = NodeKeypair::generate();
        let sig2 = att2_kp.sign(&block_hash);
        let att2 = Attestation {
            validator: att2_kp.public_key().0,
            signature: sig2.into(),
        };
        let finalized = engine.receive_attestation(att2);
        assert!(finalized.is_some());
        let finalized = finalized.unwrap();
        assert_eq!(finalized.attestations.len(), 2);
        assert_eq!(finalized.header.height, 1);
    }

    #[test]
    fn test_receive_proposal_returns_attestation() {
        let leader_kp = NodeKeypair::generate();
        let follower_kp = NodeKeypair::generate();
        let leader_id = make_id_from_keypair(&leader_kp);
        let follower_id = make_id_from_keypair(&follower_kp);

        // Create follower engine
        let mut engine = ConsensusEngine::new(follower_kp, 1.0, 0.0, 1000, 3);

        // Set up phases so leader_kp is the leader
        engine.kuramoto.update_peer(OscillatorState {
            node_id: follower_id,
            phase: 3.0, // far from mean
            natural_freq: 1.0,
            last_updated: 0,
        });
        engine.kuramoto.update_peer(OscillatorState {
            node_id: leader_id,
            phase: 0.0, // at mean
            natural_freq: 1.0,
            last_updated: 0,
        });

        // Tick to update role
        engine.tick(0, vec![]);
        assert_eq!(engine.role(), NodeRole::Follower);

        // Leader builds a block
        let tx = make_tx(&leader_kp);
        let header = BlockHeader {
            version: 1,
            height: 1, // engine starts at height 0
            timestamp: 1000,
            prev_hash: [0u8; 32], // engine starts with [0;32]
            tx_root: {
                let combined: Vec<u8> = [tx.hash].iter().flat_map(|h| h.iter().copied()).collect();
                hash(&combined)
            },
            state_root: [0u8; 32],
            proposer: leader_id,
        };
        let sig = leader_kp.sign(&header.serialize());
        let block = Block {
            header,
            header_signature: sig.into(),
            transactions: vec![tx],
            attestations: vec![],
        };

        let attestation = engine.receive_proposal(&block);
        assert!(attestation.is_some());
        let att = attestation.unwrap();
        assert_eq!(att.validator, follower_id);
        // Verify the attestation is valid
        assert!(att.verify(&block.hash()));
    }
}
