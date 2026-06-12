# Persistent Storage TTL Strategy

Soroban entries can expire if they are not periodically refreshed. This
document explains how `streampay-contracts` keeps streams and the instance
counter alive without burning resource budget unnecessarily.

## Constants

| Constant | Value | Approx. real time |
|---|---|---|
| `STREAM_TTL_THRESHOLD` | `17_280` ledgers | ~1 day |
| `STREAM_TTL_EXTEND` | `518_400` ledgers | ~30 days |
| `INSTANCE_TTL_THRESHOLD` | `17_280` ledgers | ~1 day |
| `INSTANCE_TTL_EXTEND` | `518_400` ledgers | ~30 days |

These are computed assuming the canonical ~5-second ledger cadence on
Stellar mainnet. They are deliberately identical so that a stream whose
entry is bumped is guaranteed to outlast the instance counter that
addresses it.

## When the TTL is bumped

The contract calls `extend_stream_ttl` after every successful state
transition on a stream:

- `create_stream`
- `start_stream`
- `stop_stream`
- `settle_stream` (and each iteration of `batch_settle`)
- `withdraw_stream`

The instance entry is bumped on every write through `extend_instance_ttl`.

## Why threshold + extend-to?

The Soroban `extend_ttl(threshold, extend_to)` API is conditional: it only
extends the entry if the remaining TTL is below `threshold`. That avoids
paying for a TTL bump on every transaction; we only pay roughly once per
day in the common case.

## Operational guidance

- A stream that is idle for more than 30 days may be archived by network
  garbage collection. Off-chain systems polling `get_stream_info` should
  treat an `Err(EntryArchived)`-style failure as "needs restoration", not
  as "stream no longer exists".
- Long-dated payroll streams should be touched at least monthly. The
  cheapest way is a no-op `settle_stream` call from any account.
