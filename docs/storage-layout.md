# Storage Layout — StreamPay Contracts

This document is the authoritative reference for every key written to Soroban
storage by the `streampay-contracts` crate.  It is intended for auditors,
integrators, and future maintainers.

---

## Storage tiers used

| Tier | Soroban API | Persistence |
|------|-------------|-------------|
| Instance | `env.storage().instance()` | Lives as long as the contract instance; one shared TTL for all keys in this tier. |
| Persistent | `env.storage().persistent()` | Per-key TTL; survives ledger archival if TTL is extended regularly. |

---

## Instance storage keys

Instance storage holds lightweight, contract-wide state.  All keys in this
tier share the contract instance's TTL and are extended together by
`extend_instance_ttl`.

### `"next_id"` — `Symbol`

| Field | Value |
|-------|-------|
| Key type | `Symbol::new(env, "next_id")` |
| Value type | `u32` |
| Default (absent) | `1` |
| Written by | `create_stream` (via `set_next_stream_id`) |
| Read by | `create_stream` (via `get_next_stream_id`) |

Stores the next stream ID to be assigned.  Starts at `1` (the first stream
created gets ID `1`).  Incremented by one on every successful `create_stream`
call.  IDs are monotonically increasing and never reused, even after a stream
is archived.

**Migration note:** If this key is renamed or its type changes, all existing
counters must be migrated in a contract upgrade before any new stream can be
created.  A missing key is treated as `1`, so a fresh deployment is safe
without an explicit initialisation step.

---

## Persistent storage keys

Each stream occupies exactly one persistent storage entry.  The key is a
two-element tuple so that stream records are namespaced away from any future
top-level persistent keys.

### `("stream", stream_id)` — `(Symbol, u32)`

| Field | Value |
|-------|-------|
| Key type | `(Symbol::new(env, "stream"), stream_id: u32)` |
| Value type | `StreamInfo` (see below) |
| Written by | `create_stream`, `start_stream`, `stop_stream`, `settle_stream` |
| Removed by | `archive_stream` |
| Read by | `get_stream_info`, and every mutating entry-point via `get_stream` |

One entry exists per live stream.  The entry is removed (not zeroed) by
`archive_stream`, so a missing key unambiguously means the stream never
existed or has been archived.

#### `StreamInfo` fields

```rust
pub struct StreamInfo {
    pub schema_version:    u32,     // storage schema sentinel
    pub payer:             Address,  // account that funded the stream; authorises mutations
    pub recipient:         Address,  // beneficiary of streamed funds
    pub token:             Address,  // SEP-41 token contract
    pub rate_per_second:   i128,     // tokens streamed per second (linear mode)
    pub balance:           i128,     // remaining unstreamed balance
    pub claimable_balance: i128,     // earned but not yet withdrawn
    pub start_time:        u64,      // ledger timestamp when stream was last started/settled
    pub end_time:          u64,      // stop timestamp or optional auto-terminate time
    pub is_active:         bool,     // true while the stream is running
    pub paused_at:         u64,      // pause timestamp (0 if not paused)
    pub memo:              String,   // immutable off-chain correlation string
    pub recipient_can_stop: bool,    // recipient may call stop_stream
    pub mode:              StreamMode, // Linear or LinearVesting
}
```

**Invariants enforced by the contract:**

- `rate_per_second > 0` and `balance > 0` at creation time.
- `archive_stream` requires `!is_active && balance == 0`; this protects the
  recipient's entitlement — a stream with a non-zero balance cannot be deleted
  by the payer.
- `settle_stream` uses `saturating_mul` / `saturating_sub` to prevent
  arithmetic overflow on extreme values.

**Migration note:** Adding fields to `StreamInfo` is a breaking change for
existing encoded entries.  A version-gated migration helper or a new key
prefix must be introduced before deploying such a change.

---

## TTL strategy

All TTL constants are defined at the top of `src/lib.rs`:

| Constant | Ledgers | Approximate wall-clock time (@ 5 s/ledger) |
|----------|---------|---------------------------------------------|
| `STREAM_TTL_THRESHOLD` | 17 280 | ~1 day |
| `STREAM_TTL_EXTEND` | 518 400 | ~30 days |
| `INSTANCE_TTL_THRESHOLD` | 17 280 | ~1 day |
| `INSTANCE_TTL_EXTEND` | 518 400 | ~30 days |

Every mutating entry-point (`create_stream`, `start_stream`, `stop_stream`,
`settle_stream`, `archive_stream`) calls both `extend_stream_ttl` (for the
affected persistent key) and `extend_instance_ttl` (for the instance tier).
This means TTLs are refreshed on every interaction, so an actively-used stream
will never expire.

**Version byte strategy:** The contract does not currently embed a version
byte inside storage values.  The `VERSION` constant (`u32`, encoded as
`major * 1_000_000 + minor * 1_000 + patch`) is exposed via the `version()`
entry-point for off-chain tooling.  If a future upgrade requires on-chain
value migration, the recommended approach is to introduce a new key prefix
(e.g. `("stream_v2", stream_id)`) and migrate lazily on first access, or
eagerly in a one-shot migration entry-point protected by admin auth.

---

## Key inventory (summary)

| Storage tier | Key | Value type | Purpose |
|--------------|-----|------------|---------|
| Instance | `"next_id"` | `u32` | Monotonic stream ID counter |
| Persistent | `("stream", id)` | `StreamInfo` | Per-stream state |

---

## Security notes for auditors

1. **Payer-only mutations** — `start_stream`, `stop_stream`, and
   `archive_stream` all call `payer.require_auth()`.  `settle_stream` is
   intentionally permissionless so that any party can trigger settlement.
2. **No key collision** — instance key `"next_id"` and persistent key prefix
   `"stream"` are distinct in both name and storage tier, so there is no
   possibility of collision.
3. **Archive guard** — the dual check (`is_active == false && balance == 0`)
   in `archive_stream` ensures storage cannot be used to silently destroy a
   recipient's unclaimed balance.
4. **Overflow safety** — `settle_stream` uses `saturating_mul` and
   `saturating_sub`; the streamed amount is also capped at `info.balance`,
   preventing over-settlement.
5. **No wildcard removal** — the contract never calls `storage().persistent().remove`
   in a loop or on a computed range; only the exact key for the targeted
   stream is removed.
