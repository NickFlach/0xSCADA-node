use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
}

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node_id: [u8; 32],
    pub addr: SocketAddr,
    pub state: PeerState,
    pub last_seen: u64,
    pub reconnect_attempts: u32,
}

pub struct PeerManager {
    _local_id: [u8; 32],
    peers: HashMap<[u8; 32], PeerInfo>,
    seed_addrs: Vec<SocketAddr>,
    max_reconnect_attempts: u32,
    _reconnect_delay_ms: u64,
}

impl PeerManager {
    pub fn new(local_id: [u8; 32], seed_addrs: Vec<SocketAddr>) -> Self {
        Self {
            _local_id: local_id,
            peers: HashMap::new(),
            seed_addrs,
            max_reconnect_attempts: 5,
            _reconnect_delay_ms: 1000,
        }
    }

    pub fn add_peer(&mut self, node_id: [u8; 32], addr: SocketAddr) {
        self.peers.insert(node_id, PeerInfo {
            node_id,
            addr,
            state: PeerState::Disconnected,
            last_seen: 0,
            reconnect_attempts: 0,
        });
    }

    pub fn set_peer_state(&mut self, node_id: &[u8; 32], state: PeerState, now_ms: u64) {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.state = state;
            peer.last_seen = now_ms;
        }
    }

    pub fn mark_seen(&mut self, node_id: &[u8; 32], now_ms: u64) {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.last_seen = now_ms;
        }
    }

    pub fn peers_needing_connect(&self) -> Vec<PeerInfo> {
        self.peers.values()
            .filter(|p| {
                (p.state == PeerState::Disconnected || p.state == PeerState::Reconnecting)
                    && p.reconnect_attempts < self.max_reconnect_attempts
            })
            .cloned()
            .collect()
    }

    pub fn connected_peers(&self) -> Vec<&PeerInfo> {
        self.peers.values().filter(|p| p.state == PeerState::Connected).collect()
    }

    pub fn peer_count(&self) -> (usize, usize, usize) {
        let mut connected = 0;
        let mut connecting = 0;
        let mut disconnected = 0;
        for p in self.peers.values() {
            match p.state {
                PeerState::Connected => connected += 1,
                PeerState::Connecting => connecting += 1,
                PeerState::Disconnected | PeerState::Reconnecting => disconnected += 1,
            }
        }
        (connected, connecting, disconnected)
    }

    pub fn get_peer(&self, node_id: &[u8; 32]) -> Option<&PeerInfo> {
        self.peers.get(node_id)
    }

    pub fn increment_reconnect(&mut self, node_id: &[u8; 32]) {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.reconnect_attempts += 1;
        }
    }

    pub fn reset_reconnect(&mut self, node_id: &[u8; 32]) {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.reconnect_attempts = 0;
        }
    }

    pub fn remove_peer(&mut self, node_id: &[u8; 32]) {
        self.peers.remove(node_id);
    }

    pub fn peer_ids(&self) -> Vec<[u8; 32]> {
        self.peers.keys().copied().collect()
    }

    pub fn seeds(&self) -> &[SocketAddr] {
        &self.seed_addrs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    fn addr(port: u16) -> SocketAddr {
        use std::net::{IpAddr, Ipv4Addr};
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    fn new_mgr() -> PeerManager {
        PeerManager::new(make_id(0), vec![addr(9000)])
    }

    #[test]
    fn test_add_peer() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        let peer = mgr.get_peer(&make_id(1)).unwrap();
        assert_eq!(peer.state, PeerState::Disconnected);
        assert_eq!(peer.addr, addr(9001));
    }

    #[test]
    fn test_set_peer_state() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        mgr.set_peer_state(&make_id(1), PeerState::Connecting, 100);
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().state, PeerState::Connecting);
        mgr.set_peer_state(&make_id(1), PeerState::Connected, 200);
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().state, PeerState::Connected);
    }

    #[test]
    fn test_mark_seen() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().last_seen, 0);
        mgr.mark_seen(&make_id(1), 500);
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().last_seen, 500);
    }

    #[test]
    fn test_peers_needing_connect() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        mgr.add_peer(make_id(2), addr(9002));
        mgr.set_peer_state(&make_id(2), PeerState::Connected, 100);
        let need = mgr.peers_needing_connect();
        assert_eq!(need.len(), 1);
        assert_eq!(need[0].node_id, make_id(1));
    }

    #[test]
    fn test_max_reconnect_exceeded() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        for _ in 0..5 {
            mgr.increment_reconnect(&make_id(1));
        }
        assert!(mgr.peers_needing_connect().is_empty());
    }

    #[test]
    fn test_connected_peers() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        mgr.add_peer(make_id(2), addr(9002));
        mgr.set_peer_state(&make_id(1), PeerState::Connected, 100);
        let connected = mgr.connected_peers();
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].node_id, make_id(1));
    }

    #[test]
    fn test_peer_count() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        mgr.add_peer(make_id(2), addr(9002));
        mgr.add_peer(make_id(3), addr(9003));
        mgr.set_peer_state(&make_id(1), PeerState::Connected, 100);
        mgr.set_peer_state(&make_id(2), PeerState::Connecting, 100);
        let (c, ing, d) = mgr.peer_count();
        assert_eq!(c, 1);
        assert_eq!(ing, 1);
        assert_eq!(d, 1);
    }

    #[test]
    fn test_reset_reconnect() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        mgr.increment_reconnect(&make_id(1));
        mgr.increment_reconnect(&make_id(1));
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().reconnect_attempts, 2);
        mgr.reset_reconnect(&make_id(1));
        assert_eq!(mgr.get_peer(&make_id(1)).unwrap().reconnect_attempts, 0);
    }

    #[test]
    fn test_remove_peer() {
        let mut mgr = new_mgr();
        mgr.add_peer(make_id(1), addr(9001));
        assert!(mgr.get_peer(&make_id(1)).is_some());
        mgr.remove_peer(&make_id(1));
        assert!(mgr.get_peer(&make_id(1)).is_none());
    }
}
