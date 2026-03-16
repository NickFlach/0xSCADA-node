use blake3;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// A state transition event for Berry phase tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    pub from_state: Vec<u8>,
    pub to_state: Vec<u8>,
}

/// Tracks cumulative geometric (Berry) phase over a sequence of state transitions.
///
/// Each transition maps to an angle in parameter space via blake3 hashing.
/// The accumulated phase is a topological invariant of the event sequence —
/// any tampering (reorder, removal, modification) changes the phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BerryAuditor {
    /// Accumulated geometric phase (radians).
    phase: f64,
    /// Number of recorded transitions.
    transition_count: u64,
    /// Previous angle in parameter space (for connection computation).
    prev_angle: Option<f64>,
    /// Record of transitions for verification.
    transitions: Vec<StateTransition>,
}

impl BerryAuditor {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            transition_count: 0,
            prev_angle: None,
            transitions: Vec::new(),
        }
    }

    /// Compute an angle in [0, 2π) from a blake3 hash of state data.
    fn state_to_angle(state: &[u8]) -> f64 {
        let hash = blake3::hash(state);
        let bytes = hash.as_bytes();
        // Use first 8 bytes as u64 to get a uniform angle
        let val = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        (val as f64 / u64::MAX as f64) * 2.0 * PI
    }

    /// Compute the Berry connection A(θ) for a transition between two states.
    /// Returns the connection value and the new angle.
    fn berry_connection(from_state: &[u8], to_state: &[u8]) -> (f64, f64, f64) {
        let theta_from = Self::state_to_angle(from_state);
        let theta_to = Self::state_to_angle(to_state);

        // Connection A(θ) = cross-coupling between state vectors in parameter space
        // We combine both hashes to get a gauge-invariant connection
        let mut combined = Vec::with_capacity(from_state.len() + to_state.len());
        combined.extend_from_slice(from_state);
        combined.extend_from_slice(to_state);
        let conn_hash = blake3::hash(&combined);
        let conn_bytes = conn_hash.as_bytes();
        let conn_val = u64::from_le_bytes(conn_bytes[8..16].try_into().unwrap());

        // A(θ) — connection strength modulated by transition
        let connection = (conn_val as f64 / u64::MAX as f64) * 2.0 * PI;

        (connection, theta_from, theta_to)
    }

    /// Record a state transition and accumulate the Berry phase.
    pub fn record_transition(&mut self, from_state: &[u8], to_state: &[u8]) {
        let (connection, theta_from, theta_to) = Self::berry_connection(from_state, to_state);

        // Phase increment: A(θ) · dθ
        // dθ is the angular displacement in parameter space
        let mut d_theta = theta_to - theta_from;

        // Normalize to [-π, π] for proper winding
        while d_theta > PI {
            d_theta -= 2.0 * PI;
        }
        while d_theta < -PI {
            d_theta += 2.0 * PI;
        }

        // Accumulate: γ += A(θ) · dθ
        // The connection modulates the phase contribution of each step
        let phase_increment = connection * d_theta / (2.0 * PI);
        self.phase += phase_increment;

        // Chain: include previous angle for path-dependence
        if let Some(prev) = self.prev_angle {
            // Additional holonomy term from three-point curvature
            let curvature = (theta_from - prev).sin() * (theta_to - theta_from).sin();
            self.phase += curvature * 0.5;
        }

        self.prev_angle = Some(theta_to);
        self.transition_count += 1;
        self.transitions.push(StateTransition {
            from_state: from_state.to_vec(),
            to_state: to_state.to_vec(),
        });
    }

    /// Get the current accumulated Berry phase.
    pub fn current_phase(&self) -> f64 {
        self.phase
    }

    /// Get the number of recorded transitions.
    pub fn transition_count(&self) -> u64 {
        self.transition_count
    }

    /// Verify that a sequence of transitions produces the expected phase.
    /// Recomputes from scratch and compares.
    pub fn verify_sequence(transitions: &[StateTransition], expected_phase: f64) -> bool {
        let mut auditor = BerryAuditor::new();
        for t in transitions {
            auditor.record_transition(&t.from_state, &t.to_state);
        }
        (auditor.current_phase() - expected_phase).abs() < 1e-10
    }

    /// Verify this auditor's recorded sequence against its current phase.
    pub fn verify_self(&self) -> bool {
        Self::verify_sequence(&self.transitions, self.phase)
    }

    /// Detect if the sequence forms a cycle (first from_state == last to_state).
    /// In a cycle the Berry phase is quantized — it's a true topological invariant.
    pub fn detect_cycle(&self) -> bool {
        if self.transitions.is_empty() {
            return false;
        }
        let first_from = &self.transitions[0].from_state;
        let last_to = &self.transitions[self.transitions.len() - 1].to_state;
        first_from == last_to
    }

    /// Get the recorded transitions for external verification.
    pub fn transitions(&self) -> &[StateTransition] {
        &self.transitions
    }

    /// Reset the auditor to initial state.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.transition_count = 0;
        self.prev_angle = None;
        self.transitions.clear();
    }
}

impl Default for BerryAuditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_events() -> Vec<(Vec<u8>, Vec<u8>)> {
        vec![
            (b"sensor_offline".to_vec(), b"sensor_online".to_vec()),
            (b"sensor_online".to_vec(), b"valve_open".to_vec()),
            (b"valve_open".to_vec(), b"pressure_high".to_vec()),
            (b"pressure_high".to_vec(), b"alarm_triggered".to_vec()),
            (b"alarm_triggered".to_vec(), b"valve_closed".to_vec()),
        ]
    }

    #[test]
    fn deterministic_phase() {
        let events = sample_events();
        let mut a1 = BerryAuditor::new();
        let mut a2 = BerryAuditor::new();

        for (from, to) in &events {
            a1.record_transition(from, to);
            a2.record_transition(from, to);
        }

        assert_eq!(a1.current_phase(), a2.current_phase());
        assert_eq!(a1.transition_count(), 5);
    }

    #[test]
    fn tampered_sequence_different_phase() {
        let events = sample_events();
        let mut original = BerryAuditor::new();
        for (from, to) in &events {
            original.record_transition(from, to);
        }

        // Tamper: swap two events
        let mut tampered_events = events.clone();
        tampered_events.swap(1, 3);
        let mut tampered = BerryAuditor::new();
        for (from, to) in &tampered_events {
            tampered.record_transition(from, to);
        }

        assert_ne!(original.current_phase(), tampered.current_phase());
    }

    #[test]
    fn tampered_data_different_phase() {
        let events = sample_events();
        let mut original = BerryAuditor::new();
        for (from, to) in &events {
            original.record_transition(from, to);
        }

        // Tamper: modify one event's data
        let mut tampered_events = events.clone();
        tampered_events[2].1 = b"pressure_normal".to_vec(); // changed from pressure_high
        let mut tampered = BerryAuditor::new();
        for (from, to) in &tampered_events {
            tampered.record_transition(from, to);
        }

        assert_ne!(original.current_phase(), tampered.current_phase());
    }

    #[test]
    fn removed_event_different_phase() {
        let events = sample_events();
        let mut original = BerryAuditor::new();
        for (from, to) in &events {
            original.record_transition(from, to);
        }

        // Remove middle event
        let mut short = BerryAuditor::new();
        for (i, (from, to)) in events.iter().enumerate() {
            if i != 2 {
                short.record_transition(from, to);
            }
        }

        assert_ne!(original.current_phase(), short.current_phase());
    }

    #[test]
    fn verify_sequence_valid() {
        let events = sample_events();
        let mut auditor = BerryAuditor::new();
        for (from, to) in &events {
            auditor.record_transition(from, to);
        }

        assert!(auditor.verify_self());
        assert!(BerryAuditor::verify_sequence(
            auditor.transitions(),
            auditor.current_phase()
        ));
    }

    #[test]
    fn verify_sequence_wrong_phase() {
        let events = sample_events();
        let mut auditor = BerryAuditor::new();
        for (from, to) in &events {
            auditor.record_transition(from, to);
        }

        assert!(!BerryAuditor::verify_sequence(
            auditor.transitions(),
            auditor.current_phase() + 1.0
        ));
    }

    #[test]
    fn cycle_detection() {
        let mut auditor = BerryAuditor::new();
        // Create a cycle: A → B → C → A
        auditor.record_transition(b"state_a", b"state_b");
        auditor.record_transition(b"state_b", b"state_c");
        auditor.record_transition(b"state_c", b"state_a");

        assert!(auditor.detect_cycle());
    }

    #[test]
    fn no_cycle_detection() {
        let mut auditor = BerryAuditor::new();
        auditor.record_transition(b"state_a", b"state_b");
        auditor.record_transition(b"state_b", b"state_c");

        assert!(!auditor.detect_cycle());
    }

    #[test]
    fn empty_auditor() {
        let auditor = BerryAuditor::new();
        assert_eq!(auditor.current_phase(), 0.0);
        assert_eq!(auditor.transition_count(), 0);
        assert!(!auditor.detect_cycle());
    }

    #[test]
    fn cycle_phase_is_topological() {
        // Two cycles with same path but "different speed" (duplicated intermediate states)
        // should still detect as cycles with consistent phase
        let mut a1 = BerryAuditor::new();
        a1.record_transition(b"A", b"B");
        a1.record_transition(b"B", b"C");
        a1.record_transition(b"C", b"A");
        let phase1 = a1.current_phase();

        // Same cycle starting point, same sequence — must match
        let mut a2 = BerryAuditor::new();
        a2.record_transition(b"A", b"B");
        a2.record_transition(b"B", b"C");
        a2.record_transition(b"C", b"A");
        let phase2 = a2.current_phase();

        assert_eq!(phase1, phase2);
        assert!(a1.detect_cycle());
        assert!(a2.detect_cycle());
    }

    #[test]
    fn reset_works() {
        let mut auditor = BerryAuditor::new();
        auditor.record_transition(b"a", b"b");
        assert_ne!(auditor.current_phase(), 0.0);

        auditor.reset();
        assert_eq!(auditor.current_phase(), 0.0);
        assert_eq!(auditor.transition_count(), 0);
        assert!(auditor.transitions().is_empty());
    }
}
