# Contract Version Getter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose a `VERSION` const and `version()` view function so UIs and indexers can query the deployed contract version.

**Architecture:** Add a module-level `const VERSION: u32` using packed semver encoding (`major * 1_000_000 + minor * 1_000 + patch`). Expose it via a zero-cost `version()` public function on `StreamPayContract`. No storage, no auth, no new dependencies.

**Tech Stack:** Rust, Soroban SDK 22.0

**Spec:** `docs/superpowers/specs/contract-version-getter-design.md`

---

### Task 1: Add VERSION const and version() view — tests first

**Files:**
- Modify: `src/lib.rs:125-183` (test module)
- Modify: `src/lib.rs:1-5` (module header / const)
- Modify: `src/lib.rs:22-96` (contractimpl block)

- [ ] **Step 1: Write the failing tests**

Add three tests to the existing `mod test` block in `src/lib.rs`, before the closing `}` of `mod test` (line 183):

```rust
    #[test]
    fn test_version_returns_expected() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), 1_000);
    }

    #[test]
    fn test_version_matches_const() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), VERSION);
    }

    #[test]
    fn test_version_is_positive() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert!(client.version() > 0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test 2>&1`
Expected: Compilation error — `version` method does not exist on client, `VERSION` not found.

- [ ] **Step 3: Add the VERSION const**

Insert after line 5 (`use soroban_sdk::{...};`) in `src/lib.rs`:

```rust

/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.1.0 → 1_000
const VERSION: u32 = 1_000;
```

- [ ] **Step 4: Add the version() view function**

Insert inside the `#[contractimpl] impl StreamPayContract` block, after `get_stream_info` (after line 95):

```rust

    /// Returns the contract version as a u32 (see VERSION encoding).
    pub fn version(_env: Env) -> u32 {
        VERSION
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test 2>&1`
Expected: All 6 tests pass (3 existing + 3 new).

- [ ] **Step 6: Run format check**

Run: `cargo fmt --all -- --check 2>&1`
Expected: No formatting issues.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "feat(contracts): expose contract version to callers

Add VERSION const (u32, packed semver) and version() view function.
Three new tests for correctness."
```

---

### Task 2: Update README with version() docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add version() to the contract interface list**

In `README.md`, add after the `get_stream_info` line in the contract interface section:

```markdown
- **`version()`** — Returns the contract version as a `u32` (no auth required).
```

- [ ] **Step 2: Add version encoding section**

Add a new section after the "Contract interface" section:

```markdown
### Version encoding

The on-chain version uses a packed `u32` scheme: `major * 1_000_000 + minor * 1_000 + patch`.

| Semver | u32       |
|--------|-----------|
| 0.1.0  | 1 000     |
| 1.0.0  | 1 000 000 |
| 1.2.3  | 1 002 003 |

When releasing, update **both** `Cargo.toml` `version` and the `VERSION` const in `src/lib.rs`.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document version() and u32 encoding scheme in README"
```

---

### Task 3: Update module doc comment

**Files:**
- Modify: `src/lib.rs:1-3`

- [ ] **Step 1: Update the module-level doc comment**

Change line 3 from:
```rust
//! Provides: create_stream, start_stream, stop_stream, settle_stream.
```
to:
```rust
//! Provides: create_stream, start_stream, stop_stream, settle_stream, version.
```

- [ ] **Step 2: Run tests and fmt check**

Run: `cargo test 2>&1 && cargo fmt --all -- --check 2>&1`
Expected: All pass.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "docs: mention version in module doc comment"
```

---

### Task 4: Final verification

- [ ] **Step 1: Run full build**

Run: `cargo build 2>&1`
Expected: Compiles with no errors or warnings.

- [ ] **Step 2: Run full test suite**

Run: `cargo test 2>&1`
Expected: All 6 tests pass.

- [ ] **Step 3: Run format check**

Run: `cargo fmt --all -- --check 2>&1`
Expected: Clean.
