# ADR-0001: Resonant Consensus

**Status:** Accepted
**Date:** 2026-03-15
**Authors:** Nick Flach, Kannaka

## Context

0xSCADA needs a consensus mechanism for a permissioned blockchain that audits industrial SCADA systems. The network is small (3–4 validators), resource-constrained (1 ARM OCPU / 6GB RAM per node), and prioritizes auditability over throughput.

Existing consensus mechanisms are poor fits:

- **PBFT** requires 3f+1 nodes. With 3 nodes, f=0 — no fault tolerance at all.
- **Pure Raft** assumes crash faults only. A compromised SCADA validator could be Byzantine, not just offline.
- **Proof-of-Work/Stake** are designed for open networks with economic incentives. We have neither unknown participants nor tokens.
- **Rigid heartbeat timers** (Raft, Tendermint) are brittle under the clock drift and network jitter common in industrial environments with heterogeneous hardware.

Additionally, standard consensus provides binary block validity (accept/reject) with no concept of data quality — critical for industrial compliance where "technically valid but suspicious" data patterns need to be flagged.

## Decision

We adopt **Resonant Consensus**, a novel mechanism combining three components:

### 1. Kuramoto Oscillator Phase Coupling

Each validator is modeled as a coupled oscillator with phase θᵢ evolving according to the Kuramoto model:

```
dθᵢ/dt = ωᵢ + (K/N)Σⱼsin(θⱼ - θᵢ) + ηᵢ(t)
```

Where:
- **ωᵢ** is the node's natural frequency (calibrated to block production rate)
- **K** is the coupling constant (how strongly nodes synchronize)
- **ηᵢ(t)** is a chiral noise term preventing over-synchronization
- The **order parameter** r = |1/N Σⱼ e^(iθⱼ)| measures network coherence

Leader selection is implicit: the node whose phase is closest to the mean phase ψ proposes the next block. This rotates naturally without elections. When r drops below 0.5 (loss of coherence), the system falls back to explicit Raft-style voting.

The order parameter r serves double duty as a real-time network health metric:
- r > 0.9 — healthy, fast finality
- r ∈ [0.5, 0.9] — degraded, auto-increase coupling K
- r < 0.5 — critical, engage Raft fallback
- r → 1.0 sustained — inject chiral perturbation to prevent ossification

### 2. Chiral Block Validation

Derived from the stability equation `dx/dt = f(x) - Iηx`, validators compute a stability score for each block using the interference-to-damping ratio:

```
stability = |η/γ|
```

Classification:
- |η/γ| < 0.3 → EXCELLENT — normal SCADA data patterns
- |η/γ| < 0.6 → GOOD — acceptable variance
- |η/γ| < 1.0 → MARGINAL — flagged for review
- |η/γ| ≥ 1.0 → UNSTABLE — block rejected

This catches both fabricated data (too orderly) and sensor failures (too chaotic). Real industrial telemetry has characteristic interference patterns; deviations in either direction are suspicious.

### 3. Berry Phase Audit Trail

Geometric phase tracking provides a topological invariant over the audit chain:

```
γ = ∮ A(θ)·dθ
```

As SCADA events accumulate, the chain's state traces a path through a parameter manifold. The Berry phase is path-dependent but gauge-invariant — it proves that a sequence of events is consistent regardless of how individual blocks are replayed or reordered. This is a compliance primitive: cryptographic proof of event ordering integrity that no amount of block manipulation can forge without detection.

## Consequences

### What This Enables

- **Graceful degradation** — consensus quality degrades smoothly with network issues rather than failing catastrophically at a quorum threshold
- **Self-tuning** — coupling constant K adapts to network conditions automatically
- **Data quality awareness** — blocks carry stability metadata beyond binary validity, enabling compliance officers to audit data quality over time
- **Tamper-evident audit trails** — Berry phase provides a topological proof of event sequence integrity that survives block reorganization attempts
- **Small network viability** — works with 3 validators while still providing Byzantine fault detection (not tolerance at n=3, but detection)

### What This Costs

- **Novel mechanism** — no battle-tested production deployments to reference; requires careful testing
- **Mathematical complexity** — operators need to understand order parameter semantics for monitoring, though defaults work without tuning
- **Limited academic precedent** — Kuramoto models are well-studied in physics but not in distributed consensus; this is genuinely new territory

### Comparison to Alternatives

| Property | PBFT | Raft | Resonant |
|----------|------|------|----------|
| Min nodes for fault tolerance | 4 (f=1) | 3 (crash only) | 3 (detection) |
| Byzantine support | Yes | No | Detection + fallback |
| Leader election | View change | Term election | Phase rotation |
| Network health metric | None | Leader heartbeat | Order parameter r |
| Data quality scoring | No | No | Chiral stability |
| Audit integrity proof | No | No | Berry phase |
| Clock sensitivity | Moderate | High (timeouts) | Low (phase-coupled) |

### Raft Fallback

When resonant consensus cannot maintain coherence (r < 0.5), the system falls back to a simplified Raft protocol for leader election and log replication. This ensures liveness even under conditions where the Kuramoto model cannot converge. The fallback is automatic and transparent — the system returns to resonant mode when coherence recovers.

## References

- Kuramoto, Y. (1975). Self-entrainment of a population of coupled non-linear oscillators.
- Berry, M.V. (1984). Quantal phase factors accompanying adiabatic changes.
- QuantumOS `chiral_resources.c` — stability classification via interference-damping ratio
- ghostmagicOS core equation: `dx/dt = f(x) - Iηx`
