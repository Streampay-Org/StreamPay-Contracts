# StreamPay — Formal Accrual Formula Specification

**Document:** `docs/accrual-spec.md`  
**Contract:** `StreamPay-Contracts` (Soroban / Rust)  
**Version:** 0.2.0 (`VERSION = 2_000`)
**Status:** Normative — matches `src/lib.rs` line-by-line

---

## Current implementation note

This document now tracks the current `src/lib.rs` surface: `VERSION = 2_000`,
pause/resume entrypoints, SEP-41 token custody on creation and withdrawal, and
the `claimable_balance` settlement model. `settle_stream` moves accrued value
from `balance` into `claimable_balance`; `withdraw_stream` performs the
recipient-authorized on-ledger transfer and resets claimable funds to zero.

Linear vesting streams are covered separately in
[`vesting-spec.md`](./vesting-spec.md).

## 1. Overview

A StreamPay stream is a continuous payment channel that accrues value from a
`payer` to a `recipient` at a fixed `rate_per_second` (in the smallest token
unit, e.g. stroops). Accrual begins when the stream is **started** and is
realised on each call to `settle_stream`. The contract is intentionally minimal:
no pauses, no cliffs, pure linear accrual capped by the deposited balance.

---

## 2. State Variables

Each stream stores the following fields (see `StreamInfo`):

| Field | Type | Description |
|---|---|---|
| `payer` | `Address` | Account that funds the stream and controls it |
| `recipient` | `Address` | Beneficiary of the streamed payments |
| `rate_per_second` | `i128` | Token units accrued per second; must be > 0 |
| `balance` | `i128` | Remaining deposited balance; must be > 0 at creation |
| `start_time` | `u64` | Ledger timestamp (seconds) of the last start or settle |
| `end_time` | `u64` | Ledger timestamp (seconds) when the stream was stopped |
| `is_active` | `bool` | `true` iff the stream is currently streaming |

> **Note on `start_time` semantics.** After each `settle_stream` call,
> `start_time` is **reset to `now`**. It therefore represents the beginning of
> the *current unsettled window*, not the stream's original creation time.

---

## 3. Accrual Formula

### 3.1 Elapsed Time

For a stream that is active and has `start_time = T₀`, queried at ledger
timestamp `now`:

```
elapsed = now − start_time
```

Both values are `u64` seconds from the Soroban ledger clock
(`env.ledger().timestamp()`). Subtraction is plain unsigned arithmetic; since
`now ≥ start_time` is guaranteed by the ledger's monotonic clock, no underflow
can occur.

### 3.2 Raw Accrued Amount

```
raw_accrued = elapsed × rate_per_second
```

This is a **saturating multiply** on `i128`:

```rust
let amount = (elapsed as i128)
    .saturating_mul(info.rate_per_second)
    .min(info.balance);
```

`saturating_mul` means that if the product would exceed `i128::MAX`
(≈ 1.7 × 10³⁸), it clamps to `i128::MAX` rather than wrapping. In practice,
with `elapsed` in seconds and `rate_per_second` in stroops, overflow is
unreachable, but the guard is present.

### 3.3 Balance Cap

The accrued amount is capped at the remaining balance:

```
accrued = min(raw_accrued, balance)
```

This ensures the stream never pays out more than it holds, even if the stream
is left running past exhaustion.

### 3.4 Complete Formula (single expression)

```
accrued(start_time, now, rate_per_second, balance) =
    min( (now − start_time) × rate_per_second , balance )
```

All variables are non-negative integers. The result is a non-negative integer
in the same token-unit as `rate_per_second` and `balance`.

---

## 4. State Transition on Settlement

`settle_stream` applies the formula and mutates state atomically:

```
balance'    = balance − accrued
start_time' = now
```

In Rust (from `settle_stream`):

```rust
info.balance = info.balance.saturating_sub(amount);
info.start_time = now;
```

`saturating_sub` is used for safety; given the cap in §3.3 it can never
actually saturate (amount ≤ balance), but the guard is present.

After settlement the stream remains **active**. To stop streaming, the payer
must call `stop_stream` explicitly.

---

## 5. Lifecycle & Formula Applicability

```
create_stream → [inactive, start_time=0]
     ↓  start_stream
[active, start_time=T₁]   ←──────────────────────────────┐
     ↓  settle_stream(now=T₂)                            │
[active, start_time=T₂, balance reduced]  ───────────────┘
     ↓  stop_stream(now=T₃)
[inactive, end_time=T₃]
     ↓  settle_stream (no-op: is_active=false → returns 0)
     ↓  archive_stream (requires balance=0)
[removed from storage]
```

The accrual formula in §3.4 is only evaluated when `is_active = true`.
When `is_active = false`, `settle_stream` returns `0` immediately without
touching state.

---

## 6. Worked Examples

### Example 1 — Partial settlement

```
rate_per_second = 10
balance         = 1_000
start_time      = 1_000_000   (Unix-style seconds, Soroban ledger)
now             = 1_000_050   (50 seconds later)

elapsed         = 1_000_050 − 1_000_000 = 50
raw_accrued     = 50 × 10 = 500
accrued         = min(500, 1_000) = 500

balance'        = 1_000 − 500 = 500
start_time'     = 1_000_050
```

### Example 2 — Balance exhaustion (cap fires)

```
rate_per_second = 100
balance         = 1_000
start_time      = 0
now             = 20   (20 seconds later)

elapsed         = 20 − 0 = 20
raw_accrued     = 20 × 100 = 2_000
accrued         = min(2_000, 1_000) = 1_000   ← cap fires

balance'        = 1_000 − 1_000 = 0
start_time'     = 20
```

This mirrors `test_archive_settled_stream` in the test suite:
`rate=100, balance=1_000, elapsed=10 → amount=1_000`.

### Example 3 — Zero elapsed (same ledger timestamp)

```
rate_per_second = 50
balance         = 5_000
start_time      = 42
now             = 42   (settle called in same ledger second)

elapsed         = 0
raw_accrued     = 0 × 50 = 0
accrued         = min(0, 5_000) = 0

balance'        = 5_000   (unchanged)
start_time'     = 42      (unchanged in effect)
```

### Example 4 — Multiple sequential settlements

```
rate_per_second = 10
balance         = 300

Settlement 1: start_time=0,  now=10  → accrued=100, balance=200, start_time'=10
Settlement 2: start_time=10, now=20  → accrued=100, balance=100, start_time'=20
Settlement 3: start_time=20, now=30  → accrued=100, balance=0,   start_time'=30
Settlement 4: start_time=30, now=40  → accrued=min(100,0)=0,    balance=0
```

---

## 7. Constraints and Invariants

| Invariant | Source |
|---|---|
| `rate_per_second > 0` | Enforced by `create_stream` panic |
| `balance > 0` at creation | Enforced by `create_stream` panic |
| `balance ≥ 0` always | `saturating_sub` + cap guarantee |
| `accrued ≤ balance` | `min(raw, balance)` in §3.3 |
| `start_time` monotonically non-decreasing | Reset to `now` on settle; ledger clock is monotonic |
| Archival only when `balance = 0` and `is_active = false` | `archive_stream` guards |

---

## 8. What This Spec Does NOT Cover

The current implementation (`v0.1.0`) deliberately omits:

- **Pauses** — there is no pause/resume mechanism; `stop_stream` is terminal
  until a new `start_stream` call.
- **Cliffs** — accrual begins immediately from `start_time`; no cliff period.
- **Token transfers** — `settle_stream` updates the on-chain balance field but
  does not invoke a token contract transfer. Actual disbursement is handled at
  the application layer.
- **Multi-asset streams** — a single stream carries a single implicit asset.

---

## 9. Version History

| Version | Change |
|---|---|
| 0.1.0 | Initial specification; linear accrual, no pauses or cliffs |
