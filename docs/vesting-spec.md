# StreamPay — Linear Vesting Specification

**Document:** `docs/vesting-spec.md`  
**Contract:** `StreamPay-Contracts` (Soroban / Rust)  
**Version:** 0.2.0 (`VERSION = 2_000`)  
**Status:** Normative — matches `src/lib.rs` and `src/stream.rs`

---

## 1. Overview

Linear vesting streams unlock a fixed `total_amount` evenly over
`duration_seconds`. Unlike rate-per-second (`StreamMode::Linear`) streams,
vesting does **not** use `rate_per_second`; accrual is driven entirely by the
`StreamMode::LinearVesting` schedule.

Create vesting streams with `create_vesting_stream(payer, recipient,
total_amount, duration_seconds)`.

---

## 2. State

Vesting metadata lives in `StreamInfo.mode`:

```rust
StreamMode::LinearVesting {
    duration_seconds: u64,
    vested_amount: i128,      // cumulative amount already settled
    schedule_anchor: u64,       // first start_time (0 until started)
}
```

| Field | Meaning |
|---|---|
| `duration_seconds` | Total vesting period in ledger seconds |
| `vested_amount` | Tokens already moved from `balance` to `claimable_balance` |
| `schedule_anchor` | Unix timestamp when the vesting clock began (set on first `start_stream`) |

---

## 3. Formula

Total vested at elapsed seconds `e` from the schedule anchor:

```
vested_total = total_amount × min(e, duration_seconds) ÷ duration_seconds
```

Implemented as:

```rust
fn compute_linear_vested(total_amount: i128, duration_seconds: u64, elapsed_seconds: u64) -> i128 {
    let capped = min(elapsed_seconds, duration_seconds);
    total_amount.saturating_mul(capped as i128) / duration_seconds as i128
}
```

On each settlement, the **incremental** unlock is:

```
delta = vested_total(now − schedule_anchor) − vested_amount
settled = min(delta, balance)
```

Integer division truncates toward zero; any rounding remainder is released on
the final second of the schedule (see §5 Example 2).

---

## 4. Schedule anchoring

The vesting clock is anchored to the **first** `start_stream` call:

1. `schedule_anchor` is set to `now` when the stream starts for the first time.
2. Subsequent `stop_stream` / `start_stream` cycles do **not** reset the anchor.
3. Elapsed vesting time includes seconds while the stream is stopped.

This ensures a recipient cannot extend the schedule by pausing the stream.

---

## 5. Worked examples

### Example 1 — Partial unlock

```
total_amount      = 1_000
duration_seconds  = 100
schedule_anchor   = T₀

At T₀ + 10s:  vested_total = 100,  delta = 100
At T₀ + 50s:  vested_total = 500,  delta = 400 (after first settle of 100)
```

### Example 2 — Rounding remainder

```
total_amount = 1_000, duration = 3

T₀ + 1s → 1000×1/3 = 333
T₀ + 2s → 1000×2/3 = 666, delta = 333
T₀ + 3s → 1000×3/3 = 1000, delta = 334  (remainder released)
```

### Example 3 — Full unlock past duration

```
duration = 10, advance 15s → vested_total = 1_000 (capped at duration)
```

---

## 6. Interaction with other entry points

| Entry point | Vesting behaviour |
|---|---|
| `settle_stream` | Unlocks incremental `delta` into `claimable_balance` |
| `withdraw_stream` | Settles then transfers `claimable_balance` on-ledger |
| `update_rate` | N/A — vesting streams have `rate_per_second = 0` |
| `archive_stream` | Requires `balance == 0` and `claimable_balance == 0` |

---

## 7. Invariants

1. `vested_amount ≤ total_amount` at all times.
2. `balance + vested_amount == total_amount` (tokens are never conjured).
3. `compute_linear_vested` uses saturating multiply — no silent wrap.
4. `schedule_anchor` is immutable after the first start.
