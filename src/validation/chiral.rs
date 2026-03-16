/// Chiral stability scoring for SCADA data batches.
///
/// Ports the QuantumOS chiral stability equation: `stability = |η/γ|`
/// where η measures disorder (entropy-like) and γ measures structure.
/// Natural industrial sensor data has characteristic η/γ ratios that
/// distinguish it from synthetic or tampered signals.

use std::fmt;

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StabilityClass {
    Excellent,
    Good,
    Marginal,
    Unstable,
}

impl fmt::Display for StabilityClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Excellent => write!(f, "EXCELLENT"),
            Self::Good => write!(f, "GOOD"),
            Self::Marginal => write!(f, "MARGINAL"),
            Self::Unstable => write!(f, "UNSTABLE"),
        }
    }
}

// ---------------------------------------------------------------------------
// Scoring result
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ChiralScore {
    pub stability: f64,
    pub eta: f64,
    pub gamma: f64,
    pub class: StabilityClass,
}

impl ChiralScore {
    pub fn is_rejected(&self) -> bool {
        self.class == StabilityClass::Unstable
    }

    pub fn needs_review(&self) -> bool {
        self.class == StabilityClass::Marginal
    }
}

// ---------------------------------------------------------------------------
// Thresholds — natural SCADA typically lands in [0.4, 1.2]
// ---------------------------------------------------------------------------

const EXCELLENT_LOW: f64 = 0.4;
const EXCELLENT_HIGH: f64 = 1.2;
const GOOD_LOW: f64 = 0.2;
const GOOD_HIGH: f64 = 2.0;
const MARGINAL_LOW: f64 = 0.1;
const MARGINAL_HIGH: f64 = 3.0;

fn classify(stability: f64) -> StabilityClass {
    if stability >= EXCELLENT_LOW && stability <= EXCELLENT_HIGH {
        StabilityClass::Excellent
    } else if stability >= GOOD_LOW && stability <= GOOD_HIGH {
        StabilityClass::Good
    } else if stability >= MARGINAL_LOW && stability <= MARGINAL_HIGH {
        StabilityClass::Marginal
    } else {
        StabilityClass::Unstable
    }
}

// ---------------------------------------------------------------------------
// Statistical helpers
// ---------------------------------------------------------------------------

fn mean(data: &[f64]) -> f64 {
    if data.is_empty() { return 0.0; }
    data.iter().sum::<f64>() / data.len() as f64
}

fn variance(data: &[f64]) -> f64 {
    if data.len() < 2 { return 0.0; }
    let m = mean(data);
    data.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (data.len() - 1) as f64
}

/// Lag-1 autocorrelation.
fn autocorrelation(data: &[f64]) -> f64 {
    if data.len() < 3 { return 0.0; }
    let m = mean(data);
    let var: f64 = data.iter().map(|x| (x - m).powi(2)).sum();
    if var == 0.0 { return 0.0; }
    let cov: f64 = (0..data.len() - 1)
        .map(|i| (data[i] - m) * (data[i + 1] - m))
        .sum();
    cov / var
}

/// Coefficient of variation of second differences.
/// Natural signals have irregular second derivatives; linear ramps have zero.
fn second_diff_cv(data: &[f64]) -> f64 {
    if data.len() < 3 { return 0.0; }
    let diffs: Vec<f64> = (0..data.len() - 2)
        .map(|i| data[i + 2] - 2.0 * data[i + 1] + data[i])
        .collect();
    let m = mean(&diffs);
    let v = variance(&diffs);
    if m.abs() < f64::EPSILON {
        // Mean near zero — use raw stddev as measure
        v.sqrt()
    } else {
        v.sqrt() / m.abs()
    }
}

/// Shannon entropy via histogram binning, normalised to [0,1].
fn entropy_normalised(data: &[f64]) -> f64 {
    if data.len() < 2 { return 0.0; }
    let min = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON { return 0.0; }

    let num_bins = (data.len() as f64).sqrt().ceil().max(2.0) as usize;
    let bin_width = (max - min) / num_bins as f64;
    let mut counts = vec![0usize; num_bins];
    for &v in data {
        let idx = ((v - min) / bin_width).floor() as usize;
        counts[idx.min(num_bins - 1)] += 1;
    }

    let n = data.len() as f64;
    let max_entropy = (num_bins as f64).ln();
    if max_entropy == 0.0 { return 0.0; }
    let h: f64 = counts.iter()
        .filter(|&&c| c > 0)
        .map(|&c| { let p = c as f64 / n; -p * p.ln() })
        .sum();
    h / max_entropy
}

// ---------------------------------------------------------------------------
// ChiralValidator
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ChiralValidator;

impl ChiralValidator {
    pub fn new() -> Self { Self }

    /// Score a single channel of sensor readings.
    pub fn score(&self, data: &[f64]) -> ChiralScore {
        if data.len() < 4 || data.iter().any(|x| !x.is_finite()) {
            return ChiralScore { stability: 0.0, eta: 0.0, gamma: 0.0, class: StabilityClass::Unstable };
        }

        // η (disorder): entropy + coefficient of variation
        let ent = entropy_normalised(data);
        let var = variance(data);
        let m = mean(data);
        let cv = if m.abs() > f64::EPSILON { var.sqrt() / m.abs() } else { var.sqrt() };
        let eta = 0.6 * ent + 0.4 * (cv.min(2.0) / 2.0);

        // γ (structure): autocorrelation weighted by second-difference roughness.
        // Natural SCADA: high autocorrelation + irregular second differences → high γ
        // Linear ramp: high autocorrelation + zero second differences → low γ
        // Random noise: low autocorrelation → low γ
        let ac = autocorrelation(data).abs().max(f64::EPSILON);
        let roughness = second_diff_cv(data);
        // Normalise roughness: natural SCADA has roughness >> 0; ramps ≈ 0
        let rough_norm = (roughness.min(5.0) / 5.0).max(0.001);
        let gamma = (ac * rough_norm).max(f64::EPSILON);

        let stability = (eta / gamma).abs();
        let class = classify(stability);

        ChiralScore { stability, eta, gamma, class }
    }

    /// Score multiple sensor channels. Aggregate = worst channel.
    pub fn score_batch(&self, channels: &[&[f64]]) -> (Vec<ChiralScore>, StabilityClass) {
        if channels.is_empty() {
            return (vec![], StabilityClass::Unstable);
        }
        let scores: Vec<ChiralScore> = channels.iter().map(|ch| self.score(ch)).collect();
        let worst = scores.iter()
            .map(|s| s.class)
            .max_by_key(|c| match c {
                StabilityClass::Excellent => 0,
                StabilityClass::Good => 1,
                StabilityClass::Marginal => 2,
                StabilityClass::Unstable => 3,
            })
            .unwrap_or(StabilityClass::Unstable);
        (scores, worst)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(state: &mut u64) -> f64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((*state >> 33) as f64 / (u32::MAX as f64 / 2.0)) - 1.0
    }

    /// Natural SCADA: slow random walk with mean reversion + noise.
    fn natural_scada_data(n: usize) -> Vec<f64> {
        let mut s: u64 = 42;
        let mut value = 100.0;
        (0..n).map(|_| {
            value += 0.3 * (100.0 - value) * 0.01 + 0.5 * lcg(&mut s);
            value
        }).collect()
    }

    fn synthetic_ordered(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 0.1).collect()
    }

    fn random_noise(n: usize) -> Vec<f64> {
        let mut s: u64 = 99;
        (0..n).map(|_| { lcg(&mut s) * 100.0 }).collect()
    }

    #[test]
    fn natural_data_scores_excellent() {
        let v = ChiralValidator::new();
        let data = natural_scada_data(200);
        let score = v.score(&data);
        println!("Natural: stability={:.4} η={:.4} γ={:.4} class={}", score.stability, score.eta, score.gamma, score.class);
        assert!(
            score.class == StabilityClass::Excellent || score.class == StabilityClass::Good,
            "Natural SCADA should be EXCELLENT/GOOD, got {}", score.class
        );
        assert!(!score.is_rejected());
    }

    #[test]
    fn synthetic_ordered_flagged() {
        let v = ChiralValidator::new();
        let data = synthetic_ordered(200);
        let score = v.score(&data);
        println!("Ordered: stability={:.4} η={:.4} γ={:.4} class={}", score.stability, score.eta, score.gamma, score.class);
        assert!(score.class != StabilityClass::Excellent, "Synthetic ordered should not be EXCELLENT, got {}", score.class);
    }

    #[test]
    fn random_noise_flagged() {
        let v = ChiralValidator::new();
        let data = random_noise(200);
        let score = v.score(&data);
        println!("Noise: stability={:.4} η={:.4} γ={:.4} class={}", score.stability, score.eta, score.gamma, score.class);
        assert!(score.class != StabilityClass::Excellent, "Random noise should not be EXCELLENT, got {}", score.class);
    }

    #[test]
    fn unstable_is_rejected() {
        let s = ChiralScore { stability: 5.0, eta: 5.0, gamma: 1.0, class: StabilityClass::Unstable };
        assert!(s.is_rejected());
        assert!(!s.needs_review());
    }

    #[test]
    fn marginal_needs_review() {
        let s = ChiralScore { stability: 2.5, eta: 2.5, gamma: 1.0, class: StabilityClass::Marginal };
        assert!(!s.is_rejected());
        assert!(s.needs_review());
    }

    #[test]
    fn too_few_samples_is_unstable() {
        let v = ChiralValidator::new();
        assert_eq!(v.score(&[1.0, 2.0]).class, StabilityClass::Unstable);
    }

    #[test]
    fn batch_scoring_worst_wins() {
        let v = ChiralValidator::new();
        let natural = natural_scada_data(200);
        let noise = random_noise(200);
        let (scores, worst) = v.score_batch(&[&natural, &noise]);
        assert_eq!(scores.len(), 2);
        assert!(worst != StabilityClass::Excellent);
    }

    #[test]
    fn nan_input_is_unstable() {
        let v = ChiralValidator::new();
        let data = vec![1.0, 2.0, f64::NAN, 4.0, 5.0];
        assert_eq!(v.score(&data).class, StabilityClass::Unstable);
    }

    #[test]
    fn infinity_input_is_unstable() {
        let v = ChiralValidator::new();
        let data = vec![1.0, 2.0, f64::INFINITY, 4.0, 5.0];
        assert_eq!(v.score(&data).class, StabilityClass::Unstable);
    }

    #[test]
    fn constant_signal_flagged() {
        let v = ChiralValidator::new();
        let data = vec![42.0; 200];
        let score = v.score(&data);
        println!("Constant: stability={:.4} η={:.4} γ={:.4} class={}", score.stability, score.eta, score.gamma, score.class);
        assert!(score.class != StabilityClass::Excellent);
    }
}
