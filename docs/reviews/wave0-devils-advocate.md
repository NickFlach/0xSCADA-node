# Wave 0 — Devil's Advocate Review

**Date:** 2026-03-15  
**Reviewer:** Adversarial code review (automated)  
**Scope:** All `src/*.rs` files, `Cargo.toml`  
**Tests:** 27/27 passing, 3 clippy warnings  

---

## CRITICAL

### C1: Transaction `hash` field is not validated — anyone can set it to anything
**File:** `src/transaction.rs:8-12`  
**Severity:** CRITICAL  

`Transaction.hash` is a public field set by the caller. There is no validation that `hash == body.hash()`. A malicious node can set `hash` to any 32 bytes while the body contains different data. Any code that indexes or references transactions by `hash` (e.g., `storage.rs` tx_index) will be operating on a lie.

**Impact:** Complete integrity bypass for transaction references. A node could claim transaction X exists in a block while the actual body is Y.

**Fix:** Either (a) make `hash` private and compute it on construction, or (b) add a `verify_hash(&self) -> bool` method and enforce it during block validation.

### C2: No block validation exists — invalid blocks are accepted unconditionally  
**File:** All of `src/block.rs`  
**Severity:** CRITICAL  

There is zero validation logic for blocks:
- `header_signature` is never verified against `proposer`
- `attestations[*].signature` is never verified against `attestations[*].validator`
- `tx_root` is never verified against the actual transaction list
- `state_root` is never verified
- `prev_hash` chain is never validated
- `height` monotonicity is never checked
- No minimum attestation threshold

This is Wave 0 foundation code, but since `store_block()` in storage accepts anything, there is currently no barrier to persisting garbage. Block validation MUST exist before any networking code lands.

**Fix:** Create a `BlockValidator` that checks all of the above before storage. Even a stub with TODO markers for consensus-dependent checks would establish the pattern.

### C3: `expect()` panics in serialization paths — production crash risk
**Files:** `src/block.rs:38,51`, `src/transaction.rs:40,50`  
**Severity:** CRITICAL  

`BlockHeader::serialize()`, `Block::serialize()`, `Transaction::serialize()`, and `TransactionBody::serialize()` all use `.expect("CBOR serialize failed")`. CBOR serialization of user-controlled data (e.g., arbitrarily large `metadata: Vec<u8>`, `site_id: String`) could trigger OOM or writer errors, causing a panic that crashes the node.

**Fix:** Return `Result<Vec<u8>, Error>` instead of panicking.

---

## HIGH

### H1: No size limits on transactions — denial of service via memory exhaustion
**Files:** `src/transaction.rs:15-28`  
**Severity:** HIGH  

`TransactionBody::AnchorBatch.metadata` is `Vec<u8>` with no size limit. `TransactionBody::Extension.payload` is `Vec<u8>` with no limit. `site_id` is `String` with no limit. A malicious actor can craft a multi-GB transaction that exhausts node memory during deserialization.

Similarly, `Block.transactions: Vec<Transaction>` and `Block.attestations: Vec<Attestation>` are unbounded.

**Fix:** Add `#[serde(deserialize_with = ...)]` size-limiting wrappers, or validate sizes post-deserialization before processing. Define protocol constants: `MAX_TX_SIZE`, `MAX_BLOCK_TRANSACTIONS`, `MAX_ATTESTATIONS`, `MAX_METADATA_SIZE`.

### H2: Storage `store_block` is not atomic — crash mid-write leaves inconsistent state
**File:** `src/storage.rs:60-74`  
**Severity:** HIGH  

`store_block()` performs 3+ separate sled inserts (block data, hash index, N tx indexes) without a transaction. If the process crashes after writing the block but before writing `block_hashes`, you get a block accessible by height but not by hash. If it crashes mid-tx-index, some transactions are indexed and others aren't.

**Fix:** Use `sled::Db::transaction()` (sled's built-in transactional API) to make all writes atomic.

### H3: `StorableBlock` duplicates `Block` — schema will diverge
**File:** `src/storage.rs:7-15`  
**Severity:** HIGH  

`StorableBlock` is a separate struct from `Block` with a comment saying "Will be replaced." Until it is, there's a dual-schema problem: the actual `Block` type and the storage type can drift apart. The `data: Vec<u8>` field is opaque and undocumented.

**Fix:** Implement `From<Block> for StorableBlock` now, or just serialize `Block` directly into storage.

### H4: Sparse Merkle Tree `update_path` has unused variable suggesting logic bug
**File:** `src/merkle.rs:146`  
**Severity:** HIGH  

```rust
let bit = get_bit(key, d);  // unused!
let mut sibling_key = *key;
let byte = d / 8;
let bit_pos = 7 - (d % 8);
sibling_key[byte] ^= 1 << bit_pos;
let sibling = self.node_hash(d + 1, &sibling_key);
let parent = if bit {  // Wait — `bit` IS used on line 153
```

Actually, looking more carefully: the `bit` variable at line 146 IS used at line 153 (`if bit {`). The compiler warning about "unused variable `bit`" is **wrong** or there's a shadowing issue. Let me re-check...

The warning comes from `inclusion_proof()` line 146, NOT `update_path()`. In `inclusion_proof()`:
```rust
let bit = get_bit(key, d);  // line 146 — UNUSED
let mut sibling_key = *key;
```
The `bit` is computed but never used — the sibling is collected regardless of direction. This is fine for proof generation (you collect all siblings), but the unused variable suggests the code was copy-pasted from `update_path` without cleanup. Not a correctness bug but messy.

**Revised severity:** LOW (just cleanup)

### H5: `crypto` module types are disconnected from `block`/`transaction` types
**Files:** `src/crypto.rs`, `src/block.rs`, `src/transaction.rs`  
**Severity:** HIGH  

`Block` stores signatures as raw `[u8; 64]` and public keys as `[u8; 32]`, while `crypto.rs` has `Signature([u8; 64])` and `PublicKey([u8; 32])` newtypes. These types are never used together. The crypto module provides `verify()` but nothing in block.rs or transaction.rs calls it.

This means the crypto module is orphaned — it exists but isn't integrated. When validation is added, someone will need to bridge these types, and the mismatch (newtype vs raw array) will cause friction.

**Fix:** Either use the crypto newtypes in Block/Transaction/Attestation, or provide `impl From` conversions. Add a `Block::verify_signatures()` method that uses the crypto module.

### H6: CBOR field ordering is struct-declaration-order-dependent
**Files:** All serializable structs  
**Severity:** HIGH  

ciborium serializes structs as CBOR maps with string keys by default (via serde). Reordering struct fields or renaming them changes the serialization. This means:
- Adding a field in the middle of a struct is a **breaking change** to the hash
- No `#[serde(rename)]` attributes are used, so field names are baked into the wire format
- No versioning strategy for struct evolution

For a blockchain where hashes must be deterministic across all nodes forever, this is dangerous.

**Fix:** Either (a) add `#[serde(rename = "0", rename = "1", ...)]` to use positional keys, or (b) serialize as CBOR arrays instead of maps (`#[serde(serialize_as = "tuple")]` or manual impl), or (c) document that struct field order is part of the protocol spec and must never change.

---

## MEDIUM

### M1: `sled` is unmaintained — consider alternatives for production
**File:** `Cargo.toml`  
**Severity:** MEDIUM  

sled 0.34 has been in alpha/beta for years and the author has moved on to other projects. It has known issues with large databases and crash recovery. For an industrial SCADA audit trail that needs reliability, this is concerning.

**Fix:** Acceptable for prototyping. For production, evaluate `redb`, `rocksdb`, or `sqlite` (via `rusqlite`). Document the decision and plan for migration.

### M2: No `PartialEq`/`Eq` derives on core types
**Files:** `src/block.rs`, `src/transaction.rs`  
**Severity:** MEDIUM  

`Block`, `BlockHeader`, `Transaction`, `TransactionBody`, `Attestation` lack `PartialEq`/`Eq`. This makes testing harder (can't `assert_eq!` on blocks) and will complicate consensus logic that needs to compare blocks.

**Fix:** Add `#[derive(PartialEq, Eq)]` to all core types.

### M3: `StorageError` reuses `BlockNotFound` for missing state roots
**File:** `src/storage.rs:103`  
**Severity:** MEDIUM  

`get_state_root()` returns `StorageError::BlockNotFound` when a state root is missing. This is semantically wrong and will confuse callers.

**Fix:** Add `StorageError::StateRootNotFound` or a generic `StorageError::NotFound(String)`.

### M4: No `Display`/`Debug` for `MerkleProof`'s large arrays
**File:** `src/merkle.rs`  
**Severity:** MEDIUM  

`MerkleProof` derives `Debug` but has 256 siblings of 32 bytes each. Accidentally debug-printing a proof will dump 8KB of hex to logs.

**Fix:** Implement a custom `Debug` that truncates, or at least document this.

### M5: `from_bytes` returns `Result` but can never fail
**File:** `src/crypto.rs:19-24`  
**Severity:** MEDIUM  

`NodeKeypair::from_bytes()` returns `Result<Self, SignatureError>` but `SigningKey::from_bytes()` in ed25519-dalek v2 is infallible (any 32 bytes is a valid secret key). The `let _ = signing_key.verifying_key()` check also always succeeds. The function signature implies fallibility that doesn't exist.

**Fix:** Either return `Self` directly, or if you want to reserve the right to fail in the future, document why it returns Result.

### M6: No `delete_block` or chain reorganization support in storage
**File:** `src/storage.rs`  
**Severity:** MEDIUM  

Storage is append-only with no ability to remove blocks. Chain reorgs (even small ones) will require deleting blocks and their tx indexes. The current API doesn't support this.

**Fix:** Add `delete_block(height)` that removes the block, its hash index, and all associated tx index entries. Even if not needed in Wave 0, the API should be designed with this in mind.

### M7: `height_key` uses big-endian — correct for ordering but undocumented
**File:** `src/storage.rs:42`  
**Severity:** MEDIUM  

Using BE encoding for height keys is correct (sled sorts lexicographically, and BE u64 preserves numeric order). But this is a subtle invariant that should be documented, as changing it would silently corrupt the database.

**Fix:** Add a comment explaining why BE is used.

---

## LOW

### L1: Unused variable in `inclusion_proof`
**File:** `src/merkle.rs:146`  
**Severity:** LOW  

`let bit = get_bit(key, d)` is unused. Prefix with underscore.

### L2: Clippy warning — `new_without_default` for SparseMerkleTree
**File:** `src/merkle.rs:68`  
**Severity:** LOW  

Implement `Default` trait.

### L3: Clippy warning — needless borrow in storage
**File:** `src/storage.rs:66`  
**Severity:** LOW  

`&block.hash` → `block.hash` (it implements the required traits).

### L4: No `#[must_use]` on hash/root functions
**Files:** Various  
**Severity:** LOW  

Functions like `BlockHeader::hash()`, `SparseMerkleTree::root()`, `TransactionBody::hash()` return values that should never be silently discarded.

### L5: `main.rs` is a stub
**File:** `src/main.rs`  
**Severity:** LOW  

Expected for Wave 0, but should be noted.

### L6: No documentation comments (`///`) on any public types or functions
**Files:** All  
**Severity:** LOW  

For a protocol implementation, doc comments on public API are important for correctness review and onboarding.

### L7: Test helpers use simple patterns that may mask edge cases
**Files:** Various test modules  
**Severity:** LOW  

Test keys like `[0u8; 32]`, `[height as u8; 32]` have many repeated bytes. These don't exercise the full key space and could miss byte-order bugs. Consider using `blake3::hash(height.to_be_bytes())` for more realistic test data.

---

## Summary

| Severity | Count | Key Themes |
|----------|-------|------------|
| CRITICAL | 3 | No validation, panics in production paths, unverified tx hashes |
| HIGH | 5* | DoS via unbounded sizes, non-atomic storage, type disconnects, serialization fragility |
| MEDIUM | 7 | sled concerns, missing derives, API gaps |
| LOW | 7 | Clippy warnings, docs, test quality |

*H4 was revised down to LOW after analysis — effective HIGH count is 4.

### Top 3 Actions Before Wave 1:
1. **Add block/transaction validation** (C2, C1) — this is the most important missing piece
2. **Remove `expect()` panics from serialization** (C3) — production crash risk
3. **Add size limits and atomic storage writes** (H1, H2) — DoS and data integrity
