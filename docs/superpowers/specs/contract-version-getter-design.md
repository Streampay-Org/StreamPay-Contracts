# Contract Version Getter — Design Spec

**Date:** 2026-03-24
**Status:** Approved
**Issue:** Expose VERSION const and a `version()` view for UIs and indexers

## Problem

UIs, indexers, and other off-chain consumers have no way to query which version of the StreamPay contract is deployed. This makes it difficult to detect upgrades or confirm compatibility.

## Decision

Use a **u32 encoding scheme**: `major * 1_000_000 + minor * 1_000 + patch`.

### Why u32 over semver string

- No heap allocation in the Soroban VM — cheaper to call
- Trivially comparable by indexers (`if version > 1_002_000 { ... }`)
- Matches patterns used by established Soroban contracts (Blend, Phoenix)
- `Cargo.toml` remains the canonical semver string — no duplication needed

### Encoding examples

| Semver  | u32         |
|---------|-------------|
| 0.1.0   | 1_000       |
| 1.0.0   | 1_000_000   |
| 1.2.3   | 1_002_003   |

Range: supports up to `999.999.999`.

## Changes

### src/lib.rs

1. **Module-level constant** (above the contract impl):

```rust
/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.1.0 → 1_000
const VERSION: u32 = 1_000;
```

2. **New public view function** on `StreamPayContract`:

```rust
/// Returns the contract version as a u32 (see VERSION encoding).
pub fn version(_env: Env) -> u32 {
    VERSION
}
```

No storage reads, no auth, no state mutation — pure constant return.

### README.md

- Add `version()` to the contract interface list
- Add a section documenting the u32 encoding scheme

### Tests (in src/lib.rs `mod test`)

| Test                            | Assertion                                    |
|---------------------------------|----------------------------------------------|
| `test_version_returns_expected` | `version()` returns `1_000`                  |
| `test_version_matches_const`    | View return equals the `VERSION` const       |
| `test_version_is_positive`      | `version() > 0`                              |

## Security Notes

- **View-only** — no auth required (version is public information)
- **No storage access** — cannot be used to manipulate state
- **No new storage keys** — zero footprint increase
- **No new dependencies**

## Out of Scope

- Automatic sync between `Cargo.toml` version and `VERSION` const (manual update on release)
- On-chain upgrade detection logic
- String-based version endpoint

## Release Process Impact

When cutting a new release, update **both**:
1. `Cargo.toml` → `version = "X.Y.Z"`
2. `src/lib.rs` → `const VERSION: u32 = X_00Y_00Z;`
