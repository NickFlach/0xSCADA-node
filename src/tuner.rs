use std::collections::VecDeque;

/// Generic sliding window buffer for storing the last N items of any type.
/// 
/// When the capacity is reached, new items push out the oldest items in FIFO order.
/// This is used to maintain a rolling window of consensus metrics for analysis.
#[derive(Clone, Debug)]
pub struct SlidingWindow<T> {
    buffer: VecDeque<T>,
    capacity: usize,
}

impl<T> SlidingWindow<T> {
    /// Create a new sliding window with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new item into the window. If at capacity, removes the oldest item.
    pub fn push(&mut self, item: T) {
        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(item);
    }

    /// Get the current number of items in the window.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if the window is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Check if the window is at full capacity.
    pub fn is_full(&self) -> bool {
        self.buffer.len() == self.capacity
    }

    /// Iterate over items in the window (oldest to newest).
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.buffer.iter()
    }

    /// Clear all items from the window.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Get the capacity of the window.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Metrics collected from a single consensus tick.
/// 
/// These capture the state of the Kuramoto oscillator consensus at a specific moment,
/// allowing the adaptive tuner to analyze patterns and adjust parameters accordingly.
#[derive(Clone, Debug)]
pub struct ConsensusMetrics {
    /// Order parameter (synchronization measure) - ranges from 0.0 (chaos) to 1.0 (perfect sync)
    pub r: f64,
    /// Mean phase angle across all nodes in the network
    pub mean_phase: f64,
    /// This node's current phase angle
    pub local_phase: f64,
    /// ID of the current consensus leader (32-byte identifier)
    pub leader_id: [u8; 32],
    /// Whether our last proposal was successfully attested/finalized
    pub attestation_success: bool,
    /// Number of peers we can see in the network
    pub peer_count: usize,
    /// Current blockchain height
    pub block_height: u64,
    /// Timestamp when these metrics were recorded
    pub timestamp_ms: u64,
}

impl ConsensusMetrics {
    /// Create new consensus metrics with all required fields.
    pub fn new(
        r: f64,
        mean_phase: f64,
        local_phase: f64,
        leader_id: [u8; 32],
        attestation_success: bool,
        peer_count: usize,
        block_height: u64,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            r,
            mean_phase,
            local_phase,
            leader_id,
            attestation_success,
            peer_count,
            block_height,
            timestamp_ms,
        }
    }
}

/// Configuration for the adaptive tuning system.
/// 
/// Controls when and how the consensus parameters are adjusted based on observed metrics.
#[derive(Clone, Debug)]
pub struct TuningConfig {
    /// Whether adaptive tuning is enabled
    pub enabled: bool,
    /// Number of metrics to keep in the sliding window
    pub window_size: usize,
    /// How many blocks between parameter adjustment attempts
    pub adjustment_interval: usize,
    /// Minimum allowed coupling strength
    pub min_coupling_k: f64,
    /// Maximum allowed coupling strength
    pub max_coupling_k: f64,
    /// Minimum allowed chiral perturbation
    pub min_chiral_eta: f64,
    /// Maximum allowed chiral perturbation
    pub max_chiral_eta: f64,
    /// Target minimum order parameter (below this triggers coupling increase)
    pub target_r_low: f64,
    /// Target maximum order parameter (above this triggers coupling decrease)
    pub target_r_high: f64,
    /// Target minimum leader tenure in blocks (below this decreases eta)
    pub target_leader_tenure_low: usize,
    /// Target maximum leader tenure in blocks (above this increases eta)
    pub target_leader_tenure_high: usize,
    /// Rate of parameter adjustment (0.05 = 5% change per adjustment)
    pub adjustment_rate: f64,
}

impl Default for TuningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_size: 100,
            adjustment_interval: 50,
            min_coupling_k: 0.1,
            max_coupling_k: 5.0,
            min_chiral_eta: 0.0,
            max_chiral_eta: 0.5,
            target_r_low: 0.7,
            target_r_high: 0.9,
            target_leader_tenure_low: 10,
            target_leader_tenure_high: 50,
            adjustment_rate: 0.05,
        }
    }
}

/// Represents a parameter adjustment that should be applied to the consensus engine.
#[derive(Clone, Debug, PartialEq)]
pub enum ParameterAdjustment {
    /// Adjust only the coupling strength
    CouplingK(f64),
    /// Adjust only the chiral perturbation
    ChiralEta(f64),
    /// Adjust both parameters simultaneously
    Both { coupling_k: f64, chiral_eta: f64 },
}

/// Record of a parameter adjustment for logging and analysis.
#[derive(Clone, Debug)]
pub struct AdjustmentRecord {
    /// Block height when adjustment was made
    pub block_height: u64,
    /// Name of the parameter that was adjusted
    pub param: String,
    /// Previous value of the parameter
    pub old_value: f64,
    /// New value of the parameter
    pub new_value: f64,
    /// Human-readable reason for the adjustment
    pub reason: String,
    /// Mean order parameter before the adjustment
    pub mean_r_before: f64,
}

/// The adaptive consensus parameter tuner.
/// 
/// This implements the core self-optimization logic, analyzing consensus metrics
/// over time and adjusting Kuramoto oscillator parameters to maintain resonance.
pub struct AdaptiveTuner {
    config: TuningConfig,
    window: SlidingWindow<ConsensusMetrics>,
    current_coupling_k: f64,
    current_chiral_eta: f64,
    blocks_since_adjustment: usize,
    adjustment_history: Vec<AdjustmentRecord>,
}

impl AdaptiveTuner {
    /// Create a new adaptive tuner with the given configuration and initial parameters.
    pub fn new(config: TuningConfig, initial_coupling_k: f64, initial_chiral_eta: f64) -> Self {
        let window = SlidingWindow::new(config.window_size);
        
        Self {
            config,
            window,
            current_coupling_k: initial_coupling_k,
            current_chiral_eta: initial_chiral_eta,
            blocks_since_adjustment: 0,
            adjustment_history: Vec::new(),
        }
    }

    /// Record new consensus metrics and increment the block counter.
    pub fn record(&mut self, metrics: ConsensusMetrics) {
        self.window.push(metrics);
        self.blocks_since_adjustment += 1;
    }

    /// Check if parameters should be adjusted based on accumulated metrics.
    /// 
    /// Returns Some(ParameterAdjustment) if changes are needed, None otherwise.
    pub fn maybe_adjust(&mut self) -> Option<ParameterAdjustment> {
        if !self.config.enabled {
            return None;
        }

        // Only adjust every adjustment_interval blocks
        if self.blocks_since_adjustment < self.config.adjustment_interval {
            return None;
        }

        // Need enough metrics to make a decision
        if self.window.len() < self.config.adjustment_interval / 2 {
            return None;
        }

        // Get the current block height from the latest metric
        let current_height = self.window.iter().last()
            .map(|m| m.block_height)
            .unwrap_or(0);

        // Analyze order parameter
        let mean_r = self.compute_mean_r();
        let mut coupling_adjustment = None;
        
        if mean_r < self.config.target_r_low {
            // Low synchronization - increase coupling
            let new_coupling = (self.current_coupling_k * (1.0 + self.config.adjustment_rate))
                .min(self.config.max_coupling_k);
            
            if (new_coupling - self.current_coupling_k).abs() > 1e-6 {
                self.adjustment_history.push(AdjustmentRecord {
                    block_height: current_height,
                    param: "coupling_k".to_string(),
                    old_value: self.current_coupling_k,
                    new_value: new_coupling,
                    reason: format!("Low order parameter (r={})", mean_r),
                    mean_r_before: mean_r,
                });
                
                self.current_coupling_k = new_coupling;
                coupling_adjustment = Some(new_coupling);
            }
        } else if mean_r > self.config.target_r_high {
            // Over-synchronization - decrease coupling
            let new_coupling = (self.current_coupling_k * (1.0 - self.config.adjustment_rate))
                .max(self.config.min_coupling_k);
            
            if (new_coupling - self.current_coupling_k).abs() > 1e-6 {
                self.adjustment_history.push(AdjustmentRecord {
                    block_height: current_height,
                    param: "coupling_k".to_string(),
                    old_value: self.current_coupling_k,
                    new_value: new_coupling,
                    reason: format!("High order parameter (r={})", mean_r),
                    mean_r_before: mean_r,
                });
                
                self.current_coupling_k = new_coupling;
                coupling_adjustment = Some(new_coupling);
            }
        }

        // Analyze leader tenure
        let avg_tenure = self.compute_average_leader_tenure();
        let mut eta_adjustment = None;
        
        if let Some(tenure) = avg_tenure {
            if tenure < self.config.target_leader_tenure_low as f64 {
                // Too much leader churn - decrease eta (reduce chaos)
                let new_eta = (self.current_chiral_eta * (1.0 - self.config.adjustment_rate))
                    .max(self.config.min_chiral_eta);
                
                if (new_eta - self.current_chiral_eta).abs() > 1e-6 {
                    self.adjustment_history.push(AdjustmentRecord {
                        block_height: current_height,
                        param: "chiral_eta".to_string(),
                        old_value: self.current_chiral_eta,
                        new_value: new_eta,
                        reason: format!("Rapid leader changes (tenure={})", tenure),
                        mean_r_before: mean_r,
                    });
                    
                    self.current_chiral_eta = new_eta;
                    eta_adjustment = Some(new_eta);
                }
            } else if tenure > self.config.target_leader_tenure_high as f64 {
                // Leader too stable - increase eta (add chaos)
                let new_eta = (self.current_chiral_eta * (1.0 + self.config.adjustment_rate))
                    .min(self.config.max_chiral_eta);
                
                if (new_eta - self.current_chiral_eta).abs() > 1e-6 {
                    self.adjustment_history.push(AdjustmentRecord {
                        block_height: current_height,
                        param: "chiral_eta".to_string(),
                        old_value: self.current_chiral_eta,
                        new_value: new_eta,
                        reason: format!("Stuck leader (tenure={})", tenure),
                        mean_r_before: mean_r,
                    });
                    
                    self.current_chiral_eta = new_eta;
                    eta_adjustment = Some(new_eta);
                }
            }
        }

        // Reset the adjustment counter
        self.blocks_since_adjustment = 0;

        // Return the appropriate adjustment
        match (coupling_adjustment, eta_adjustment) {
            (Some(k), Some(eta)) => Some(ParameterAdjustment::Both { coupling_k: k, chiral_eta: eta }),
            (Some(k), None) => Some(ParameterAdjustment::CouplingK(k)),
            (None, Some(eta)) => Some(ParameterAdjustment::ChiralEta(eta)),
            (None, None) => None,
        }
    }

    /// Get the adjustment history for logging and analysis.
    pub fn history(&self) -> &[AdjustmentRecord] {
        &self.adjustment_history
    }

    /// Get the current parameter values.
    pub fn current_params(&self) -> (f64, f64) {
        (self.current_coupling_k, self.current_chiral_eta)
    }

    /// Compute the mean order parameter over the current window.
    fn compute_mean_r(&self) -> f64 {
        if self.window.is_empty() {
            return 0.0;
        }

        let sum: f64 = self.window.iter().map(|m| m.r).sum();
        sum / self.window.len() as f64
    }

    /// Compute the average leader tenure (blocks per leader) in the current window.
    /// 
    /// Returns None if there's insufficient data to compute tenure.
    fn compute_average_leader_tenure(&self) -> Option<f64> {
        if self.window.len() < 2 {
            return None;
        }

        let metrics: Vec<_> = self.window.iter().collect();
        let mut leader_changes = 0;
        let mut current_leader = &metrics[0].leader_id;
        
        for metric in &metrics[1..] {
            if metric.leader_id != *current_leader {
                leader_changes += 1;
                current_leader = &metric.leader_id;
            }
        }

        if leader_changes == 0 {
            // Same leader throughout - tenure is the entire window
            Some(metrics.len() as f64)
        } else {
            // Average tenure = total blocks / number of leaders
            // Number of leaders = leader_changes + 1
            Some(metrics.len() as f64 / (leader_changes + 1) as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sliding_window_basic_operations() {
        let mut window = SlidingWindow::<i32>::new(3);
        
        assert_eq!(window.len(), 0);
        assert!(window.is_empty());
        assert!(!window.is_full());
        
        window.push(1);
        assert_eq!(window.len(), 1);
        assert!(!window.is_empty());
        assert!(!window.is_full());
        
        window.push(2);
        window.push(3);
        assert_eq!(window.len(), 3);
        assert!(window.is_full());
        
        let values: Vec<_> = window.iter().cloned().collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn test_sliding_window_capacity_overflow() {
        let mut window = SlidingWindow::<i32>::new(2);
        
        window.push(1);
        window.push(2);
        assert_eq!(window.len(), 2);
        assert!(window.is_full());
        
        // This should remove the oldest (1) and add 3
        window.push(3);
        assert_eq!(window.len(), 2);
        
        let values: Vec<_> = window.iter().cloned().collect();
        assert_eq!(values, vec![2, 3]);
        
        window.push(4);
        let values: Vec<_> = window.iter().cloned().collect();
        assert_eq!(values, vec![3, 4]);
    }

    #[test]
    fn test_sliding_window_clear() {
        let mut window = SlidingWindow::<i32>::new(3);
        window.push(1);
        window.push(2);
        
        assert_eq!(window.len(), 2);
        
        window.clear();
        assert_eq!(window.len(), 0);
        assert!(window.is_empty());
        assert!(!window.is_full());
    }

    #[test]
    fn test_consensus_metrics_construction() {
        let leader_id = [1u8; 32];
        let metrics = ConsensusMetrics::new(
            0.85,
            1.5,
            1.4,
            leader_id,
            true,
            5,
            100,
            1234567890,
        );
        
        assert_eq!(metrics.r, 0.85);
        assert_eq!(metrics.mean_phase, 1.5);
        assert_eq!(metrics.local_phase, 1.4);
        assert_eq!(metrics.leader_id, leader_id);
        assert!(metrics.attestation_success);
        assert_eq!(metrics.peer_count, 5);
        assert_eq!(metrics.block_height, 100);
        assert_eq!(metrics.timestamp_ms, 1234567890);
    }

    #[test]
    fn test_tuning_config_default() {
        let config = TuningConfig::default();
        
        assert!(config.enabled);
        assert_eq!(config.window_size, 100);
        assert_eq!(config.adjustment_interval, 50);
        assert_eq!(config.min_coupling_k, 0.1);
        assert_eq!(config.max_coupling_k, 5.0);
        assert_eq!(config.min_chiral_eta, 0.0);
        assert_eq!(config.max_chiral_eta, 0.5);
        assert_eq!(config.target_r_low, 0.7);
        assert_eq!(config.target_r_high, 0.9);
        assert_eq!(config.target_leader_tenure_low, 10);
        assert_eq!(config.target_leader_tenure_high, 50);
        assert_eq!(config.adjustment_rate, 0.05);
    }

    #[test]
    fn test_adaptive_tuner_low_r_increases_coupling() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_r_low: 0.7,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.1);
        
        // Add 50 metrics with low r (0.3)
        for i in 0..50 {
            let metrics = create_test_metrics(0.3, [1u8; 32], i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::CouplingK(new_k)) => {
                assert!(new_k > 1.0);
                assert_eq!(new_k, 1.05); // 1.0 * 1.05
            }
            Some(ParameterAdjustment::Both { coupling_k, .. }) => {
                assert!(coupling_k > 1.0);
                assert_eq!(coupling_k, 1.05);
            }
            _ => panic!("Expected coupling adjustment for low r"),
        }
    }

    #[test]
    fn test_adaptive_tuner_high_r_decreases_coupling() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_r_high: 0.9,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.1);
        
        // Add 50 metrics with high r (0.98)
        for i in 0..50 {
            let metrics = create_test_metrics(0.98, [1u8; 32], i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::CouplingK(new_k)) => {
                assert!(new_k < 1.0);
                assert_eq!(new_k, 0.95); // 1.0 * 0.95
            }
            Some(ParameterAdjustment::Both { coupling_k, .. }) => {
                assert!(coupling_k < 1.0);
                assert_eq!(coupling_k, 0.95);
            }
            _ => panic!("Expected coupling adjustment for high r"),
        }
    }

    #[test]
    fn test_adaptive_tuner_normal_r_no_adjustment() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_r_low: 0.7,
            target_r_high: 0.9,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.1);
        
        // Add 50 metrics with normal r (0.8)
        for i in 0..50 {
            let metrics = create_test_metrics(0.8, [1u8; 32], i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        // Should be no adjustment for normal r
        match adjustment {
            Some(ParameterAdjustment::ChiralEta(_)) => {
                // Might adjust eta due to leader tenure, but not coupling
            }
            Some(ParameterAdjustment::Both { coupling_k, .. }) => {
                // If both adjusted, coupling should be unchanged
                assert_eq!(coupling_k, 1.0);
            }
            None => {
                // Expected if leader tenure is also normal
            }
            Some(ParameterAdjustment::CouplingK(_)) => {
                panic!("Should not adjust coupling for normal r");
            }
        }
    }

    #[test]
    fn test_adaptive_tuner_respects_coupling_bounds() {
        let config = TuningConfig {
            adjustment_interval: 50,
            min_coupling_k: 0.1,
            max_coupling_k: 5.0,
            target_r_low: 0.7,
            adjustment_rate: 0.5, // Large adjustment rate for testing
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 4.9, 0.1);
        
        // Add 50 metrics with low r to trigger increase
        for i in 0..50 {
            let metrics = create_test_metrics(0.3, [1u8; 32], i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::CouplingK(new_k)) => {
                assert_eq!(new_k, 5.0); // Clamped to max
            }
            Some(ParameterAdjustment::Both { coupling_k, .. }) => {
                assert_eq!(coupling_k, 5.0); // Clamped to max
            }
            _ => panic!("Expected coupling adjustment"),
        }
    }

    #[test]
    fn test_adaptive_tuner_records_history() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_r_low: 0.7,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.1);
        
        // Add 50 metrics with low r
        for i in 0..50 {
            let metrics = create_test_metrics(0.3, [1u8; 32], i);
            tuner.record(metrics);
        }
        
        tuner.maybe_adjust();
        
        let history = tuner.history();
        assert_eq!(history.len(), 1);
        
        let record = &history[0];
        assert_eq!(record.param, "coupling_k");
        assert_eq!(record.old_value, 1.0);
        assert_eq!(record.new_value, 1.05);
        assert!(record.reason.contains("Low order parameter"));
        assert_eq!(record.mean_r_before, 0.3);
    }

    #[test]
    fn test_adaptive_tuner_rapid_leader_changes_decreases_eta() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_leader_tenure_low: 10,
            target_r_low: 0.0, // Don't trigger coupling adjustment
            target_r_high: 1.0,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.2);
        
        // Add metrics with leader changing every block
        for i in 0..50 {
            let leader_id = [i as u8; 32];
            let metrics = create_test_metrics(0.8, leader_id, i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::ChiralEta(new_eta)) => {
                assert!(new_eta < 0.2);
                assert_eq!(new_eta, 0.19); // 0.2 * 0.95
            }
            Some(ParameterAdjustment::Both { chiral_eta, .. }) => {
                assert!(chiral_eta < 0.2);
                assert_eq!(chiral_eta, 0.19);
            }
            _ => panic!("Expected eta adjustment for rapid leader changes"),
        }
    }

    #[test]
    fn test_adaptive_tuner_stuck_leader_increases_eta() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_leader_tenure_high: 50,
            target_r_low: 0.0, // Don't trigger coupling adjustment
            target_r_high: 1.0,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.2);
        
        // Add metrics with same leader throughout (100 blocks > 50 target)
        let stuck_leader = [42u8; 32];
        for i in 0..100 {
            let metrics = create_test_metrics(0.8, stuck_leader, i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::ChiralEta(new_eta)) => {
                assert!(new_eta > 0.2);
                assert_eq!(new_eta, 0.21); // 0.2 * 1.05
            }
            Some(ParameterAdjustment::Both { chiral_eta, .. }) => {
                assert!(chiral_eta > 0.2);
                assert_eq!(chiral_eta, 0.21);
            }
            _ => panic!("Expected eta adjustment for stuck leader"),
        }
    }

    #[test]
    fn test_adaptive_tuner_both_params_adjust_simultaneously() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_r_low: 0.7,
            target_leader_tenure_high: 50,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.2);
        
        // Add metrics with low r AND stuck leader
        let stuck_leader = [42u8; 32];
        for i in 0..100 {
            let metrics = create_test_metrics(0.3, stuck_leader, i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        match adjustment {
            Some(ParameterAdjustment::Both { coupling_k, chiral_eta }) => {
                assert_eq!(coupling_k, 1.05); // Increased for low r
                assert_eq!(chiral_eta, 0.21); // Increased for stuck leader
            }
            _ => panic!("Expected both parameters to adjust"),
        }
    }

    #[test]
    fn test_adaptive_tuner_current_params() {
        let config = TuningConfig::default();
        let tuner = AdaptiveTuner::new(config, 1.5, 0.3);
        
        let (coupling, eta) = tuner.current_params();
        assert_eq!(coupling, 1.5);
        assert_eq!(eta, 0.3);
    }

    #[test]
    fn test_adaptive_tuner_normal_leader_rotation_no_eta_adjustment() {
        let config = TuningConfig {
            adjustment_interval: 50,
            target_leader_tenure_low: 10,
            target_leader_tenure_high: 50,
            target_r_low: 0.0, // Don't trigger coupling adjustment
            target_r_high: 1.0,
            ..TuningConfig::default()
        };
        
        let mut tuner = AdaptiveTuner::new(config, 1.0, 0.2);
        
        // Add metrics with leader changing every 20 blocks (normal rotation)
        for i in 0..50 {
            let leader_id = [(i / 20) as u8; 32];
            let metrics = create_test_metrics(0.8, leader_id, i);
            tuner.record(metrics);
        }
        
        let adjustment = tuner.maybe_adjust();
        
        // Should be no adjustment for normal leader rotation
        assert!(adjustment.is_none());
    }

    // Helper function to create test metrics
    fn create_test_metrics(r: f64, leader_id: [u8; 32], block_height: u64) -> ConsensusMetrics {
        ConsensusMetrics::new(
            r,
            1.0,
            1.0,
            leader_id,
            true,
            3,
            block_height,
            block_height * 1000,
        )
    }
}