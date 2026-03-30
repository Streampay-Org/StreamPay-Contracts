# docs(contracts): StreamInfo schema evolution guidelines

## Summary

This PR implements issue #9 — documenting and enforcing a schema versioning
strategy for `StreamInfo` so that future field additions do not silently break
existing ledger entries.

The deliverables are:

1. **`docs/schema-versioning.md`** — comprehensive policy document covering
   Soroban serialisation rules, the sentinel-default pattern, a worked
   migration example, and a versioned field table.
2. **`StreamInfo` rustdoc** — inline documentation on every field plus a
   schema-evolution warning block that links to the policy doc.
3. **`STREAM_SCHEMA_VERSION` constant** — a `u32` sentinel written into every
   new `StreamInfo` entry so stale entries are detectable at runtime.
4. **`schema_version` field on `StreamInfo`** — stored in persistent ledger
   alongside all other stream metadata; set to `STREAM_SCHEMA_VERSION` at
   creation and preserved across all lifecycle operations.
5. **Four new tests** — covering sentinel value, positivity, lifecycle
   preservation, and constant regression.

---

## Motivation

Soroban serialises `#[contracttype]` structs as XDR maps keyed by field name.
Adding or removing a field is a **hard breaking change**: any ledger entry
written before the upgrade will fail to deserialise into the new struct,
causing a host panic on every read.  Because StreamPay streams live in
persistent storage with TTLs up to 30 days (renewable indefinitely), a
contract upgrade that changes `StreamInfo` without a migration plan would
silently brick every live stream.

This PR establishes the guardrails to prevent that from happening.

---

## Changes

### `src/lib.rs`

**New constant: `STREAM_SCHEMA_VERSION`**

```rust
/// Bump this whenever StreamInfo fields change.
const STREAM_SCHEMA_VERSION: u32 = 1;
```

Tracks the current struct layout version.  Any code that reads a field added
in version N can assert `info.schema_version >= N` before using it.

**`StreamInfo` — new field `schema_version: u32`**

Written at creation time via `create_stream`.  Preserved unchanged by
`start_stream`, `stop_stream`, `settle_stream`, and `archive_stream`.
Enables runtime detection of stale entries written by an older contract.

**`StreamInfo` — comprehensive rustdoc**

Every field now has a doc comment explaining its semantics, units, and
sentinel value (e.g. `start_time = 0` means "never started").  The struct
doc block contains a schema-evolution warning with the four rules contributors
must follow when changing the struct.

**`create_stream`**

Sets `schema_version: STREAM_SCHEMA_VERSION` in the newly created `StreamInfo`.

### `docs/schema-versioning.md`

New document covering:

- Why schema versioning matters in Soroban (XDR map serialisation, no implicit
  defaults for missing keys).
- The four rules for evolving `StreamInfo` safely.
- The sentinel defaults table (one row per field, with the version it was
  introduced and the safe default for pre-existing entries).
- A worked end-to-end migration example for adding `recipient_can_stop: bool`
  (the scenario from issue #8), showing the transitional contract pattern.
- Soroban serialisation reference links.

---

## Security notes

- The `schema_version` field is **read-only after creation** — no contract
  function modifies it post-write, so it cannot be tampered with through
  normal contract calls.
- The sentinel pattern does not introduce any new auth surface.  Migration
  functions (when needed in future) should require admin auth, as shown in
  the worked example.
- Stale-entry detection via `schema_version` prevents logic errors that could
  arise from reading a field that was absent when the entry was written (e.g.
  treating a missing boolean as `false` when the correct sentinel is
  something else).

---

## Tests

16 tests total, all passing (`cargo test`).

New tests added for schema versioning:

| Test | What it verifies |
|------|-----------------|
| `test_stream_schema_version_is_current` | Newly created stream has `schema_version == STREAM_SCHEMA_VERSION` |
| `test_stream_schema_version_is_positive` | `schema_version` is never zero (guards against accidental zero-init) |
| `test_stream_schema_version_survives_lifecycle` | `schema_version` is unchanged after start → settle → stop round-trip |
| `test_schema_version_const_value` | `STREAM_SCHEMA_VERSION == 1` regression guard |

### Test output

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

---

## Edge cases covered

- `schema_version` is written at creation and never mutated by any other
  contract function — verified by the lifecycle round-trip test.
- `STREAM_SCHEMA_VERSION` starts at `1` (not `0`) so a zero-initialised
  entry is always detectable as invalid.
- The policy doc explicitly covers the "migration window" problem: the old
  and new struct layouts cannot coexist in the same compiled contract, so
  migration must happen in a transitional deployment.

---

Closes issue #9
