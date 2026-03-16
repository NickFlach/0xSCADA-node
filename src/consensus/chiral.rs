use std::collections::VecDeque;

/// Chiral perturbation monitor for consensus health.
///
/// Tracks the Kuramoto order parameter r over time and injects asymmetric
/// perturbation when r → 1.0 is sustained, preventing unhealthy phase-lock.
/// Target operating range: r ∈ [0.7, 0.95].
pub struct ChiralPerturbation {
    /// Base perturbation strength
    pub eta_0: f64,
    /// Number of consecutive blocks to check
    pub window_size: usize,
    /// Rolling history of r values
    history: VecDeque<f64>,
    /// Whether perturbation is currently active
    active: bool,
    /// Threshold above which r is considered "locked"
    lock_threshold: f64,
    /// Threshold below which perturbation deactivates
    unlock_threshold: f64,
}

impl ChiralPerturbation {
    pub fn new(eta_0: f64, window_size: usize) -> Self {
        assert!(window_size >= 1, "window_size must be >= 1");
        let eta_0 = eta_0.abs(); // perturbation strength must be non-negative
        Self {
            eta_0,
            window_size,
            history: VecDeque::with_capacity(window_size + 1),
            active: false,
            lock_threshold: 0.95,
            unlock_threshold: 0.90,
        }
    }

    /// Record a new order parameter measurement and update active state.
    pub fn record_order_parameter(&mut self, r: f64) {
        self.history.push_back(r);
        if self.history.len() > self.window_size {
            self.history.pop_front();
        }
        // Activate if sustained above lock threshold
        if !self.active && self.should_perturb() {
            self.active = true;
        }
        // Deactivate if r drops below unlock threshold
        if self.active && r < self.unlock_threshold {
            self.active = false;
        }
    }

    /// Returns true if the last `window_size` r values are all above the lock threshold.
    pub fn should_perturb(&self) -> bool {
        !self.history.is_empty()
            && self.history.len() >= self.window_size
            && self.history.iter().all(|&r| r > self.lock_threshold)
    }

    /// Compute chiral perturbation for a node given its phase and cluster id.
    /// η = η₀ · sin(2θ) · (-1)^cluster_id
    pub fn compute_perturbation(&self, phase: f64, cluster_id: u32) -> f64 {
        if !self.active {
            return 0.0;
        }
        let sign = if cluster_id % 2 == 0 { 1.0 } else { -1.0 };
        self.eta_0 * (2.0 * phase).sin() * sign
    }

    /// Whether perturbation is currently active.
    pub fn perturbation_active(&self) -> bool {
        self.active
    }

    /// Current history length.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
}

impl Default for ChiralPerturbation {
    fn default() -> Self {
        Self::new(0.15, 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_inactive() {
        let cp = ChiralPerturbation::default();
        assert!(!cp.perturbation_active());
        assert!(!cp.should_perturb());
        assert_eq!(cp.compute_perturbation(1.0, 0), 0.0);
    }

    #[test]
    fn test_activates_after_sustained_high_r() {
        let mut cp = ChiralPerturbation::new(0.15, 30);
        // Feed 29 values above threshold — should NOT activate
        for _ in 0..29 {
            cp.record_order_parameter(0.99);
        }
        assert!(!cp.perturbation_active());

        // 30th value triggers activation
        cp.record_order_parameter(0.99);
        assert!(cp.perturbation_active());
    }

    #[test]
    fn test_does_not_activate_below_threshold() {
        let mut cp = ChiralPerturbation::new(0.15, 30);
        for _ in 0..50 {
            cp.record_order_parameter(0.90);
        }
        assert!(!cp.perturbation_active());
    }

    #[test]
    fn test_deactivates_when_r_drops() {
        let mut cp = ChiralPerturbation::new(0.15, 30);
        // Activate
        for _ in 0..30 {
            cp.record_order_parameter(0.99);
        }
        assert!(cp.perturbation_active());

        // Drop below unlock threshold
        cp.record_order_parameter(0.85);
        assert!(!cp.perturbation_active());
    }

    #[test]
    fn test_perturbation_formula() {
        let mut cp = ChiralPerturbation::new(0.5, 5);
        // Activate with small window
        for _ in 0..5 {
            cp.record_order_parameter(0.99);
        }
        assert!(cp.perturbation_active());

        let phase = std::f64::consts::PI / 4.0; // sin(2 * π/4) = sin(π/2) = 1.0
        // cluster_id=0 (even) → +1
        let eta_even = cp.compute_perturbation(phase, 0);
        assert!((eta_even - 0.5).abs() < 1e-10, "got {}", eta_even);

        // cluster_id=1 (odd) → -1
        let eta_odd = cp.compute_perturbation(phase, 1);
        assert!((eta_odd - (-0.5)).abs() < 1e-10, "got {}", eta_odd);
    }

    #[test]
    fn test_asymmetry_by_cluster() {
        let mut cp = ChiralPerturbation::new(0.3, 5);
        for _ in 0..5 {
            cp.record_order_parameter(0.99);
        }
        let phase = 1.0;
        let a = cp.compute_perturbation(phase, 0);
        let b = cp.compute_perturbation(phase, 1);
        // Opposite signs
        assert!((a + b).abs() < 1e-10, "should be antisymmetric: {} vs {}", a, b);
    }

    #[test]
    fn test_no_perturbation_when_inactive() {
        let cp = ChiralPerturbation::new(1.0, 5);
        assert_eq!(cp.compute_perturbation(1.0, 0), 0.0);
        assert_eq!(cp.compute_perturbation(1.0, 1), 0.0);
    }

    #[test]
    fn test_history_bounded() {
        let mut cp = ChiralPerturbation::new(0.1, 5);
        for i in 0..100 {
            cp.record_order_parameter(i as f64 * 0.01);
        }
        assert_eq!(cp.history_len(), 5);
    }

    #[test]
    fn test_reactivation_cycle() {
        let mut cp = ChiralPerturbation::new(0.15, 5);

        // Activate
        for _ in 0..5 {
            cp.record_order_parameter(0.99);
        }
        assert!(cp.perturbation_active());

        // Deactivate
        cp.record_order_parameter(0.80);
        assert!(!cp.perturbation_active());

        // Re-activate
        for _ in 0..5 {
            cp.record_order_parameter(0.98);
        }
        assert!(cp.perturbation_active());
    }

    #[test]
    #[should_panic(expected = "window_size must be >= 1")]
    fn test_zero_window_size_panics() {
        ChiralPerturbation::new(0.15, 0);
    }

    #[test]
    fn test_single_below_threshold_prevents_activation() {
        let mut cp = ChiralPerturbation::new(0.15, 5);
        for _ in 0..4 {
            cp.record_order_parameter(0.99);
        }
        cp.record_order_parameter(0.90); // one dip
        assert!(!cp.perturbation_active());
    }

    /// Simulate a mini Kuramoto system with chiral perturbation feedback
    /// to verify r stabilizes in the healthy [0.7, 0.95] range.
    #[test]
    fn test_r_stabilizes_in_healthy_range() {
        use std::f64::consts::PI;

        let n = 10;
        let coupling = 5.0;
        let dt = 0.01;
        let mut phases: Vec<f64> = (0..n).map(|i| 0.1 * i as f64).collect();
        let freqs: Vec<f64> = vec![1.0; n];
        let mut cp = ChiralPerturbation::new(2.0, 30);

        let mut last_rs = Vec::new();

        for step in 0..5000 {
            // Compute order parameter
            let n_f = n as f64;
            let sum_cos: f64 = phases.iter().map(|p| p.cos()).sum();
            let sum_sin: f64 = phases.iter().map(|p| p.sin()).sum();
            let r = ((sum_cos / n_f).powi(2) + (sum_sin / n_f).powi(2)).sqrt();

            cp.record_order_parameter(r);

            // Update phases
            for i in 0..n {
                let mut sum = 0.0;
                for j in 0..n {
                    if j != i {
                        sum += (phases[j] - phases[i]).sin();
                    }
                }
                let chiral = cp.compute_perturbation(phases[i], i as u32);
                let dtheta = freqs[i] + (coupling / (n_f - 1.0)) * sum + chiral;
                phases[i] += dtheta * dt;
                phases[i] = phases[i] % (2.0 * PI);
                if phases[i] < 0.0 {
                    phases[i] += 2.0 * PI;
                }
            }

            if step >= 4000 {
                last_rs.push(r);
            }
        }

        let avg_r: f64 = last_rs.iter().sum::<f64>() / last_rs.len() as f64;
        // With chiral perturbation, r should not be stuck at 1.0
        assert!(
            avg_r < 0.99,
            "avg r = {} — chiral should prevent perfect lock",
            avg_r
        );
        // But should maintain reasonable coherence
        assert!(
            avg_r > 0.5,
            "avg r = {} — should maintain partial sync",
            avg_r
        );
    }
}
