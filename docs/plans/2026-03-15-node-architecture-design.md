# 0xSCADA-node — Architecture Design

**Date:** 2026-03-15
**Status:** Approved
**Author:** Kannaka + Nick

## Purpose

A purpose-built, lightweight blockchain node for the 0xSCADA industrial control system. Replaces dependency on external chains (Ethereum/Hardhat) with a permissioned audit ledger that provides cryptographic integrity, immutable event trails, and CFR 21 Part 11 compliance for industrial SCADA deployments.

## Constraints

- Must run on 1 ARM OCPU / 6GB RAM (Oracle Free Tier)
- Must run on old x86 hardware (i5-2450M, 8GB RAM)
- Cross-platform: aarch64-linux + x86_64-linux (+ x86_64-windows for dev)
- No external runtime dependencies (single static binary)
- Rust — consistent with kannaka-memory stack
- 3-4 node initial network, upgradeable

## Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│                   0xSCADA Node                       │
├──────────────┬──────────────┬────────────────────────┤
│   Network    │  Consensus   │      Storage           │
│   (QUIC)     │  (Raft-BFT)  │      (sled)            │
├──────────────┼──────────────┼────────────────────────┤
│  RPC Server  │  Block Mgr   │   State (Sparse MT)    │
├──────────────┴──────────────┴────────────────────────┤
│                  Plugin System                       │
│            (future WASM VM slot)                     │
└──────────────────────────────────────────────────────┘
```

### Hybrid Design

**Phase 1 (now):** Audit-only chain with native transaction types. The EventAnchor contract logic becomes a built-in transaction handler — no VM overhead.

**Phase 2 (future):** WASM plugin system via `Extension` transaction type. Smart contracts can be deployed when needed, without rewriting the core.

## Network Layer — QUIC (quinn)

- **Transport:** QUIC via `quinn` crate — built-in TLS 1.3, multiplexed streams, NAT traversal
- **Streams:**
  - Stream 0: Block gossip (new blocks broadcast to peers)
  - Stream 1: Consensus messages (Raft heartbeats, vote requests, log replication)
  - Stream 2: RPC (client queries, submit transactions)
  - Stream 3: Sync (catch-up for nodes joining late)
- **Peer discovery:** Static seed list (config file). No DHT needed for 3-4 known nodes.
- **Identity:** Each node has an ed25519 keypair. QUIC certificates derived from node identity.
- **Connections:** Full mesh between all validators. Persistent connections with automatic reconnect.

## Consensus — Raft-BFT Hybrid

### Why not pure PBFT?
Classic PBFT requires 3f+1 nodes to tolerate f Byzantine faults. With 3 nodes, f=0 — no fault tolerance. With 4 nodes, f=1, but we don't have 4 validators yet.

### Why not pure Raft?
Raft assumes crash faults only (honest nodes). For an audit chain, we want cryptographic attestation — proof that validators actually signed off on blocks, not just replicated a leader's log.

### The Hybrid
- **Leader election:** Raft-style. Heartbeat timeout → candidate → vote → leader. Simple, well-understood, fast.
- **Block proposal:** Leader collects transactions, builds block, proposes to followers.
- **Attestation:** Followers validate the block and return ed25519 signatures. Block is final when 2-of-3 (or ⌈n/2⌉+1) attestations collected.
- **Finality:** Immediate. Once attestation threshold met, block is committed. No forks, no reorgs.
- **Crypto guarantee:** Every committed block carries validator signatures. Even if you don't trust the leader, you can verify the attestations independently. This is what makes it "BFT-lite" — not full Byzantine tolerance, but cryptographic non-repudiation.

### Parameters
- **Block time:** 2 seconds (configurable)
- **Heartbeat interval:** 500ms
- **Election timeout:** 2-5 seconds (randomized)
- **Max transactions per block:** 1000
- **Empty blocks:** Skipped (no empty block spam)

## Block Structure

```
Block {
  header: BlockHeader {
    version: u8,              // Protocol version (1)
    height: u64,              // Block number
    timestamp: u64,           // Unix millis
    prev_hash: [u8; 32],      // blake3 of previous block
    tx_root: [u8; 32],        // Merkle root of transactions
    state_root: [u8; 32],     // Sparse Merkle Tree root
    proposer: [u8; 32],       // ed25519 pubkey of block proposer
    signature: [u8; 64],      // Proposer's signature over header
  },
  transactions: Vec<Transaction>,
  attestations: Vec<Attestation {
    validator: [u8; 32],      // ed25519 pubkey
    signature: [u8; 64],      // Signature over block hash
  }>,
}
```

### Transaction Types

```rust
enum TransactionBody {
    // Phase 1: Native audit transactions
    AnchorBatch {
        merkle_root: [u8; 32],   // Merkle root of SCADA event batch
        event_count: u32,         // Number of events in batch
        site_id: String,          // Industrial site identifier
        metadata: Vec<u8>,        // CBOR-encoded metadata
    },
    ValidatorUpdate {
        action: ValidatorAction,  // Add or Remove
        pubkey: [u8; 32],        // Validator's ed25519 pubkey
        proof: Vec<u8>,          // Authorization proof
    },
    
    // Phase 2: Plugin transactions (future)
    Extension {
        module_id: [u8; 32],     // Plugin/contract identifier
        payload: Vec<u8>,        // Opaque payload for WASM VM
    },
}
```

### Cryptographic Choices
- **Hashing:** blake3 — fastest cryptographic hash, pure Rust, SIMD-accelerated
- **Signatures:** ed25519 via `ed25519-dalek` — small (64 bytes), fast, used across our stack
- **Serialization:** CBOR via `ciborium` — compact binary, schema-flexible, self-describing
- **State tree:** Sparse Merkle Tree — supports inclusion AND exclusion proofs (critical for compliance)

## Storage — sled

- **Engine:** sled embedded key-value store — pure Rust, zero external dependencies, crash-safe
- **Trees (namespaces):**
  - `blocks` — height → serialized block
  - `block_hashes` — hash → height (reverse index)
  - `state` — Sparse Merkle Tree nodes
  - `tx_index` — tx_hash → (height, index) for transaction lookup
  - `metadata` — chain config, validator set, checkpoint data
- **Pruning:** Optional — keep last N blocks for resource-constrained nodes, full archive for compliance nodes
- **Estimated storage:** ~200 bytes/block overhead + transaction data. At 2s block time, ~15K blocks/day = ~3MB/day empty, ~30MB/day with typical SCADA event load. Years of data fit in the free tier's storage.

## RPC Interface

JSON-RPC over QUIC stream 2 (and optional HTTP for dev tooling):

```
// Queries
get_block(height | hash) → Block
get_transaction(hash) → Transaction + inclusion proof
get_state_root(height?) → [u8; 32]
verify_anchor(merkle_root) → {exists, height, attestations}
get_validators() → Vec<ValidatorInfo>
get_status() → {height, peers, role, uptime}

// Submissions
submit_transaction(signed_tx) → tx_hash
submit_anchor_batch(batch) → tx_hash  // Convenience wrapper
```

## Node Topology

| Node | Location | Arch | CPU | RAM | Role |
|------|----------|------|-----|-----|------|
| alpha | Oracle VM #2 | aarch64 | 1 OCPU | 6GB | Validator |
| beta | Oracle VM #3 | aarch64 | 1 OCPU | 6GB | Validator |
| gamma | Old laptop | x86_64 | i5-2450M 2c | 8GB | Validator |
| dev | Nick's desktop | x86_64 | — | — | Dev/test |
| ninja-portal | Oracle VM #1 | aarch64 | 1 OCPU | 6GB | Seed/bootstrap (not validator) |

## Testing Strategy

1. **Unit tests** — per module (crypto, block, consensus state machine, storage, merkle tree)
2. **Integration tests** — 3-node in-process cluster using tokio tasks:
   - Normal operation: propose, attest, commit
   - Leader failure: election, recovery
   - Network partition: split, rejoin, sync
   - Late joiner: new node catches up
3. **All tests run with `cargo test`** — no external dependencies
4. **Simulated network** proven locally before any deployment

## Deployment

- Cross-compile: `cross` or `cargo-zigbuild` for ARM + x86 targets
- Deploy: scp binary + config to node, systemd service
- Config: TOML file with seed peers, keypair path, storage path, RPC bind address
- Genesis: CLI command generates genesis block + initial validator set

## Crate Dependencies (estimated)

```toml
quinn = "0.11"          # QUIC transport
ed25519-dalek = "2"     # Signatures
blake3 = "1"            # Hashing
ciborium = "0.2"        # CBOR serialization
sled = "0.34"           # Storage
serde = "1"             # Serialization framework
tokio = "1"             # Async runtime
tracing = "0.1"         # Logging
clap = "4"              # CLI
toml = "0.8"            # Config parsing
```

No heavy deps. No protobuf codegen. No C++ bindings. Clean Rust.

## What This Is NOT

- Not a general-purpose blockchain (no smart contracts in Phase 1)
- Not a cryptocurrency (no tokens, no fees, no mining)
- Not trying to be Ethereum (no EVM, no gas)
- Not a public chain (permissioned validators only)

This is an **industrial audit ledger** with cryptographic integrity guarantees, purpose-built for SCADA compliance.

## Success Criteria

1. Three nodes form a network and produce blocks
2. SCADA event batches can be anchored and verified
3. Node can crash and rejoin without data loss
4. Single binary < 20MB, < 50MB RAM at runtime
5. Block finality < 3 seconds
6. All operations work on 1 ARM OCPU / 6GB RAM
