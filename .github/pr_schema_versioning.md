# docs(contracts): StreamInfo schema evolution guidelines

## Overview

This PR introduces a schema versioning strategy for `StreamInfo` — the
`#[contracttype]` struct that every payment stream is serialised into inside
Soroban persistent storage.  It adds a runtime-detectable version sentinel,
comprehensive rustdoc on the struct and all its fields, and a policy document
that gives contributors a clear, step-by-step process for evolving the struct
safely in future upgrades.

Without this groundwork, any future field addition to `StreamInfo` would
silently break every live stream on the network the moment the upgraded
contract is deployed.  This PR makes that class of mistake detectable,
preventable, and recoverable.

---

## Problem statement

### How Soroban serialises `#[contracttype]` structs

Soroban serialises structs annotated with `#[contracttype]` as **XDR `ScMap`
entries keyed by field name** (each key is a `Symbol`).  The map is sorted
lexicographically by key at write time.  At read time the host matches each
map key to the corresponding struct field by name.

This means the on-ledger binary representation of a `StreamInfo` looks
roughly like:

```
{
  "balance":         i128(10000),
  "end_time":        u64(0),
  "is_active":       bool(false),
  "payer":           Address(...),
  "rate_per_second": i128(100),
  "recipient":       Address(...),
  "start_time":      u64(0),
}
```

### Why struct changes are breaking

When the contract is upgraded and `StreamInfo` gains or loses a field, the
host tries to deserialise every existing ledger entry into the new struct
definition.  The result depends on the type of change:

| Change type | What happens at read time |
|-------------|--------------------------|
| Field added | New key absent in old map → host panics |
| Field removed | Old key present, new struct has no slot → host panics |
| Field renamed | Equivalent to remove + add → host panics |
| Field type changed | Key present but wrong XDR type → host panics |

There is **no implicit default** for a missing key.  The host does not
silently skip unknown keys or zero-initialise missing ones.  Every mismatch
is a hard deserialisation error.

### Why this matters for StreamPay specifically

StreamPay streams are stored in **Soroban persistent storage** with a TTL of
~30 days, renewable indefinitely.  A stream created today could still be live
months or years from now.  A contract upgrade that changes `StreamInfo`
without a migration plan would:

1. Immediately break `get_stream_info`, `stop_stream`, `settle_stream`, and
   `archive_stream` for every stream written before the upgrade.
2. Provide no recovery path — the old binary data is still on-ledger but
   unreadable by the new contract.
3. Potentially lock funds permanently if `settle_stream` and `archive_stream`
   can no longer execute.

This is not a theoretical risk.  Issue #8 (`recipient_can_stop`) is exactly
the kind of field addition that would trigger this failure if deployed without
the migration infrastructure this PR establishes.

---

## Design decisions

### Sentinel field vs. external version map

Two approaches were considered for tracking schema version:

**Option A — external version map in instance storage**
Store a `Map<u32, u32>` (stream_id → schema_version) in instance storage.
Rejected because: instance storage has a single TTL shared across all entries;
a stream that outlives the instance storage TTL would lose its version record.
Also adds a second storage read on every stream access.

**Option B — `schema_version` field inside `StreamInfo` (chosen)**
Store the version directly in the struct.  It travels with the entry, has the
same TTL as the stream itself, and requires no extra storage reads.  The only
cost is one additional `u32` field in the XDR map — negligible.

### Starting at version 1, not 0

`STREAM_SCHEMA_VERSION` is initialised to `1`.  This means a
zero-initialised or default-constructed `StreamInfo` (which Rust would give
`schema_version = 0`) is always detectable as invalid.  Any entry with
`schema_version == 0` can be treated as corrupted or uninitialised without
ambiguity.

### `schema_version` is immutable after creation

No contract function modifies `schema_version` after `create_stream` writes
it.  Migration functions (introduced in future upgrades) are the only code
that should ever rewrite it, and those require admin auth.  This makes the
field a reliable indicator of when the entry was written, not just what
version it currently claims to be.

---

## Files changed

### `src/lib.rs`

#### New constant: `STREAM_SCHEMA_VERSION`

```rust
/// Schema version stored inside every [`StreamInfo`] entry.
///
/// Increment this constant whenever a field is added to or removed from
/// [`StreamInfo`]. The value is written at creation time and can be read
/// back to detect entries that pre-date a schema change.
///
/// | Value | Fields present |
/// |-------|----------------|
/// | 1     | All fields in the initial StreamInfo definition (this release) |
///
/// See `docs/schema-versioning.md` for the full evolution policy.
const STREAM_SCHEMA_VERSION: u32 = 1;
```

This constant is the single source of truth for the current struct layout
version.  Bumping it is the first step in any future migration.

#### `StreamInfo` — new field `schema_version: u32`

```rust
pub struct StreamInfo {
    /// Schema version of this entry; set to [`STREAM_SCHEMA_VERSION`] at
    /// creation. Used to detect entries written by an older contract version.
    pub schema_version: u32,
    // ... existing fields
}
```

Written once by `create_stream` and never modified by any other contract
function.  Future code that depends on a field added in version N can guard
with:

```rust
assert!(info.schema_version >= N, "stream entry is stale; run migration first");
```

#### `StreamInfo` — comprehensive rustdoc

Every field now carries a doc comment covering:
- Semantic meaning and units (e.g. `start_time` is seconds since Unix epoch).
- Sentinel value and what it means (e.g. `start_time = 0` → never started,
  `end_time = 0` → never stopped or currently active).
- Relationship to other fields where relevant.

The struct-level doc block contains a four-rule schema-evolution warning that
links to `docs/schema-versioning.md`, so contributors encounter the policy
the moment they open the struct definition.

#### `create_stream`

The `StreamInfo` literal now includes `schema_version: STREAM_SCHEMA_VERSION`.
No other function constructs a `StreamInfo` from scratch, so this is the only
place the field needs to be set.

### `docs/schema-versioning.md` (new file)

A standalone policy document covering:

1. **Why schema versioning matters** — explains the XDR map serialisation
   model, the three categories of breaking change (add/remove/rename), and
   why StreamPay's long-lived persistent entries make this especially risky.

2. **The `schema_version` sentinel** — shows the constant, the version table,
   and how to use `schema_version` in guard assertions.

3. **Four rules for evolving `StreamInfo`**:
   - Never remove or rename existing fields.
   - Adding a new field requires a sentinel default and a migration function.
   - Bump `STREAM_SCHEMA_VERSION` with every struct change.
   - Update this document (version table + sentinel defaults table).

4. **The migration window problem** — explains why the old and new struct
   layouts cannot coexist in the same compiled contract, and why migration
   must happen in a dedicated transitional deployment before the final
   upgraded contract goes live.

5. **Sentinel defaults table** — one row per field, recording the version it
   was introduced and the safe default for pre-existing entries.  Future
   contributors append a row here when adding a field.

6. **Worked end-to-end migration example** — uses the `recipient_can_stop`
   field from issue #8 as a concrete scenario, showing:
   - How to choose a sentinel (`false` preserves payer-only behaviour).
   - The transitional contract with the migration function.
   - The final contract with the guard assertion.
   - The document update step.

7. **Soroban serialisation reference** — links to `rs-soroban-env` and
   `stellar-xdr` for the authoritative XDR specification.

### `.github/pr_schema_versioning.md` (this file)

Detailed PR description for review and audit trail.

---

## Security analysis

### `schema_version` cannot be manipulated by callers

The field is written by `create_stream` (which requires payer auth) and never
exposed as a writable parameter.  No public contract function accepts a
`schema_version` argument.  A caller cannot forge or downgrade the version of
an existing entry through normal contract invocations.

### Sentinel pattern does not expand the auth surface

The `schema_version` field and `STREAM_SCHEMA_VERSION` constant are purely
informational.  They do not gate any auth check and do not introduce any new
privileged operation.  Future migration functions will require admin auth (as
shown in the worked example), but that is a concern for the PR that introduces
those functions, not this one.

### Stale-entry detection prevents silent logic errors

Without `schema_version`, a contract that adds a boolean field and reads it
from an old entry would silently get a host panic rather than a controlled
error.  With `schema_version`, the contract can detect the stale entry before
attempting to read the new field and surface a clear, actionable error message
(`"stream entry is stale; run migration first"`).

### Zero-version sentinel catches uninitialised entries

Starting at version `1` means any entry with `schema_version == 0` is
unambiguously invalid.  This guards against bugs where a `StreamInfo` is
constructed without explicitly setting `schema_version` (Rust's default
numeric initialisation is `0`).

---

## Test coverage

16 tests total, all passing.  Four new tests cover the schema versioning
feature specifically.

### New tests

| Test | Assertion | Why it matters |
|------|-----------|----------------|
| `test_stream_schema_version_is_current` | `info.schema_version == STREAM_SCHEMA_VERSION` after `create_stream` | Verifies the field is written correctly at creation |
| `test_stream_schema_version_is_positive` | `info.schema_version > 0` | Guards against zero-init bugs |
| `test_stream_schema_version_survives_lifecycle` | `schema_version` unchanged after start → settle → stop | Confirms no lifecycle function accidentally overwrites the field |
| `test_schema_version_const_value` | `STREAM_SCHEMA_VERSION == 1` | Regression guard; catches accidental constant changes |

### Full test output

```
running 16 tests
test test::test_schema_version_const_value ... ok
test test::test_version_matches_const ... ok
test test::test_version_is_positive ... ok
test test::test_stream_schema_version_is_positive ... ok
test test::test_version_returns_expected ... ok
test test::test_archive_settled_stream ... ok
test test::test_stream_schema_version_is_current ... ok
test test::test_stream_uses_persistent_storage ... ok
test test::test_archive_unsettled_stream_panics - should panic ... ok
test test::test_create_stream_extends_ttl ... ok
test test::test_create_stream_valid ... ok
test test::test_archive_active_stream_panics - should panic ... ok
test test::test_settle_returns_amount ... ok
test test::test_stream_schema_version_survives_lifecycle ... ok
test test::test_start_and_stop_stream ... ok
test test::test_archived_stream_not_found - should panic ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Coverage gaps and justification

The one scenario not covered by an automated test is the actual migration
path (reading a v1 entry with a v2 struct).  This cannot be tested in the
current contract because the struct has only one version.  The worked example
in `docs/schema-versioning.md` documents the pattern; the test coverage
requirement for that path belongs to the PR that introduces the first schema
change.

---

## Checklist

- [x] `STREAM_SCHEMA_VERSION` constant added and documented
- [x] `schema_version` field added to `StreamInfo`
- [x] `create_stream` writes `schema_version: STREAM_SCHEMA_VERSION`
- [x] All existing lifecycle functions leave `schema_version` untouched
- [x] Rustdoc on `StreamInfo` struct and every field
- [x] `docs/schema-versioning.md` written and covers all four rules
- [x] Sentinel defaults table populated for all v1 fields
- [x] Worked migration example included in policy doc
- [x] Four new tests added and passing
- [x] All 16 tests pass (`cargo test`)
- [x] No existing tests broken

---

Closes issue #65
