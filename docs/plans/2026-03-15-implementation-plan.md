# 0xSCADA-node — Implementation Plan

**Date:** 2026-03-15
**Design:** [2026-03-15-node-architecture-design.md](2026-03-15-node-architecture-design.md)

## Task Order

Build bottom-up: crypto primitives → storage → block management → consensus → networking → RPC → CLI → integration tests.

---

### Task 1: Project scaffold + crypto primitives
**Time:** 5 min | **TDD:** Yes

- `cargo init` with workspace-ready structure
- Implement `crypto` module: ed25519 keypair generation, signing, verification; blake3 hashing
- Tests: roundtrip sign/verify, hash determinism, invalid signature rejection

**Files:** `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `src/crypto.rs`
**Verify:** `cargo test`

---

### Task 2: Block and transaction types
**Time:** 5 min | **TDD:** Yes

- Define `Block`, `BlockHeader`, `Transaction`, `TransactionBody`, `Attestation` structs
- CBOR serialization via serde + ciborium
- Block hashing (blake3 of serialized header)
- Block signing and attestation creation
- Tests: serialize/deserialize roundtrip, block hash stability, attestation verification

**Files:** `src/block.rs`, `src/transaction.rs`
**Verify:** `cargo test`

---

### Task 3: Sparse Merkle Tree
**Time:** 5 min | **TDD:** Yes

- Implement Sparse Merkle Tree with blake3
- Operations: insert, get, root hash, inclusion proof, exclusion proof, verify proof
- Tests: empty tree root, single insert, multiple inserts, proof verification, exclusion proof

**Files:** `src/merkle.rs`
**Verify:** `cargo test`

---

### Task 4: Storage layer (sled)
**Time:** 5 min | **TDD:** Yes

- `Storage` struct wrapping sled with typed trees (blocks, block_hashes, state, tx_index, metadata)
- Operations: store_block, get_block_by_height, get_block_by_hash, get_tx, get_latest_height, store_state_root
- Tests: store/retrieve blocks, latest height tracking, transaction index lookup

**Files:** `src/storage.rs`
**Verify:** `cargo test`

---

### Task 5: Block manager
**Time:** 5 min | **TDD:** Yes

- `BlockManager` — validates and applies blocks to storage
- Genesis block creation
- Block validation: hash chain, signature, attestation threshold, timestamp ordering
- Transaction execution: AnchorBatch → update state tree, ValidatorUpdate → update validator set
- Tests: genesis creation, valid block acceptance, invalid block rejection (bad sig, wrong height, insufficient attestations)

**Files:** `src/block_manager.rs`
**Verify:** `cargo test`

---

### Task 6: Consensus engine (Raft-BFT)
**Time:** 5 min | **TDD:** Yes

- State machine: Follower → Candidate → Leader
- Leader: collect transactions, propose blocks, gather attestations
- Follower: validate proposals, return attestations
- Election: randomized timeout, vote request/response
- Message types: Heartbeat, VoteRequest, VoteResponse, BlockProposal, Attestation
- Tests: election with 3 nodes (in-process channels), leader produces block, follower attests, leader failure triggers re-election

**Files:** `src/consensus.rs`
**Verify:** `cargo test`

---

### Task 7: Network layer (QUIC)
**Time:** 5 min | **TDD:** Yes

- `Network` struct using quinn — listen, connect to peers, send/receive messages
- Message framing: length-prefixed CBOR over QUIC streams
- Peer management: connect to seed list, track connected peers, reconnect on disconnect
- Multiplexed streams: consensus, gossip, sync, RPC
- Tests: two-node connection, message roundtrip, reconnect after disconnect

**Files:** `src/network.rs`
**Verify:** `cargo test`

---

### Task 8: Node orchestrator
**Time:** 5 min | **TDD:** Yes

- `Node` struct — ties together Network, Consensus, BlockManager, Storage
- Event loop: receive network messages → feed to consensus → apply blocks → gossip
- Transaction mempool: accept submitted transactions, feed to leader
- Sync protocol: new node requests blocks from height 0, catches up
- Tests: single node starts and creates genesis, two nodes sync

**Files:** `src/node.rs`
**Verify:** `cargo test`

---

### Task 9: RPC server
**Time:** 5 min | **TDD:** Yes

- JSON-RPC over HTTP (hyper, minimal — for dev tooling and external clients)
- Methods: get_block, get_transaction, verify_anchor, get_validators, get_status, submit_transaction
- Tests: submit anchor batch via RPC, query it back, verify inclusion proof

**Files:** `src/rpc.rs`
**Verify:** `cargo test`

---

### Task 10: CLI and configuration
**Time:** 5 min | **TDD:** Yes

- `clap` CLI: `0xscada-node run`, `0xscada-node init` (generate keypair + genesis), `0xscada-node status`
- TOML config: listen address, seed peers, keypair path, data directory, RPC port, block time
- Tests: config parsing, CLI arg validation

**Files:** `src/main.rs`, `src/config.rs`
**Verify:** `cargo test`

---

### Task 11: Integration test — 3-node cluster
**Time:** 5 min | **TDD:** Yes

- Spin up 3 nodes in-process (different ports)
- Submit anchor batch to leader
- Verify all 3 nodes have the block with attestations
- Kill leader, verify re-election
- Restart killed node, verify sync
- Submit another batch, verify consistency

**Files:** `tests/cluster.rs`
**Verify:** `cargo test --test cluster`

---

### Task 12: README + ADR
**Time:** 3 min | **TDD:** No

- Write README.md with project overview, build instructions, usage
- Write ADR-0001 documenting architecture decisions
- Commit everything, create GitHub repo, push

**Files:** `README.md`, `docs/adr/ADR-0001-node-architecture.md`
**Verify:** Manual review

---

## Execution Mode

Subagent-driven: each task dispatched as a sub-agent with full plan context, followed by spec review and code quality review.
