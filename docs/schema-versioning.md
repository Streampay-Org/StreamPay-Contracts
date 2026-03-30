# StreamInfo Schema Versioning

This document describes how `StreamInfo` — the core `#[contracttype]` struct
stored in Soroban persistent storage — evolves safely across contract upgrades.

---

## Why schema versioning matters

Soroban serialises `#[contracttype]` structs as **XDR maps keyed by field
name** (a `Symbol`).  When the contract is upgraded and the struct gains or
loses a field, any ledger entry written by the old contract will no longer
deserialise cleanly into the new struct:

- **Added field** — the new key is absent in the old map; the host panics.
- **Removed field** — the old key is present but the new struct has no slot
  for it; the host panics.
- **Renamed field** — equivalent to a remove + add; same result.

Because Soroban persistent entries can live for months (TTL up to ~30 days,
renewable indefinitely), a contract upgrade that changes `StreamInfo` without
a migration plan will silently brick every existing stream.

---

## The `schema_version` sentinel

Every `StreamInfo` entry carries a `schema_version: u32` field set to the
`STREAM_SCHEMA_VERSION` constant at creation time.

```rust
/// Bump this whenever StreamInfo fields change.
const STREAM_SCHEMA_VERSION: u32 = 1;
```

| Version | Fields present |
|---------|----------------|
| 1       | `schema_version`, `payer`, `recipient`, `rate_per_second`, `balance`, `start_time`, `end_time`, `is_active` |

Reading code can compare `info.schema_version` against `STREAM_SCHEMA_VERSION`
to detect stale entries and decide whether migration is required before
proceeding.

---

## Rules for evolving `StreamInfo`

### 1 — Never remove or rename existing fields

Removing a field breaks deserialisation of every entry written before the
change.  Renaming is identical to a remove + add.  Both are **hard breaking
changes** with no safe in-place recovery.

If a field is truly obsolete, keep it in the struct and stop writing
meaningful values to it (document it as deprecated).  Remove it only after
all live entries have been migrated and archived.

### 2 — Adding a new field requires a sentinel default

When a new field must be added:

1. Choose a **sentinel value** that is safe and meaningful for old entries
   that lack the field (e.g. `false` for a boolean flag, `0` for a counter,
   `None` for an `Option`).
2. Document the sentinel in the table below.
3. Add a `migrate_stream(stream_id)` admin function (or equivalent) that:
   - Reads the old entry (which will panic if the struct already changed —
     see the migration window note below).
   - Writes it back with the new field set to the sentinel.
4. Run the migration for all live entries **before** deploying any contract
   logic that reads the new field.
5. Bump `STREAM_SCHEMA_VERSION` and update the table in this file.

> **Migration window**: the old and new struct definitions cannot coexist in
> the same compiled contract.  The migration function must be deployed in a
> **transitional contract version** that still matches the old struct layout
> but writes the new field.  Only after all entries are migrated should the
> final version (with the new field in the struct) be deployed.

### 3 — Bump `STREAM_SCHEMA_VERSION` with every struct change

This makes stale entries detectable at runtime.  Any function that reads a
`StreamInfo` and depends on a field added in version N should assert:

```rust
assert!(
    info.schema_version >= N,
    "stream entry is stale; run migration first"
);
```

### 4 — Update this document

Add a row to the version table above and describe the sentinel default for
every new field in the sentinel defaults table below.

---

## Sentinel defaults table

| Field | Added in version | Sentinel for pre-existing entries | Rationale |
|-------|-----------------|-----------------------------------|-----------|
| `schema_version` | 1 | N/A — present from the start | Enables all future migration detection |
| `payer` | 1 | N/A | Core field, always present |
| `recipient` | 1 | N/A | Core field, always present |
| `rate_per_second` | 1 | N/A | Core field, always present |
| `balance` | 1 | N/A | Core field, always present |
| `start_time` | 1 | N/A | Core field, always present |
| `end_time` | 1 | N/A | Core field, always present |
| `is_active` | 1 | N/A | Core field, always present |

When adding a new field, append a row here with its sentinel value and the
rationale for that choice.

---

## Worked example: adding `recipient_can_stop: bool`

Suppose we want to add a boolean flag allowing the recipient to stop a stream
(see issue #8).

**Step 1 — choose sentinel**: `false`.  Old streams should default to
payer-only behaviour, so `false` is the safe sentinel.

**Step 2 — transitional contract** (version 1 → 1.1):

```rust
// StreamInfo still has the OLD layout (no recipient_can_stop).
// New migration function:
pub fn migrate_stream_v1_to_v2(env: Env, stream_id: u32) {
    let admin = get_admin(&env);
    admin.require_auth();
    // Read with old layout — still works because struct hasn't changed yet.
    let old: StreamInfoV1 = get_stream_v1(&env, stream_id);
    let new = StreamInfoV2 {
        schema_version: 2,
        recipient_can_stop: false, // sentinel
        payer: old.payer,
        // ... copy remaining fields
    };
    set_stream(&env, stream_id, &new);
}
```

**Step 3 — final contract** (version 2):

```rust
const STREAM_SCHEMA_VERSION: u32 = 2;

pub struct StreamInfo {
    pub schema_version: u32,
    pub recipient_can_stop: bool,  // new field
    // ... existing fields
}

pub fn stop_stream(env: Env, stream_id: u32, stopper: Address) {
    let info = get_stream(&env, stream_id);
    assert!(info.schema_version >= 2, "run migration first");
    // ... auth and logic using recipient_can_stop
}
```

**Step 4 — update this document** with the new version row and sentinel entry.

---

## Soroban serialisation reference

- Fields are serialised as an XDR `ScMap` sorted by key symbol.
- The host matches map keys to struct fields by name at deserialisation time.
- There is no implicit default for missing keys — a missing key is a hard
  deserialisation error.
- Field order in the Rust struct does not affect the serialised key order
  (keys are sorted lexicographically by the host).

For the authoritative specification see the
[Soroban Environment Interface](https://github.com/stellar/rs-soroban-env)
and the [Stellar XDR definitions](https://github.com/stellar/stellar-xdr).
