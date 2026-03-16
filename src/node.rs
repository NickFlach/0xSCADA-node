use crate::block::*;
use crate::consensus::engine::*;
use crate::consensus::kuramoto::OscillatorState;
use crate::consensus::messages::*;
use crate::crypto::NodeKeypair;
use crate::mempool::*;
use crate::transaction::Transaction;

use std::collections::HashSet;

/// Node configuration
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub listen_addr: String,
    pub seed_peers: Vec<String>,
    pub data_dir: String,
    pub block_time_ms: u64,
    pub coupling: f64,
    pub chiral_eta: f64,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9000".to_string(),
            seed_peers: vec![],
            data_dir: "./data".to_string(),
            block_time_ms: 2000,
            coupling: 2.0,
            chiral_eta: 0.05,
        }
    }
}

/// Events that the node processes
#[derive(Debug)]
pub enum NodeEvent {
    ConsensusMsg(ConsensusMessage),
    NewTransaction(Transaction),
    Tick(u64),
    PeerConnected { node_id: [u8; 32] },
    PeerDisconnected { node_id: [u8; 32] },
}

/// Actions the node wants to perform (returned from handle_event)
#[derive(Debug)]
pub enum NodeAction {
    Broadcast(ConsensusMessage),
    SendTo { node_id: [u8; 32], msg: ConsensusMessage },
    BlockCommitted { height: u64, hash: [u8; 32] },
    None,
}

/// The node state machine — processes events and returns actions
/// Deliberately NOT async — pure state machine that the outer loop drives
pub struct Node {
    #[allow(dead_code)]
    config: NodeConfig,
    node_id: [u8; 32],
    consensus: ConsensusEngine,
    mempool: Mempool,
    latest_height: u64,
    latest_hash: [u8; 32],
    #[allow(dead_code)]
    validator_set: Vec<[u8; 32]>,
    connected_peers: HashSet<[u8; 32]>,
}

impl Node {
    pub fn new(config: NodeConfig, keypair: NodeKeypair, validator_set: Vec<[u8; 32]>) -> Self {
        let node_id = keypair.public_key().0;
        let mut consensus = ConsensusEngine::new(
            keypair,
            config.coupling,
            config.chiral_eta,
            config.block_time_ms,
            validator_set.len().max(1),
        );
        consensus.set_validator_set(validator_set.clone());

        Node {
            config,
            node_id,
            consensus,
            mempool: Mempool::new(),
            latest_height: 0,
            latest_hash: [0u8; 32],
            validator_set,
            connected_peers: HashSet::new(),
        }
    }

    /// Process an event and return action(s) to perform
    pub fn handle_event(&mut self, event: NodeEvent) -> Vec<NodeAction> {
        match event {
            NodeEvent::Tick(now_ms) => {
                let txs = self.mempool.drain(MAX_TRANSACTIONS_PER_BLOCK);
                let mut actions = Vec::new();

                if let Some(block) = self.consensus.tick(now_ms, txs) {
                    let block_data = block.serialize();
                    actions.push(NodeAction::Broadcast(ConsensusMessage::BlockProposal {
                        block_data,
                    }));
                }

                // Broadcast phase update
                let phase = self.consensus.local_phase();
                actions.push(NodeAction::Broadcast(ConsensusMessage::PhaseUpdate {
                    node_id: self.node_id,
                    phase,
                    natural_freq: 1.0,
                    timestamp: now_ms,
                }));

                actions
            }

            NodeEvent::ConsensusMsg(ConsensusMessage::PhaseUpdate {
                node_id, phase, natural_freq, timestamp,
            }) => {
                // Auto-discover validators from phase updates
                if !self.validator_set.contains(&node_id) {
                    self.validator_set.push(node_id);
                    self.consensus.set_validator_set(self.validator_set.clone());
                }
                self.consensus.update_peer_phase(OscillatorState {
                    node_id,
                    phase,
                    natural_freq,
                    last_updated: timestamp,
                });
                vec![NodeAction::None]
            }

            NodeEvent::ConsensusMsg(ConsensusMessage::BlockProposal { block_data }) => {
                match Block::deserialize(&block_data) {
                    Ok(block) => {
                        let proposer = block.header.proposer;
                        if let Some(attestation) = self.consensus.receive_proposal(&block) {
                            // ** FIX: Update follower chain state after creating attestation **
                            self.latest_height = block.header.height;
                            self.latest_hash = block.hash();
                            // Also update consensus engine's chain state
                            self.consensus.set_chain_state(block.header.height, block.hash());
                            
                            let block_hash = block.hash();
                            vec![NodeAction::SendTo {
                                node_id: proposer,
                                msg: ConsensusMessage::BlockAttestation {
                                    block_hash,
                                    validator: attestation.validator,
                                    signature: attestation.signature,
                                },
                            }]
                        } else {
                            vec![NodeAction::None]
                        }
                    }
                    Err(_) => vec![NodeAction::None],
                }
            }

            NodeEvent::ConsensusMsg(ConsensusMessage::BlockAttestation {
                block_hash: _,
                validator,
                signature,
            }) => {
                let has_pending = self.consensus.has_pending();
                let attestation = Attestation { validator, signature };
                eprintln!("[oxscada-debug] Attestation received from {:02x}{:02x}..., pending={}", validator[0], validator[1], has_pending);
                if let Some(finalized) = self.consensus.receive_attestation(attestation) {
                    self.latest_height = finalized.header.height;
                    self.latest_hash = finalized.hash();
                    vec![NodeAction::BlockCommitted {
                        height: self.latest_height,
                        hash: self.latest_hash,
                    }]
                } else {
                    vec![NodeAction::None]
                }
            }

            NodeEvent::ConsensusMsg(_) => {
                // VoteRequest, VoteResponse, RaftHeartbeat — handled by raft fallback
                // TODO: wire up raft messages
                vec![NodeAction::None]
            }

            NodeEvent::NewTransaction(tx) => {
                self.mempool.insert(tx);
                vec![NodeAction::None]
            }

            NodeEvent::PeerConnected { node_id } => {
                self.connected_peers.insert(node_id);
                vec![NodeAction::None]
            }

            NodeEvent::PeerDisconnected { node_id } => {
                self.connected_peers.remove(&node_id);
                vec![NodeAction::None]
            }
        }
    }

    /// Get current node status
    pub fn status(&self) -> NodeStatus {
        let role = match self.consensus.role() {
            NodeRole::Leader => "leader".to_string(),
            NodeRole::Follower => "follower".to_string(),
        };
        NodeStatus {
            node_id: self.node_id,
            latest_height: self.latest_height,
            latest_hash: self.latest_hash,
            role,
            order_parameter: self.consensus.order_parameter().r,
            connected_peers: self.connected_peers.len(),
            mempool_size: self.mempool.len(),
        }
    }

    /// Get the local node's public key
    pub fn node_id(&self) -> [u8; 32] {
        self.node_id
    }
}

#[derive(Debug, Clone)]
pub struct NodeStatus {
    pub node_id: [u8; 32],
    pub latest_height: u64,
    pub latest_hash: [u8; 32],
    pub role: String,
    pub order_parameter: f64,
    pub connected_peers: usize,
    pub mempool_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::TransactionBody;

    fn make_node(validator_set: Vec<[u8; 32]>) -> Node {
        let kp = NodeKeypair::generate();
        let config = NodeConfig::default();
        Node::new(config, kp, validator_set)
    }

    fn make_node_with_keypair(keypair: NodeKeypair, validator_set: Vec<[u8; 32]>) -> Node {
        let config = NodeConfig::default();
        Node::new(config, keypair, validator_set)
    }

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

    #[test]
    fn test_node_creation() {
        let kp = NodeKeypair::generate();
        let node_id = kp.public_key().0;
        let node = make_node_with_keypair(kp, vec![node_id]);
        assert_eq!(node.node_id(), node_id);
        assert_eq!(node.latest_height, 0);
        assert_eq!(node.latest_hash, [0u8; 32]);
        assert!(node.mempool.is_empty());
        assert!(node.connected_peers.is_empty());
    }

    #[test]
    fn test_tick_as_single_validator() {
        let kp = NodeKeypair::generate();
        let node_id = kp.public_key().0;
        let tx_kp = NodeKeypair::generate();

        let mut node = make_node_with_keypair(kp, vec![node_id]);

        // First tick — sets last_tick in consensus engine
        node.handle_event(NodeEvent::Tick(0));

        // Submit transaction AFTER first tick so it's available for the second tick
        let tx = make_tx(&tx_kp);
        node.handle_event(NodeEvent::NewTransaction(tx));
        assert_eq!(node.mempool.len(), 1);

        // Second tick after block_time_ms — should produce block
        let actions = node.handle_event(NodeEvent::Tick(2000));

        // Should have a Broadcast(BlockProposal) and Broadcast(PhaseUpdate)
        let has_proposal = actions.iter().any(|a| matches!(a, NodeAction::Broadcast(ConsensusMessage::BlockProposal { .. })));
        assert!(has_proposal, "Single validator should produce a block proposal on tick");
    }

    #[test]
    fn test_submit_transaction() {
        let mut node = make_node(vec![]);
        let tx_kp = NodeKeypair::generate();
        let tx = make_tx(&tx_kp);

        assert_eq!(node.mempool.len(), 0);
        node.handle_event(NodeEvent::NewTransaction(tx));
        assert_eq!(node.mempool.len(), 1);
    }

    #[test]
    fn test_phase_update_received() {
        let mut node = make_node(vec![]);
        let peer_id = [0xAA; 32];

        let actions = node.handle_event(NodeEvent::ConsensusMsg(ConsensusMessage::PhaseUpdate {
            node_id: peer_id,
            phase: 1.5,
            natural_freq: 1.0,
            timestamp: 1000,
        }));

        assert!(actions.iter().all(|a| matches!(a, NodeAction::None)));
    }

    #[test]
    fn test_proposal_generates_attestation() {
        // Create leader and follower
        let leader_kp = NodeKeypair::generate();
        let follower_kp = NodeKeypair::generate();
        let leader_id = leader_kp.public_key().0;
        let follower_id = follower_kp.public_key().0;

        let mut follower = make_node_with_keypair(follower_kp, vec![leader_id, follower_id]);

        // Set up phases so leader is the leader (closer to mean phase)
        // Push follower's phase far from mean
        follower.handle_event(NodeEvent::ConsensusMsg(ConsensusMessage::PhaseUpdate {
            node_id: follower_id,
            phase: 3.0,
            natural_freq: 1.0,
            timestamp: 0,
        }));
        follower.handle_event(NodeEvent::ConsensusMsg(ConsensusMessage::PhaseUpdate {
            node_id: leader_id,
            phase: 0.0,
            natural_freq: 1.0,
            timestamp: 0,
        }));

        // Tick follower to update role
        follower.handle_event(NodeEvent::Tick(0));

        // Leader builds a block
        let tx_kp = NodeKeypair::generate();
        let tx = make_tx(&tx_kp);
        let header = BlockHeader {
            version: 1,
            height: 1,
            timestamp: 1000,
            prev_hash: [0u8; 32],
            tx_root: {
                let combined: Vec<u8> = [tx.hash].iter().flat_map(|h| h.iter().copied()).collect();
                crate::crypto::hash(&combined)
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

        let block_data = block.serialize();
        let actions = follower.handle_event(NodeEvent::ConsensusMsg(
            ConsensusMessage::BlockProposal { block_data },
        ));

        let has_send_to = actions.iter().any(|a| matches!(a, NodeAction::SendTo { node_id, msg: ConsensusMessage::BlockAttestation { .. } } if *node_id == leader_id));
        assert!(has_send_to, "Follower should send attestation back to leader");
    }

    #[test]
    fn test_attestation_finalizes_block() {
        let kp = NodeKeypair::generate();
        let node_id = kp.public_key().0;
        let tx_kp = NodeKeypair::generate();
        let tx = make_tx(&tx_kp);

        // Single validator — threshold is 1
        // Empty validator set so attestation validation is permissive
        let mut node = make_node_with_keypair(kp, vec![]);

        // First tick to initialize consensus timing
        node.handle_event(NodeEvent::Tick(0));
        // Submit tx after first tick
        node.handle_event(NodeEvent::NewTransaction(tx));
        let actions = node.handle_event(NodeEvent::Tick(2000));

        // Extract the proposed block to get its hash
        let proposal_data = actions.iter().find_map(|a| match a {
            NodeAction::Broadcast(ConsensusMessage::BlockProposal { block_data }) => Some(block_data.clone()),
            _ => None,
        }).expect("Should have proposal");

        let block = Block::deserialize(&proposal_data).unwrap();
        let block_hash = block.hash();

        // Create attestation from a validator
        let att_kp = NodeKeypair::generate();
        let att_sig = att_kp.sign(&block_hash);

        let actions = node.handle_event(NodeEvent::ConsensusMsg(
            ConsensusMessage::BlockAttestation {
                block_hash,
                validator: att_kp.public_key().0,
                signature: att_sig.into(),
            },
        ));

        let has_committed = actions.iter().any(|a| matches!(a, NodeAction::BlockCommitted { height: 1, .. }));
        assert!(has_committed, "Should finalize block after receiving enough attestations");
        assert_eq!(node.latest_height, 1);
    }

    #[test]
    fn test_status() {
        let kp = NodeKeypair::generate();
        let node_id = kp.public_key().0;
        let mut node = make_node_with_keypair(kp, vec![node_id]);

        // Add some peers
        node.handle_event(NodeEvent::PeerConnected { node_id: [1u8; 32] });
        node.handle_event(NodeEvent::PeerConnected { node_id: [2u8; 32] });

        // Add a transaction
        let tx_kp = NodeKeypair::generate();
        node.handle_event(NodeEvent::NewTransaction(make_tx(&tx_kp)));

        let status = node.status();
        assert_eq!(status.node_id, node_id);
        assert_eq!(status.latest_height, 0);
        assert_eq!(status.connected_peers, 2);
        assert_eq!(status.mempool_size, 1);
        assert!(status.role == "leader" || status.role == "follower");
    }

    #[test]
    fn test_peer_connect_disconnect() {
        let mut node = make_node(vec![]);
        let peer = [0xBB; 32];

        node.handle_event(NodeEvent::PeerConnected { node_id: peer });
        assert_eq!(node.connected_peers.len(), 1);

        node.handle_event(NodeEvent::PeerDisconnected { node_id: peer });
        assert_eq!(node.connected_peers.len(), 0);
    }

    #[test]
    fn test_follower_chain_state_advances() {
        let leader_kp = NodeKeypair::generate();
        let follower_kp = NodeKeypair::generate();
        let leader_id = leader_kp.public_key().0;
        let follower_id = follower_kp.public_key().0;
        
        // Create follower with leader in validator set
        let mut follower = Node::new(
            NodeConfig::default(),
            follower_kp,
            vec![leader_id, follower_id],
        );
        
        // Initial chain state should be at height 0
        assert_eq!(follower.latest_height, 0);
        assert_eq!(follower.latest_hash, [0u8; 32]);
        
        // Create a proposal from the leader
        let tx = make_tx(&leader_kp);
        let header = BlockHeader {
            version: 1,
            height: 1,
            timestamp: 1000,
            prev_hash: [0u8; 32],
            tx_root: {
                let combined: Vec<u8> = [tx.hash].iter().flat_map(|h| h.iter().copied()).collect();
                crate::crypto::hash(&combined)
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
        
        let block_hash = block.hash();
        let block_data = block.serialize();
        
        // Follower receives the proposal
        let actions = follower.handle_event(NodeEvent::ConsensusMsg(
            ConsensusMessage::BlockProposal { block_data },
        ));
        
        // ** FIX VERIFICATION: Follower chain state should advance **
        assert_eq!(follower.latest_height, 1, "Follower height should advance to 1");
        assert_eq!(follower.latest_hash, block_hash, "Follower hash should match block hash");
        
        // Should also send attestation
        let has_attestation = actions.iter().any(|a| matches!(a, NodeAction::SendTo { .. }));
        assert!(has_attestation, "Follower should send attestation to leader");
    }

    #[test]
    fn test_two_node_consensus_scenario() {
        // This test verifies the full consensus flow between a leader and follower:
        // 1. Leader proposes block
        // 2. Follower attests
        // 3. Leader finalizes
        // 4. Both nodes have consistent state
        
        let leader_kp = NodeKeypair::generate();
        let follower_kp = NodeKeypair::generate();
        let leader_id = leader_kp.public_key().0;
        let follower_id = follower_kp.public_key().0;
        
        // Create leader with both validators so it expects attestations
        let mut leader = Node::new(
            NodeConfig::default(),
            leader_kp,
            vec![leader_id, follower_id], // Include both to test attestation flow
        );
        
        // Create follower with both validators
        let mut follower = Node::new(
            NodeConfig::default(),
            follower_kp,
            vec![leader_id, follower_id],
        );
        
        // Leader first tick to initialize consensus engine timing
        leader.handle_event(NodeEvent::Tick(0));
        
        // Add transaction and tick to create a block proposal
        let tx_kp = NodeKeypair::generate();
        let tx = make_tx(&tx_kp);
        leader.handle_event(NodeEvent::NewTransaction(tx));
        
        // Second tick after block_time_ms to trigger proposal
        let leader_actions = leader.handle_event(NodeEvent::Tick(2000));
        
        // Verify leader proposed a block
        let proposal_data = leader_actions.iter().find_map(|a| match a {
            NodeAction::Broadcast(ConsensusMessage::BlockProposal { block_data }) => Some(block_data.clone()),
            _ => None,
        }).expect("Leader should propose a block");
        
        // Follower receives the proposal
        let follower_actions = follower.handle_event(NodeEvent::ConsensusMsg(
            ConsensusMessage::BlockProposal { block_data: proposal_data },
        ));
        
        // Extract the attestation that follower wants to send to leader
        let attestation_msg = follower_actions.iter().find_map(|a| match a {
            NodeAction::SendTo { node_id, msg } if *node_id == leader_id => match msg {
                ConsensusMessage::BlockAttestation { block_hash, validator, signature } => 
                    Some((*block_hash, *validator, *signature)),
                _ => None,
            },
            _ => None,
        }).expect("Follower should send attestation to leader");
        
        // Check that follower's chain state advanced (Bug 3 fix verification)
        assert_eq!(follower.latest_height, 1, "Follower chain state should advance");
        
        // Now simulate leader receiving the attestation from follower
        let leader_finalize_actions = leader.handle_event(NodeEvent::ConsensusMsg(
            ConsensusMessage::BlockAttestation {
                block_hash: attestation_msg.0,
                validator: attestation_msg.1,
                signature: attestation_msg.2,
            }
        ));
        
        // Leader should commit the block after receiving attestation
        let has_commit = leader_finalize_actions.iter().any(|a| matches!(a, NodeAction::BlockCommitted { .. }));
        assert!(has_commit, "Leader should commit finalized block");
        
        // Now both nodes should have advanced chain state
        assert_eq!(leader.latest_height, 1, "Leader should advance to height 1");
        assert_eq!(follower.latest_height, 1, "Follower should advance to height 1");
        assert_eq!(leader.latest_hash, follower.latest_hash, "Both nodes should have same latest hash");
    }
}
