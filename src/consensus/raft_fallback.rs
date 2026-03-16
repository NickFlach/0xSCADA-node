use rand::Rng;
use super::messages::ConsensusMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaftState {
    Inactive,
    Follower,
    Candidate,
    Leader,
}

pub struct RaftFallback {
    state: RaftState,
    current_term: u64,
    voted_for: Option<[u8; 32]>,
    local_id: [u8; 32],
    votes_received: Vec<[u8; 32]>,
    validator_count: usize,
    election_timeout_ms: u64,
    last_heartbeat: u64,
    leader_id: Option<[u8; 32]>,
}

impl RaftFallback {
    pub fn new(local_id: [u8; 32], validator_count: usize) -> Self {
        Self {
            state: RaftState::Inactive,
            current_term: 0,
            voted_for: None,
            local_id,
            votes_received: Vec::new(),
            validator_count,
            election_timeout_ms: rand::thread_rng().gen_range(2000..5000),
            last_heartbeat: 0,
            leader_id: None,
        }
    }

    /// Activate Raft fallback (called when Kuramoto r < 0.5)
    pub fn activate(&mut self, now_ms: u64) {
        if self.state == RaftState::Inactive {
            self.state = RaftState::Follower;
            self.last_heartbeat = now_ms;
            self.randomize_timeout();
        }
    }

    /// Deactivate (called when Kuramoto r >= 0.5 again)
    pub fn deactivate(&mut self) {
        self.state = RaftState::Inactive;
        self.leader_id = None;
        self.voted_for = None;
        self.votes_received.clear();
    }

    /// Tick — check timeouts, start election if needed
    pub fn tick(&mut self, now_ms: u64) -> Vec<ConsensusMessage> {
        match self.state {
            RaftState::Inactive => vec![],
            RaftState::Follower | RaftState::Candidate => {
                if now_ms.saturating_sub(self.last_heartbeat) >= self.election_timeout_ms {
                    self.start_election(now_ms)
                } else {
                    vec![]
                }
            }
            RaftState::Leader => {
                vec![ConsensusMessage::RaftHeartbeat {
                    term: self.current_term,
                    leader_id: self.local_id,
                    latest_height: 0,
                }]
            }
        }
    }

    /// Handle incoming vote request
    pub fn handle_vote_request(&mut self, msg: &ConsensusMessage) -> Option<ConsensusMessage> {
        if let ConsensusMessage::VoteRequest { term, candidate_id, .. } = msg {
            if *term > self.current_term {
                self.current_term = *term;
                self.state = RaftState::Follower;
                self.voted_for = None;
                self.votes_received.clear();
                self.leader_id = None;
            }

            if *term < self.current_term {
                return Some(ConsensusMessage::VoteResponse {
                    term: self.current_term,
                    voter_id: self.local_id,
                    vote_granted: false,
                });
            }

            let grant = self.voted_for.is_none() || self.voted_for == Some(*candidate_id);
            if grant {
                self.voted_for = Some(*candidate_id);
            }

            Some(ConsensusMessage::VoteResponse {
                term: self.current_term,
                voter_id: self.local_id,
                vote_granted: grant,
            })
        } else {
            None
        }
    }

    /// Handle incoming vote response. Returns true if we just won the election.
    pub fn handle_vote_response(&mut self, msg: &ConsensusMessage) -> bool {
        if let ConsensusMessage::VoteResponse { term, voter_id, vote_granted } = msg {
            if self.state != RaftState::Candidate || *term != self.current_term {
                return false;
            }
            if *vote_granted && !self.votes_received.contains(voter_id) {
                self.votes_received.push(*voter_id);
            }
            let majority = self.validator_count / 2 + 1;
            if self.votes_received.len() >= majority {
                self.state = RaftState::Leader;
                self.leader_id = Some(self.local_id);
                return true;
            }
        }
        false
    }

    /// Handle leader heartbeat with explicit timestamp
    pub fn handle_heartbeat_at(&mut self, msg: &ConsensusMessage, now_ms: u64) {
        if let ConsensusMessage::RaftHeartbeat { term, leader_id, .. } = msg {
            if *term >= self.current_term {
                self.current_term = *term;
                self.state = RaftState::Follower;
                self.leader_id = Some(*leader_id);
                self.voted_for = None;
                self.votes_received.clear();
                self.last_heartbeat = now_ms;
                self.randomize_timeout();
            }
        }
    }

    pub fn state(&self) -> RaftState {
        self.state
    }

    pub fn is_leader(&self) -> bool {
        self.state == RaftState::Leader
    }

    pub fn current_leader(&self) -> Option<[u8; 32]> {
        self.leader_id
    }

    pub fn current_term(&self) -> u64 {
        self.current_term
    }

    fn start_election(&mut self, now_ms: u64) -> Vec<ConsensusMessage> {
        self.current_term += 1;
        self.state = RaftState::Candidate;
        self.voted_for = Some(self.local_id);
        self.votes_received = vec![self.local_id];
        self.last_heartbeat = now_ms;
        self.randomize_timeout();

        // Check if single-node cluster
        let majority = self.validator_count / 2 + 1;
        if self.votes_received.len() >= majority {
            self.state = RaftState::Leader;
            self.leader_id = Some(self.local_id);
            return vec![];
        }

        vec![ConsensusMessage::VoteRequest {
            term: self.current_term,
            candidate_id: self.local_id,
            last_block_height: 0,
            last_block_hash: [0u8; 32],
        }]
    }

    fn randomize_timeout(&mut self) {
        self.election_timeout_ms = rand::thread_rng().gen_range(2000..5000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn test_starts_inactive() {
        let rf = RaftFallback::new(make_id(1), 5);
        assert_eq!(rf.state(), RaftState::Inactive);
    }

    #[test]
    fn test_activate_becomes_follower() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);
        assert_eq!(rf.state(), RaftState::Follower);
    }

    #[test]
    fn test_deactivate_becomes_inactive() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);
        rf.deactivate();
        assert_eq!(rf.state(), RaftState::Inactive);
    }

    #[test]
    fn test_election_timeout_triggers_candidate() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);
        // Tick far into the future to guarantee timeout
        let msgs = rf.tick(1000 + 10000);
        assert_eq!(rf.state(), RaftState::Candidate);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ConsensusMessage::VoteRequest { term, candidate_id, .. } => {
                assert_eq!(*term, 1);
                assert_eq!(*candidate_id, make_id(1));
            }
            _ => panic!("Expected VoteRequest"),
        }
    }

    #[test]
    fn test_wins_election_with_majority() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);
        let _ = rf.tick(1000 + 10000); // become candidate, term=1, self-vote counted

        // Need 3 votes total (majority of 5). Already have self-vote.
        let won = rf.handle_vote_response(&ConsensusMessage::VoteResponse {
            term: 1,
            voter_id: make_id(2),
            vote_granted: true,
        });
        assert!(!won);

        let won = rf.handle_vote_response(&ConsensusMessage::VoteResponse {
            term: 1,
            voter_id: make_id(3),
            vote_granted: true,
        });
        assert!(won);
        assert!(rf.is_leader());
        assert_eq!(rf.current_leader(), Some(make_id(1)));
    }

    #[test]
    fn test_vote_only_once_per_term() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);

        // Vote for candidate 2
        let resp1 = rf.handle_vote_request(&ConsensusMessage::VoteRequest {
            term: 1,
            candidate_id: make_id(2),
            last_block_height: 0,
            last_block_hash: [0; 32],
        });
        assert!(matches!(resp1, Some(ConsensusMessage::VoteResponse { vote_granted: true, .. })));

        // Try to vote for candidate 3 in same term — should be denied
        let resp2 = rf.handle_vote_request(&ConsensusMessage::VoteRequest {
            term: 1,
            candidate_id: make_id(3),
            last_block_height: 0,
            last_block_hash: [0; 32],
        });
        assert!(matches!(resp2, Some(ConsensusMessage::VoteResponse { vote_granted: false, .. })));
    }

    #[test]
    fn test_higher_term_resets_to_follower() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);
        let _ = rf.tick(1000 + 10000); // become candidate at term 1
        assert_eq!(rf.state(), RaftState::Candidate);

        // Receive vote request with higher term
        let resp = rf.handle_vote_request(&ConsensusMessage::VoteRequest {
            term: 5,
            candidate_id: make_id(2),
            last_block_height: 0,
            last_block_hash: [0; 32],
        });
        assert_eq!(rf.state(), RaftState::Follower);
        assert_eq!(rf.current_term(), 5);
        assert!(matches!(resp, Some(ConsensusMessage::VoteResponse { vote_granted: true, .. })));
    }

    #[test]
    fn test_heartbeat_resets_timeout() {
        let mut rf = RaftFallback::new(make_id(1), 5);
        rf.activate(1000);

        // Heartbeat at 3000ms
        rf.handle_heartbeat_at(&ConsensusMessage::RaftHeartbeat {
            term: 1,
            leader_id: make_id(2),
            latest_height: 0,
        }, 3000);

        // Tick at 5000ms — should NOT timeout (only 2000ms since heartbeat, timeout is 2000-5000)
        // Use 4999 to be safe (less than minimum timeout of 2000)
        let msgs = rf.tick(4999);
        assert_eq!(rf.state(), RaftState::Follower);
        assert!(msgs.is_empty());
    }
}
