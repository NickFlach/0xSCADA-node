use serde::{Deserialize, Serialize};

pub const MAX_SYNC_BATCH: usize = 100;
pub const SYNC_THRESHOLD: u64 = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SyncMessage {
    BlockRequest {
        from_height: u64,
        to_height: u64,
        requester_id: [u8; 32],
    },
    BlockResponse {
        blocks: Vec<Vec<u8>>,
        has_more: bool,
    },
    StatusRequest {
        requester_id: [u8; 32],
    },
    StatusResponse {
        latest_height: u64,
        latest_hash: [u8; 32],
        validator_count: u32,
    },
}

impl SyncMessage {
    pub fn serialize(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf).map_err(|e| format!("CBOR serialize error: {e}"))?;
        Ok(buf)
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        ciborium::de::from_reader(data).map_err(|e| format!("CBOR deserialize error: {e}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Synced,
    Syncing {
        target_height: u64,
        current_request_height: u64,
    },
    NeedSync,
}

pub struct SyncManager {
    local_height: u64,
    state: SyncState,
}

impl SyncManager {
    pub fn new(local_height: u64) -> Self {
        Self {
            local_height,
            state: SyncState::Synced,
        }
    }

    pub fn peer_status_received(&mut self, peer_height: u64) -> Option<SyncMessage> {
        if peer_height > self.local_height + SYNC_THRESHOLD {
            self.state = SyncState::NeedSync;
            let from = self.local_height + 1;
            let to = (from + MAX_SYNC_BATCH as u64 - 1).min(peer_height);
            self.state = SyncState::Syncing {
                target_height: peer_height,
                current_request_height: from,
            };
            Some(SyncMessage::BlockRequest {
                from_height: from,
                to_height: to,
                requester_id: [0u8; 32],
            })
        } else {
            None
        }
    }

    pub fn blocks_received(&mut self, count: usize, has_more: bool) -> Option<SyncMessage> {
        if let SyncState::Syncing { target_height, current_request_height } = self.state {
            let new_height = current_request_height + count as u64 - 1;
            if has_more {
                let from = new_height + 1;
                let to = (from + MAX_SYNC_BATCH as u64 - 1).min(target_height);
                self.state = SyncState::Syncing {
                    target_height,
                    current_request_height: from,
                };
                Some(SyncMessage::BlockRequest {
                    from_height: from,
                    to_height: to,
                    requester_id: [0u8; 32],
                })
            } else {
                self.state = SyncState::Synced;
                None
            }
        } else {
            None
        }
    }

    pub fn block_applied(&mut self, new_height: u64) {
        self.local_height = new_height;
    }

    pub fn handle_block_request(
        &self,
        from: u64,
        to: u64,
        block_fetcher: impl Fn(u64) -> Option<Vec<u8>>,
    ) -> SyncMessage {
        let clamped_to = to.min(from + MAX_SYNC_BATCH as u64 - 1);
        let mut blocks = Vec::new();
        for h in from..=clamped_to {
            if let Some(block) = block_fetcher(h) {
                blocks.push(block);
            }
        }
        let has_more = to > clamped_to;
        SyncMessage::BlockResponse { blocks, has_more }
    }

    pub fn state(&self) -> SyncState {
        self.state
    }

    pub fn is_synced(&self) -> bool {
        self.state == SyncState::Synced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starts_synced() {
        let mgr = SyncManager::new(0);
        assert_eq!(mgr.state(), SyncState::Synced);
        assert!(mgr.is_synced());
    }

    #[test]
    fn test_peer_triggers_sync() {
        let mut mgr = SyncManager::new(0);
        let msg = mgr.peer_status_received(100);
        assert!(msg.is_some());
        match msg.unwrap() {
            SyncMessage::BlockRequest { from_height, to_height, .. } => {
                assert_eq!(from_height, 1);
                assert_eq!(to_height, 100);
            }
            _ => panic!("expected BlockRequest"),
        }
        match mgr.state() {
            SyncState::Syncing { target_height, .. } => assert_eq!(target_height, 100),
            _ => panic!("expected Syncing"),
        }
    }

    #[test]
    fn test_no_sync_when_close() {
        let mut mgr = SyncManager::new(0);
        let msg = mgr.peer_status_received(5);
        assert!(msg.is_none());
        assert!(mgr.is_synced());
    }

    #[test]
    fn test_blocks_received_continues() {
        let mut mgr = SyncManager::new(0);
        mgr.peer_status_received(250);
        let msg = mgr.blocks_received(100, true);
        assert!(msg.is_some());
        match msg.unwrap() {
            SyncMessage::BlockRequest { from_height, .. } => {
                assert_eq!(from_height, 101);
            }
            _ => panic!("expected BlockRequest"),
        }
        assert!(!mgr.is_synced());
    }

    #[test]
    fn test_blocks_received_completes() {
        let mut mgr = SyncManager::new(0);
        mgr.peer_status_received(50);
        let msg = mgr.blocks_received(50, false);
        assert!(msg.is_none());
        assert!(mgr.is_synced());
    }

    #[test]
    fn test_block_applied_updates_height() {
        let mut mgr = SyncManager::new(0);
        mgr.block_applied(42);
        // After applying, peer at 50 is within threshold
        let msg = mgr.peer_status_received(50);
        assert!(msg.is_none());
    }

    #[test]
    fn test_handle_block_request() {
        let mgr = SyncManager::new(100);
        let resp = mgr.handle_block_request(1, 50, |h| Some(vec![h as u8]));
        match resp {
            SyncMessage::BlockResponse { blocks, has_more } => {
                assert_eq!(blocks.len(), 50);
                assert!(!has_more);
            }
            _ => panic!("expected BlockResponse"),
        }
    }

    #[test]
    fn test_handle_block_request_partial() {
        let mgr = SyncManager::new(100);
        let resp = mgr.handle_block_request(1, 10, |h| {
            if h % 2 == 0 { Some(vec![h as u8]) } else { None }
        });
        match resp {
            SyncMessage::BlockResponse { blocks, has_more } => {
                assert_eq!(blocks.len(), 5); // only even heights
                assert!(!has_more);
            }
            _ => panic!("expected BlockResponse"),
        }
    }

    #[test]
    fn test_sync_message_roundtrip() {
        let messages = vec![
            SyncMessage::BlockRequest { from_height: 1, to_height: 100, requester_id: [1u8; 32] },
            SyncMessage::BlockResponse { blocks: vec![vec![1, 2, 3]], has_more: true },
            SyncMessage::StatusRequest { requester_id: [2u8; 32] },
            SyncMessage::StatusResponse { latest_height: 42, latest_hash: [3u8; 32], validator_count: 5 },
        ];
        for msg in messages {
            let data = msg.serialize().expect("serialize");
            let decoded = SyncMessage::deserialize(&data).expect("deserialize");
            // Verify round-trip by re-serializing
            let data2 = decoded.serialize().expect("re-serialize");
            assert_eq!(data, data2);
        }
    }

    #[test]
    fn test_max_batch_respected() {
        let mgr = SyncManager::new(2000);
        let resp = mgr.handle_block_request(1, 1000, |h| Some(vec![h as u8]));
        match resp {
            SyncMessage::BlockResponse { blocks, has_more } => {
                assert_eq!(blocks.len(), MAX_SYNC_BATCH);
                assert!(has_more);
            }
            _ => panic!("expected BlockResponse"),
        }
    }
}
