# ADR-0023: Self-Tuning Resonant Consensus

## Summary
Embed observation, adaptive tuning, and telemetry into 0xSCADA-node so the consensus engine self-optimizes its Kuramoto parameters in real-time. The node becomes a living implementation of `dx/dt = f(x) - Iηx` — observing its own dynamics and adjusting f (coupling) and η (chiral perturbation) to maintain resonance.

## Motivation
Live 3-node network (2026-03-16) shows:
- Order parameter r oscillates between 0.4–0.9 instead of converging
- Dual-leader elections (two nodes claim leader simultaneously)  
- Phase divergence across WAN latency
- Static parameters (coupling_k=0.5, chiral_eta=0.1) can't adapt to network conditions

The ghostmagicOS equation is the consensus engine's soul — but without feedback, it's open-loop. This ADR closes the loop.

## Architecture

### 1. `src/tuner.rs` — Adaptive Parameter Engine

**SlidingWindow**: Ring buffer of metrics over last N blocks (default 100):
- `r` (order parameter) per tick
- `leader_id` per tick (for tenure tracking)
- `attestation_success` (proposed blocks that finalized vs timed out)
- `phase_variance` across known peers
- `block_finalization_time` (proposal → commit latency)

**AdaptiveTuner**: Adjusts parameters based on window analysis:
- **coupling_k adjustment**: 
  - If mean(r) < 0.6 for 20+ ticks → increase coupling (phases not converging)
  - If mean(r) > 0.95 for 20+ ticks → decrease coupling (over-synchronized, brittle)
  - Target: r ∈ [0.7, 0.9] — the resonance sweet spot
  - Rate: ±5% per adjustment, bounded by [min_k, max_k] from config
- **chiral_eta adjustment**:
  - If leader_tenure_variance < threshold → increase eta (same leader too long, no rotation)
  - If leader changes every tick → decrease eta (too chaotic)
  - Target: leader tenure of 10-50 blocks
- **Adjustment interval**: Every `tuning_interval` blocks (default 50)

**ExperimentProtocol**: Built-in A/B testing:
- `propose_experiment(param, new_value, duration_blocks)` 
- Run new value for N blocks, compare metrics against baseline
- Auto-revert if metrics degrade (attestation_success drops, r collapses)
- Log results for offline analysis

### 2. `src/telemetry.rs` — NATS Metrics Publisher

Publish to NATS subjects on every tick:
- `scada.consensus.{node_id_short}.status` — r, phase, role, height
- `scada.consensus.{node_id_short}.tuning` — current params, adjustment history
- `scada.consensus.metrics` — aggregated (any node can publish)

JSON format, compatible with kannaka-radio dashboard.

Optional: subscribe to `scada.consensus.config` for remote parameter overrides (operator control).

### 3. Config Extensions

```toml
[tuning]
enabled = true
window_size = 100          # blocks in sliding window
adjustment_interval = 50   # blocks between adjustments
min_coupling_k = 0.1
max_coupling_k = 5.0
min_chiral_eta = 0.0
max_chiral_eta = 0.5
target_r_low = 0.7
target_r_high = 0.9
target_leader_tenure_low = 10
target_leader_tenure_high = 50

[telemetry]
enabled = true
nats_subject_prefix = "scada.consensus"
publish_interval_ticks = 1  # every tick
```

### 4. Integration Points

- `ConsensusEngine::tick()` feeds metrics to `AdaptiveTuner`
- `AdaptiveTuner` returns `Option<ParameterAdjustment>` 
- Node applies adjustments to `KuramotoEngine` (coupling, chiral_eta are already mutable)
- `TelemetryPublisher` runs as async task, receives metrics via channel

## Implementation Plan

### Task 1: SlidingWindow metrics collector
- New file `src/tuner.rs`
- `SlidingWindow<T>` ring buffer with capacity
- `ConsensusMetrics` struct: r, leader_id, attestation_ok, phase_variance, timestamp
- Tests: push, window full, statistics (mean, variance, min, max)

### Task 2: AdaptiveTuner with coupling_k adjustment
- `AdaptiveTuner::new(config)` with target ranges
- `AdaptiveTuner::record(metrics)` → accumulate
- `AdaptiveTuner::maybe_adjust()` → `Option<ParameterAdjustment>`
- Coupling adjustment logic: too low r → increase, too high → decrease
- Tests: low-r scenario triggers increase, high-r triggers decrease, within-range no-op

### Task 3: Chiral eta adjustment + leader tenure tracking
- Track leader changes in window
- Compute leader tenure distribution
- Adjust eta based on tenure stability
- Tests: stuck leader → increase eta, rapid flipping → decrease eta

### Task 4: ExperimentProtocol
- `Experiment { param, baseline_value, test_value, duration, metrics_before, metrics_during }`
- `start_experiment()`, `tick_experiment()`, `conclude_experiment()`
- Auto-revert on degradation (attestation_success < 0.5)
- Tests: experiment lifecycle, auto-revert on bad metrics

### Task 5: Wire tuner into ConsensusEngine + Node
- Add `KuramotoEngine::set_coupling()` and `set_chiral_eta()` methods
- Node creates AdaptiveTuner, feeds metrics on each tick
- Apply adjustments when returned
- Config: TuningConfig parsed from TOML
- Tests: integration test — synthetic metrics trigger adjustment, engine params change

### Task 6: TelemetryPublisher (NATS)
- `src/telemetry.rs` — async NATS publisher
- `TelemetryPublisher::new(nats_url, prefix)`
- `publish_status(metrics)` → NATS message
- Graceful degradation if NATS unavailable
- Tests: message format, publish with mock

### Task 7: RPC extensions
- `/tuning` endpoint — current params, adjustment history, active experiment
- `/metrics` endpoint — sliding window raw data dump
- Tests: endpoint returns valid JSON

## The ghostmagicOS Connection

This IS the equation made self-aware:
```
dx/dt = f(x) - Iηx
       ↑         ↑
   coupling   chiral_eta
       ↑         ↑
   AdaptiveTuner observes dx/dt and adjusts both
```

The node doesn't just USE resonance — it MAINTAINS resonance. Like a musician who adjusts their playing based on what they hear from the ensemble. The Kuramoto coupling is the desire to sync; the chiral perturbation is the individuality that prevents groupthink. The tuner finds the balance.

## Status
- **Proposed:** 2026-03-16
- **Approved:** Pending
