use serde::{Serialize, Deserialize};
use serde_big_array::BigArray;

/// All messages exchanged between consensus nodes
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ConsensusMessage {
    /// Kuramoto phase broadcast — sent periodically
    PhaseUpdate {
        node_id: [u8; 32],
        phase: f64,
        natural_freq: f64,
        timestamp: u64,
    },

    /// Leader proposes a new block
    BlockProposal {
        block_data: Vec<u8>,
    },

    /// Follower attests to a proposed block
    BlockAttestation {
        block_hash: [u8; 32],
        validator: [u8; 32],
        #[serde(with = "BigArray")]
        signature: [u8; 64],
    },

    /// Raft fallback: request votes for leader election
    VoteRequest {
        term: u64,
        candidate_id: [u8; 32],
        last_block_height: u64,
        last_block_hash: [u8; 32],
    },

    /// Raft fallback: vote response
    VoteResponse {
        term: u64,
        voter_id: [u8; 32],
        vote_granted: bool,
    },

    /// Raft fallback: leader heartbeat (during Raft mode)
    RaftHeartbeat {
        term: u64,
        leader_id: [u8; 32],
        latest_height: u64,
    },
}

impl ConsensusMessage {
    /// Serialize to CBOR bytes
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).expect("CBOR serialization failed");
        buf
    }

    /// Deserialize from CBOR bytes
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        ciborium::from_reader(data).map_err(|e| format!("CBOR deserialization failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization_roundtrip() {
        let messages = vec![
            ConsensusMessage::PhaseUpdate {
                node_id: [1u8; 32],
                phase: 1.5,
                natural_freq: 0.1,
                timestamp: 12345,
            },
            ConsensusMessage::BlockProposal {
                block_data: vec![0xDE, 0xAD],
            },
            ConsensusMessage::BlockAttestation {
                block_hash: [2u8; 32],
                validator: [3u8; 32],
                signature: [4u8; 64],
            },
            ConsensusMessage::VoteRequest {
                term: 1,
                candidate_id: [5u8; 32],
                last_block_height: 100,
                last_block_hash: [6u8; 32],
            },
            ConsensusMessage::VoteResponse {
                term: 1,
                voter_id: [7u8; 32],
                vote_granted: true,
            },
            ConsensusMessage::RaftHeartbeat {
                term: 2,
                leader_id: [8u8; 32],
                latest_height: 50,
            },
        ];

        for msg in &messages {
            let bytes = msg.serialize();
            let recovered = ConsensusMessage::deserialize(&bytes).unwrap();
            assert_eq!(msg, &recovered);
        }
    }
}
