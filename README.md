# 0xSCADA-node

**A purpose-built blockchain for industrial SCADA audit trails with resonant consensus.**

0xSCADA-node is a permissioned audit ledger designed for industrial control system compliance. It replaces dependency on general-purpose blockchains with a lightweight, single-binary node that provides cryptographic integrity, immutable event trails, and CFR 21 Part 11 compliance — running on hardware as modest as 1 ARM OCPU with 6GB RAM.

No tokens. No mining. No smart contracts. Just provable audit integrity for critical infrastructure.

## What Makes It Different

### Resonant Consensus (Kuramoto-BFT)

Instead of rigid Raft timers or heavyweight PBFT rounds, validators are modeled as Kuramoto phase-coupled oscillators. Each validator maintains a phase θ that naturally drifts toward consensus:

```
dθᵢ/dt = ωᵢ + (K/N)Σⱼsin(θⱼ - θᵢ) + ηᵢ(t)
```

The order parameter **r** becomes a real-time network health metric. Leader selection rotates naturally as phases evolve — no explicit elections needed in steady state. Falls back to Raft-style voting when coherence drops below threshold.

### Chiral Block Validation

Blocks aren't just accept/reject — validators compute a stability score based on the interference-damping ratio `|η/γ|`. Real industrial data lives in the interference zone: blocks that are "too perfect" or "too chaotic" both get flagged. This catches both data fabrication and sensor failures.

### Berry Phase Audit Trail

Geometric phase tracking provides a topological invariant over the audit chain. The Berry phase γ = ∮ A(θ)·dθ proves event sequence integrity even if individual blocks were replayed or reordered — a novel compliance primitive that survives reorganization attempts.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                   0xSCADA Node                       │
├──────────────┬──────────────┬────────────────────────┤
│   Network    │  Consensus   │      Storage           │
│   (QUIC)     │ (Kuramoto)   │      (sled)            │
├──────────────┼──────────────┼────────────────────────┤
│  RPC Server  │  Block Mgr   │   State (Sparse MT)    │
├──────────────┼──────────────┼────────────────────────┤
│  Validation  │   Mempool    │    Transaction Pool    │
├──────────────┴──────────────┴────────────────────────┤
│                    Node Runtime                      │
└──────────────────────────────────────────────────────┘
```

### Modules

| Module | Description |
|--------|-------------|
| `crypto` | Ed25519 keypair generation, signing, verification via `ed25519-dalek` |
| `transaction` | Transaction types: `AnchorBatch`, `ValidatorUpdate`, `Extension` with CBOR serialization |
| `block` | Block and header structures with blake3 hashing, proposer signatures, and attestations |
| `merkle` | Sparse Merkle Tree for state roots with inclusion and exclusion proofs |
| `storage` | Sled-backed persistent storage with block, hash, state, and tx index trees |
| `block_manager` | Block construction, validation pipeline, and chain state management |
| `consensus` | Kuramoto oscillator engine, chiral perturbation, Raft fallback, consensus messages |
| `mempool` | Priority-ordered transaction pool with duplicate detection and expiry |
| `network` | QUIC transport via quinn, peer management, block/consensus gossip, chain sync |
| `validation` | Chiral stability scoring and Berry phase geometric verification |
| `rpc` | JSON-RPC server over HTTP for queries and transaction submission |
| `config` | TOML configuration loading with sensible defaults |
| `node` | Top-level runtime that wires all modules together |

### Cryptographic Choices

- **Hashing:** blake3 — fastest cryptographic hash, pure Rust, SIMD-accelerated
- **Signatures:** ed25519 — small (64 bytes), fast, deterministic
- **Serialization:** CBOR via ciborium — compact binary, schema-flexible
- **State tree:** Sparse Merkle Tree — supports inclusion AND exclusion proofs

## Quick Start

### Build

```bash
cargo build --release
```

The binary is at `target/release/oxscada`.

### Test

```bash
cargo test
```

All tests are self-contained — no external services required.

### Generate a Keypair

```bash
oxscada keygen --output node.key
```

### Configure

Create `node.toml`:

```toml
node_id = "alpha"
listen_addr = "0.0.0.0:4433"
rpc_port = 8545
storage_path = "./data"
keypair_path = "./node.key"
seed_peers = ["192.168.1.100:4433", "192.168.1.101:4433"]

[consensus]
block_time_ms = 2000
coupling_constant = 2.0
natural_frequency = 0.5
chiral_perturbation = 0.05
raft_fallback_threshold = 0.5
max_transactions_per_block = 1000
```

### Run

```bash
oxscada --config node.toml run
```

## Running a 3-Node Local Network

Generate three keypairs and create configs:

```bash
oxscada keygen --output node1.key
oxscada keygen --output node2.key
oxscada keygen --output node3.key
```

Create `node1.toml`, `node2.toml`, `node3.toml` — each with a unique `node_id`, `listen_addr`, `rpc_port`, and `storage_path`, and seed peers pointing to each other:

```toml
# node1.toml
node_id = "alpha"
listen_addr = "127.0.0.1:4433"
rpc_port = 8545
storage_path = "./data/node1"
keypair_path = "./node1.key"
seed_peers = ["127.0.0.1:4434", "127.0.0.1:4435"]
block_time_ms = 5000
coupling_k = 0.5
chiral_eta = 0.1

# node2.toml — listen_addr = "127.0.0.1:4434", rpc_port = 8546, etc.
# node3.toml — listen_addr = "127.0.0.1:4435", rpc_port = 8547, etc.
```

Start each node in a separate terminal:

```bash
oxscada --config node1.toml run
oxscada --config node2.toml run
oxscada --config node3.toml run
```

The nodes will discover each other via QUIC, synchronize phases, and begin producing blocks once the Kuramoto order parameter **r** exceeds the coherence threshold. Monitor consensus health via the RPC `get_status` endpoint on any node.

## CLI Usage

```
oxscada [--config <path>] <command>

Commands:
  run       Start the node
  keygen    Generate a new ed25519 keypair
              -o, --output <path>    Output file (default: node.key)
  status    Query node status via RPC
              -e, --endpoint <url>   RPC endpoint (default: http://127.0.0.1:8545)
```

## JSON-RPC API

The node exposes a JSON-RPC interface over HTTP.

| Method | Parameters | Returns |
|--------|-----------|---------|
| `get_block` | `height` or `hash` | Block with header, transactions, attestations |
| `get_transaction` | `hash` | Transaction with inclusion proof |
| `verify_anchor` | `merkle_root` | Existence proof, block height, attestations |
| `get_validators` | — | Current validator set with status |
| `get_status` | — | Chain height, peer count, node role, uptime |
| `get_berry_phase` | `start_height`, `end_height` | Berry phase γ over block range |
| `submit_transaction` | `signed_tx` | Transaction hash |

### Example

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"get_status","params":[],"id":1}'
```

## Resource Requirements

- **Binary size:** < 20MB static
- **Memory:** < 50MB at runtime
- **Storage:** ~30MB/day with typical SCADA event load
- **Block time:** 2 seconds, immediate finality

Runs on Oracle Cloud Free Tier (1 ARM OCPU, 6GB RAM) and old x86 laptops alike.

## Architecture Decision Records

- [ADR-0001: Resonant Consensus](docs/adr/ADR-0001-resonant-consensus.md) — Why Kuramoto oscillators instead of Raft or PBFT

## License

[Space Child License v1.0](LICENSE.md)
