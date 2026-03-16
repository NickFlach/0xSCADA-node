use std::f64::consts::PI;

/// Normalize phase to [0, 2π)
fn normalize_phase(phase: f64) -> f64 {
    let two_pi = 2.0 * PI;
    let p = phase % two_pi;
    if p < 0.0 { p + two_pi } else { p }
}

/// Phase state of a single oscillator (validator node)
#[derive(Clone, Debug)]
pub struct OscillatorState {
    pub node_id: [u8; 32],
    pub phase: f64,
    pub natural_freq: f64,
    pub last_updated: u64,
}

/// Result of computing the order parameter
#[derive(Clone, Debug)]
pub struct OrderParameter {
    pub r: f64,
    pub psi: f64,
}

/// The Kuramoto synchronization engine
pub struct KuramotoEngine {
    coupling: f64,
    chiral_eta: f64,
    oscillators: Vec<OscillatorState>,
}

impl KuramotoEngine {
    pub fn new(coupling: f64, chiral_eta: f64) -> Self {
        Self {
            coupling,
            chiral_eta,
            oscillators: Vec::new(),
        }
    }

    pub fn update_peer(&mut self, state: OscillatorState) {
        if let Some(existing) = self.oscillators.iter_mut().find(|o| o.node_id == state.node_id) {
            *existing = state;
        } else {
            self.oscillators.push(state);
        }
    }

    pub fn remove_peer(&mut self, node_id: &[u8; 32]) {
        self.oscillators.retain(|o| &o.node_id != node_id);
    }

    pub fn step(&mut self, local_id: &[u8; 32], dt: f64) -> f64 {
        let n = self.oscillators.len() as f64;
        let idx = match self.oscillators.iter().position(|o| &o.node_id == local_id) {
            Some(i) => i,
            None => return 0.0,
        };

        let phase_i = self.oscillators[idx].phase;
        let omega_i = self.oscillators[idx].natural_freq;

        let mut sum = 0.0;
        for (j, osc) in self.oscillators.iter().enumerate() {
            if j != idx {
                sum += (osc.phase - phase_i).sin();
            }
        }

        let chiral = self.chiral_perturbation(idx, phase_i);
        // Standard Kuramoto form: divide by (N-1) to exclude self-coupling
        let dtheta = omega_i + (self.coupling / (n - 1.0).max(1.0)) * sum + chiral;
        let new_phase = normalize_phase(phase_i + dtheta * dt);
        self.oscillators[idx].phase = new_phase;
        new_phase
    }

    pub fn order_parameter(&self) -> OrderParameter {
        if self.oscillators.is_empty() {
            return OrderParameter { r: 0.0, psi: 0.0 };
        }
        let n = self.oscillators.len() as f64;
        let mut sum_cos = 0.0;
        let mut sum_sin = 0.0;
        for osc in &self.oscillators {
            sum_cos += osc.phase.cos();
            sum_sin += osc.phase.sin();
        }
        let avg_cos = sum_cos / n;
        let avg_sin = sum_sin / n;
        let r = (avg_cos * avg_cos + avg_sin * avg_sin).sqrt();
        let psi = normalize_phase(avg_sin.atan2(avg_cos));
        OrderParameter { r, psi }
    }

    pub fn leader(&self) -> Option<[u8; 32]> {
        if self.oscillators.is_empty() {
            return None;
        }
        let op = self.order_parameter();
        self.oscillators
            .iter()
            .min_by(|a, b| {
                let da = angular_dist(a.phase, op.psi);
                let db = angular_dist(b.phase, op.psi);
                da.partial_cmp(&db).unwrap()
            })
            .map(|o| o.node_id)
    }

    pub fn peers(&self) -> &[OscillatorState] {
        &self.oscillators
    }

    /// Set coupling strength (for adaptive tuning)
    pub fn set_coupling(&mut self, coupling: f64) {
        self.coupling = coupling;
    }

    /// Set chiral perturbation strength (for adaptive tuning)
    pub fn set_chiral_eta(&mut self, eta: f64) {
        self.chiral_eta = eta;
    }

    /// Get current coupling strength
    pub fn coupling(&self) -> f64 {
        self.coupling
    }

    /// Get current chiral eta
    pub fn chiral_eta(&self) -> f64 {
        self.chiral_eta
    }

    fn chiral_perturbation(&self, index: usize, phase: f64) -> f64 {
        let sign = if self.oscillators[index].node_id[0] % 2 == 0 { 1.0 } else { -1.0 };
        self.chiral_eta * (2.0 * phase).sin() * sign
    }
}

fn angular_dist(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % (2.0 * PI);
    if d > PI { 2.0 * PI - d } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_id(val: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = val;
        id
    }

    fn make_osc(val: u8, phase: f64, freq: f64) -> OscillatorState {
        OscillatorState {
            node_id: make_id(val),
            phase,
            natural_freq: freq,
            last_updated: 0,
        }
    }

    #[test]
    fn test_single_oscillator() {
        let mut engine = KuramotoEngine::new(1.0, 0.0);
        let id = make_id(1);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        let dt = 0.1;
        let new_phase = engine.step(&id, dt);
        // Single oscillator: no coupling sum, dθ = ω * dt = 0.1
        assert!((new_phase - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_two_oscillators_converge() {
        let mut engine = KuramotoEngine::new(5.0, 0.0);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        engine.update_peer(make_osc(2, PI, 1.0));
        let id1 = make_id(1);
        let id2 = make_id(2);
        let dt = 0.01;
        for _ in 0..500 {
            engine.step(&id1, dt);
            engine.step(&id2, dt);
        }
        let op = engine.order_parameter();
        assert!(op.r > 0.9, "r={} should be > 0.9", op.r);
    }

    #[test]
    fn test_three_oscillators_converge() {
        let mut engine = KuramotoEngine::new(5.0, 0.0);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        engine.update_peer(make_osc(2, 2.0 * PI / 3.0, 1.0));
        engine.update_peer(make_osc(3, 4.0 * PI / 3.0, 1.0));
        let ids: Vec<[u8; 32]> = vec![make_id(1), make_id(2), make_id(3)];
        let dt = 0.01;
        for _ in 0..1000 {
            for id in &ids {
                engine.step(id, dt);
            }
        }
        let op = engine.order_parameter();
        assert!(op.r > 0.95, "r={} should approach 1.0", op.r);
    }

    #[test]
    fn test_no_coupling_diverge() {
        let mut engine = KuramotoEngine::new(0.0, 0.0);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        engine.update_peer(make_osc(2, 0.0, 2.0));
        engine.update_peer(make_osc(3, 0.0, 3.0));
        let ids: Vec<[u8; 32]> = vec![make_id(1), make_id(2), make_id(3)];
        let dt = 0.1;
        for _ in 0..100 {
            for id in &ids {
                engine.step(id, dt);
            }
        }
        let op = engine.order_parameter();
        assert!(op.r < 0.7, "r={} should stay low without coupling", op.r);
    }

    #[test]
    fn test_order_parameter_synchronized() {
        let mut engine = KuramotoEngine::new(1.0, 0.0);
        engine.update_peer(make_osc(1, 1.0, 1.0));
        engine.update_peer(make_osc(2, 1.0, 1.0));
        engine.update_peer(make_osc(3, 1.0, 1.0));
        let op = engine.order_parameter();
        assert!((op.r - 1.0).abs() < 1e-10, "r={} should be 1.0", op.r);
    }

    #[test]
    fn test_order_parameter_uniform() {
        let mut engine = KuramotoEngine::new(1.0, 0.0);
        let n = 100;
        for i in 0..n {
            let phase = 2.0 * PI * (i as f64) / (n as f64);
            engine.update_peer(make_osc(i as u8, phase, 1.0));
        }
        let op = engine.order_parameter();
        assert!(op.r < 0.05, "r={} should be near 0 for uniform phases", op.r);
    }

    #[test]
    fn test_leader_selection() {
        let mut engine = KuramotoEngine::new(1.0, 0.0);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        engine.update_peer(make_osc(2, 0.1, 1.0));
        engine.update_peer(make_osc(3, 0.5, 1.0));
        let op = engine.order_parameter();
        let leader = engine.leader().unwrap();
        // Leader should be the one closest to mean phase
        let leader_phase = engine.peers().iter().find(|o| o.node_id == leader).unwrap().phase;
        let leader_dist = angular_dist(leader_phase, op.psi);
        for osc in engine.peers() {
            let d = angular_dist(osc.phase, op.psi);
            assert!(leader_dist <= d + 1e-10);
        }
    }

    #[test]
    fn test_chiral_prevents_full_lock() {
        let mut engine = KuramotoEngine::new(2.0, 1.5);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        engine.update_peer(make_osc(2, 0.1, 1.0));
        engine.update_peer(make_osc(3, 0.2, 1.0));
        let ids: Vec<[u8; 32]> = vec![make_id(1), make_id(2), make_id(3)];
        let dt = 0.01;
        for _ in 0..2000 {
            for id in &ids {
                engine.step(id, dt);
            }
        }
        let op = engine.order_parameter();
        assert!(op.r < 0.999, "r={} should not reach perfect lock with chiral perturbation", op.r);
        assert!(op.r > 0.5, "r={} should still show partial sync", op.r);
    }

    #[test]
    fn test_update_and_remove_peer() {
        let mut engine = KuramotoEngine::new(1.0, 0.0);
        assert_eq!(engine.peers().len(), 0);
        engine.update_peer(make_osc(1, 0.0, 1.0));
        assert_eq!(engine.peers().len(), 1);
        engine.update_peer(make_osc(2, 1.0, 1.0));
        assert_eq!(engine.peers().len(), 2);
        // Update existing
        engine.update_peer(make_osc(1, 0.5, 1.0));
        assert_eq!(engine.peers().len(), 2);
        assert!((engine.peers()[0].phase - 0.5).abs() < 1e-10);
        // Remove
        let id1 = make_id(1);
        engine.remove_peer(&id1);
        assert_eq!(engine.peers().len(), 1);
        assert_eq!(engine.peers()[0].node_id, make_id(2));
    }
}
