# 0xSCADA-node — Implementation Plan (Wave Approach)

**Date:** 2026-03-15
**Design:** [2026-03-15-node-architecture-design.md](2026-03-15-node-architecture-design.md)

## Development Approach

Build in **waves** (proven with 0xSCADA's 3 evolution waves + ADR-0022 constellation).
**Devil's advocate sub-agent** after each wave to stress-test decisions and find bugs.

```
Wave 0: Foundation (pure infrastructure)
Wave 1: Resonant Consensus (Kuramoto sync)
Wave 2: Network + Sync (QUIC transport)
Wave 3: Chiral Validation + Berry Audit (differentiators)
Wave 4: RPC + CLI + Integration Tests
Devil's Advocate review after each wave
```

---

## Wave 0: Foundation
> Pure infrastructure. Crypto, types, storage, block management. No research — just solid bones.

### Task 0.1: Project scaffold + crypto primitives
**Time:** 5 min | **TDD:** Yes

- `cargo init` with clean module structure
- `crypto` module: ed25519 keypair gen/sign/verify (ed25519-dalek), blake3 hashing
- Tests: roundtrip sign/verify, hash determinism, invalid sig rejection, keypair serialization

**Files:** `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `src/crypto.rs`
**Verify:** `cargo test`

---

### Task 0.2: Block and transaction types
**Time:** 5 min | **TDD:** Yes

- `Block`, `BlockHeader`, `Transaction`, `TransactionBody`, `Attestation` structs
- CBOR serialization (serde + ciborium)
- Block hashing (blake3 of CBOR-serialized header)
- Block signing, attestation creation/verification
- Tests: serialize/deserialize roundtrip, hash stability, attestation verify

**Files:** `src/block.rs`, `src/transaction.rs`
**Verify:** `cargo test`

---

### Task 0.3: Sparse Merkle Tree
**Time:** 5 min | **TDD:** Yes

- Sparse Merkle Tree with blake3
- Ops: insert, get, root hash, inclusion proof, exclusion proof, verify proof
- Tests: empty tree root, single/multi insert, proof verification, exclusion proof

**Files:** `src/merkle.rs`
**Verify:** `cargo test`

---

### Task 0.4: Storage layer (sled)
**Time:** 5 min | **TDD:** Yes

- `Storage` struct wrapping sled with typed trees
- Trees: blocks, block_hashes, state, tx_index, metadata
- Ops: store_block, get_block_by_height/hash, get_tx, latest_height, store_state_root
- Tests: store/retrieve, latest height, tx index lookup, persistence across reopen

**Files:** `src/storage.rs`
**Verify:** `cargo test`

---

### Task 0.5: Block manager + genesis
**Time:** 5 min | **TDD:** Yes

- `BlockManager` — validates blocks, applies to storage
- Genesis block creation with initial validator set
- Validation: hash chain, signature, attestation threshold (⌈n/2⌉+1), timestamp ordering
- Tx execution: AnchorBatch → update state tree; ValidatorUpdate → update validator set
- Tests: genesis, valid block acceptance, reject bad sig/wrong height/insufficient attestations

**Files:** `src/block_manager.rs`
**Verify:** `cargo test`

---

### 🔴 Devil's Advocate — Wave 0
Spawn adversarial sub-agent to:
- Review all Wave 0 code for correctness, edge cases, panics
- Attempt to craft invalid blocks that pass validation
- Check serialization determinism (same block → same bytes always)
- Verify storage crash safety assumptions

---

## Wave 1: Resonant Consensus
> The Kuramoto heart. Phase-coupled validators, natural leader rotation, order parameter health.

### Task 1.1: Kuramoto oscillator engine
**Time:** 5 min | **TDD:** Yes

- `Phase` struct: θ (phase), ω (natural frequency), last_updated
- `KuramotoEngine`: N oscillators, coupling K, step function
- `dθᵢ/dt = ωᵢ + (K/N)Σⱼsin(θⱼ - θᵢ) + ηᵢ(t)`
- Order parameter computation: r·e^(iψ) = (1/N)Σⱼe^(iθⱼ)
- Tests: 3 oscillators converge (r→1), phase diverges without coupling (K=0), order parameter accuracy

**Files:** `src/consensus/kuramoto.rs`, `src/consensus/mod.rs`
**Verify:** `cargo test`

---

### Task 1.2: Leader selection + block proposal
**Time:** 5 min | **TDD:** Yes

- Leader = node closest to mean phase ψ
- `ConsensusEngine`: tracks local phase + peer phases, determines role
- Block proposal: leader builds block from mempool, signs, broadcasts
- Attestation collection: track received attestations, commit when threshold met
- Tests: leader rotates as phases evolve, correct node identified as leader

**Files:** `src/consensus/engine.rs`
**Verify:** `cargo test`

---

### Task 1.3: Raft fallback + message types
**Time:** 5 min | **TDD:** Yes

- Fallback when r < 0.5: explicit VoteRequest/VoteResponse, term-based election
- Message enum: PhaseUpdate, BlockProposal, Attestation, VoteRequest, VoteResponse, Heartbeat
- CBOR serialization for all message types
- Tests: fallback triggers at low r, election produces leader, return to Kuramoto when r recovers

**Files:** `src/consensus/raft_fallback.rs`, `src/consensus/messages.rs`
**Verify:** `cargo test`

---

### Task 1.4: Mempool
**Time:** 5 min | **TDD:** Yes

- `Mempool`: accept transactions, dedup, size limit, drain for block building
- Priority: ValidatorUpdate > AnchorBatch > Extension
- Tests: add/drain, dedup, size limit enforcement, priority ordering

**Files:** `src/mempool.rs`
**Verify:** `cargo test`

---

### 🔴 Devil's Advocate — Wave 1
Spawn adversarial sub-agent to:
- Verify Kuramoto convergence with adversarial phase updates
- Test leader selection edge cases (tied phases, single node, rapid phase changes)
- Attempt to game leader selection by sending false phase updates
- Verify Raft fallback actually works when Kuramoto degrades
- Check for floating point determinism issues across platforms

---

## Wave 2: Network + Sync
> QUIC transport, peer mesh, block sync for late joiners.

### Task 2.1: QUIC transport layer
**Time:** 5 min | **TDD:** Yes

- `Transport` using quinn: listen, connect, accept connections
- Self-signed certs from ed25519 node identity
- Message framing: length-prefixed CBOR over bidirectional streams
- Tests: two-node connect, message roundtrip, connection refused handling

**Files:** `src/network/transport.rs`, `src/network/mod.rs`
**Verify:** `cargo test`

---

### Task 2.2: Peer manager
**Time:** 5 min | **TDD:** Yes

- `PeerManager`: connect to seed list, track connected peers, auto-reconnect
- Full mesh topology for validators
- Peer state: connected, disconnecting, reconnecting
- Tests: connect to seeds, handle peer disconnect, reconnect logic

**Files:** `src/network/peers.rs`
**Verify:** `cargo test`

---

### Task 2.3: Block sync protocol
**Time:** 5 min | **TDD:** Yes

- New node requests blocks from genesis (or checkpoint)
- Streaming sync: request range → receive blocks → validate → apply
- Catch-up detection: if local height < peer height by >10, enter sync mode
- Tests: node syncs from genesis, sync with gap, reject invalid blocks during sync

**Files:** `src/network/sync.rs`
**Verify:** `cargo test`

---

### Task 2.4: Node orchestrator
**Time:** 5 min | **TDD:** Yes

- `Node` struct: ties Transport + PeerManager + ConsensusEngine + BlockManager + Mempool
- Event loop: network events → consensus → block application → gossip
- Lifecycle: init → sync → running → shutdown (graceful)
- Tests: single node starts + creates genesis, two nodes sync state

**Files:** `src/node.rs`
**Verify:** `cargo test`

---

### 🔴 Devil's Advocate — Wave 2
- Attempt man-in-the-middle between QUIC connections
- Test network partition + rejoin scenarios
- Verify sync handles corrupted blocks gracefully
- Check for deadlocks in event loop under high message volume

---

## Wave 3: Chiral Validation + Berry Audit
> The differentiators. This is what makes it *ours*.

### Task 3.1: Chiral stability scoring
**Time:** 5 min | **TDD:** Yes

- Port QuantumOS chiral stability equation: `stability = |η/γ|`
- `ChiralValidator`: score transaction batches for data pattern naturalness
- Classification: EXCELLENT/GOOD/MARGINAL/UNSTABLE
- Reject UNSTABLE blocks, flag MARGINAL for review
- Tests: normal SCADA data → EXCELLENT, synthetic ordered → flagged, random noise → flagged

**Files:** `src/validation/chiral.rs`, `src/validation/mod.rs`
**Verify:** `cargo test`

---

### Task 3.2: Berry phase audit computation
**Time:** 5 min | **TDD:** Yes

- Track cumulative geometric phase as events are anchored
- Berry connection: A(θ) computed from state transition vectors
- Phase accumulation: γ = ∮ A(θ)·dθ over event cycles
- Topological invariant: proves event sequence consistency
- Tests: known event sequence → deterministic Berry phase, tampered sequence → different phase, cycle detection

**Files:** `src/validation/berry.rs`
**Verify:** `cargo test`

---

### Task 3.3: Chiral perturbation for consensus health
**Time:** 5 min | **TDD:** Yes

- Monitor order parameter r over time
- When r → 1.0 sustained (>30 blocks): inject chiral noise η into phase updates
- `ηᵢ(t) = η₀ · sin(2·θᵢ) · (-1)^cluster_id` (asymmetric by node)
- Prevents phase lock, maintains healthy Xi (complexity)
- Tests: verify r stabilizes in [0.7, 0.95] range, perturbation activates/deactivates correctly

**Files:** `src/consensus/chiral.rs`
**Verify:** `cargo test`

---

### 🔴 Devil's Advocate — Wave 3
- Test chiral scoring with adversarial SCADA data designed to bypass detection
- Verify Berry phase is truly topologically invariant (reorder blocks, check phase)
- Attempt to destabilize consensus via crafted phase perturbation
- Check that chiral injection doesn't cause consensus failure

---

## Wave 4: RPC + CLI + Integration
> Polish and prove it works end-to-end.

### Task 4.1: RPC server
**Time:** 5 min | **TDD:** Yes

- JSON-RPC over HTTP (hyper, minimal)
- Methods: get_block, get_transaction, verify_anchor, get_validators, get_status, get_berry_phase, submit_transaction
- Tests: submit anchor batch via RPC, query back, verify inclusion proof

**Files:** `src/rpc.rs`
**Verify:** `cargo test`

---

### Task 4.2: CLI + configuration
**Time:** 5 min | **TDD:** Yes

- clap CLI: `0xscada-node run`, `init` (keygen + genesis), `status`, `verify`
- TOML config: listen addr, seeds, keypair, data dir, RPC port, block time, coupling K, chiral η
- Tests: config parsing, CLI arg validation

**Files:** `src/main.rs`, `src/config.rs`
**Verify:** `cargo test`

---

### Task 4.3: Integration test — 3-node cluster
**Time:** 5 min | **TDD:** Yes

- 3 nodes in-process (different ports)
- Submit anchor batch → verify all nodes have block with attestations
- Verify leader rotation via Kuramoto phase evolution
- Kill leader → verify fallback election → verify sync
- Check Berry phase consistency across all nodes
- Check chiral scoring on submitted data

**Files:** `tests/cluster.rs`
**Verify:** `cargo test --test cluster`

---

### Task 4.4: README + ADR-0001
**Time:** 3 min | **TDD:** No

- README: overview, build, usage, architecture diagram
- ADR-0001: all architecture decisions documented
- Commit, create GitHub repo, push

**Files:** `README.md`, `docs/adr/ADR-0001-resonant-consensus.md`
**Verify:** Manual review

---

### 🔴 Devil's Advocate — Final
Full-system adversarial review:
- Attempt double-anchoring same event batch
- Network partition with competing leaders
- State divergence detection
- Resource usage profiling (memory, CPU, disk)
- ARM vs x86 determinism check
- Security review of all crypto operations

---

## Execution

Each task dispatched via `sessions_spawn`. Two-stage review per task (spec + quality).
Devil's advocate after each wave before proceeding.
Wave 0 tasks 0.1-0.4 can run in parallel (independent). Task 0.5 depends on 0.1-0.4.
