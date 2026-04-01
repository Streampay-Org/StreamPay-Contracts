# Ledger Timestamp Assumptions for Accrual

> **Crate:** `streampay-contracts`  
> **Branch:** `docs/ledger-timestamp-assumptions`  
> **Commit:** `docs(contracts): ledger timestamp assumptions for accrual`

---

## Overview

StreamPay uses `Env::ledger().timestamp()` as the sole on-chain time source for all
time-based logic — stream start/end enforcement, elapsed-time calculation, and
claimable-amount math. This document explains what that value is, how validators
produce it, its granularity limits, and what downstream developers or UX layers
must account for.

**No contract code is changed by this document.** If a bug is identified during
review, a separate fix commit will be raised.

---

## 1. What Is `Env::ledger().timestamp()`?

| Property | Value |
|---|---|
| **Type** | `u64` |
| **Unit** | Unix seconds (whole seconds since 1970-01-01T00:00:00 UTC) |
| **Source** | Stellar Consensus Protocol (SCP) ledger close time |
| **Typical cadence** | ~5–6 s per ledger on Stellar mainnet |
| **Sub-second precision** | None |
| **Monotonicity** | Guaranteed non-decreasing by protocol rules |
| **Caller influence** | None — set by validator quorum, not by any transaction sender |

---

## 2. Validator Behavior

### 2.1 How the value is agreed

During each SCP round, validators propose a candidate close time. The protocol
selects a consensus value (roughly the median of validator proposals). No
single validator can push the timestamp arbitrarily forward or backward.

Protocol constraints enforced by every node:

1. `new_close_time >= previous_close_time` — strictly non-decreasing.
2. `new_close_time <= node_wall_clock + tolerance` — bounded ahead-drift (a few seconds).

### 2.2 Practical drift characteristics

- **Short-term drift:** ±few seconds from real UTC wall time is normal.
- **Backward jumps:** Impossible — the protocol rejects them.
- **Forward stalls:** During a network partition or quorum disruption the
  timestamp can pause; it will resume from the last agreed value once the
  network recovers.
- **Intra-ledger ordering:** All transactions in the same ledger share the
  identical timestamp. There is no per-transaction time resolution within a
  single ledger close.

### 2.3 Security implication

Because no external caller controls the timestamp, griefing via timestamp
manipulation is not possible from within the contract. The attack surface is
limited to network-level events (stall, partition) that are outside any single
actor's control.

---

## 3. Coarse Granularity — Implications for Accrual

### 3.1 Integer-second steps

`timestamp()` returns whole seconds. The core accrual math in the contract is:

\`\`\`rust
let elapsed: u64 = now.saturating_sub(last_time); // both are u64 seconds
let claimable: i128 = rate_per_second as i128 * elapsed as i128;
\`\`\`

There are no fractional seconds anywhere in this path. The minimum observable
time step on-chain is 1 second; the practical minimum between consecutive
user-visible state changes is one ledger close (~5 s).

### 3.2 Sub-second accrual is impossible on-chain

Any stream parameterised with a rate intended to express sub-second precision
(e.g., "1 unit per 100 ms") cannot be represented faithfully. The contract will
always truncate to whole-second windows.

### 3.3 Rounding behaviour by stream duration

| Stream lifetime | Ledger closes | Rounding concern |
|---|---|---|
| Hours / days | Thousands | Negligible — drift is < 0.01 % of total |
| 1–10 minutes | ~10–120 | Minor — last partial second silently truncated |
| < 60 seconds | ~10–12 | Noticeable — up to ~5 s of un-accrued tail |
| < 10 seconds | 0–2 | High — stream may close before a second close occurs |

### 3.4 The "dust" tail problem

When a stream ends (`end_time` reached) there may be a sub-ledger-cadence
remainder that is never claimable. The contract computes:

\`\`\`
claimable = min(elapsed_seconds, stream_duration) * rate_per_second
\`\`\`

Any fractional-second remainder between the last ledger close before `end_time`
and `end_time` itself is permanently unclaimable by the recipient. The sender's
cancel/withdraw path will reclaim it on stream close.

---

## 4. Off-Chain Rounding Recommendations (UX)

### 4.1 Recommended approach

\`\`\`
displayed_balance = floor(elapsed_seconds) * rate_per_second
\`\`\`

where `elapsed_seconds` is computed from the last known ledger close time
(fetched from Horizon or RPC), not from the device wall clock.

Steps:
1. Fetch the latest ledger sequence and its `close_time` from a Horizon or RPC node.
2. Estimate current elapsed time as `close_time + (expected_cadence * ledgers_since)`.
3. Display the floor of that estimate, not a live wall-clock interpolation.

### 4.2 Why not wall-clock interpolation?

Wall-clock interpolation shows a steadily incrementing balance between ledger
closes. This is misleading — the on-chain claimable amount does not increase
until the next ledger close. Users who claim based on an interpolated display
may receive less than shown.

### 4.3 Displaying the unclaimed tail

For streams near their `end_time`, surface a note such as:

> "Up to ~5 s of stream value may not be claimable due to ledger timing.
> Any remainder is returned to the sender on stream close."

---

## 5. Edge Cases

### 5.1 `start_time == end_time`
Zero duration stream. No tokens ever accrue. UIs should reject this at creation.

### 5.2 `end_time` in the past at creation
Stream is born already ended. UIs should validate `end_time > current_ledger_timestamp`.

### 5.3 Ledger stall during an active stream
Timestamp pauses; accrual pauses. On resume, all elapsed seconds are credited in
one bulk `claim`. No tokens are lost.

### 5.4 Claim at exactly `end_time`
Full stream amount is claimable if a ledger closes exactly at `end_time`. If not,
the last partial second becomes dust (see §3.4).

### 5.5 Rate == 0
Zero accrual regardless of elapsed time. Should be rejected at stream creation.

### 5.6 Very high rate — overflow risk
`rate_per_second as i128 * elapsed as i128` can overflow for large values.
Use `checked_mul` or `saturating_mul` and cap at the stream's total deposited amount.

---

## 6. Summary of Assumptions

1. `Env::ledger().timestamp()` is a trustworthy, monotonic, whole-second counter.
2. Accrual is computed in whole seconds only; sub-second amounts are truncated.
3. The sender accepts that up to ~5 s of stream tail may become unclaimable dust.
4. Off-chain UIs are responsible for smooth display — the contract does not interpolate.
5. No special handling is needed for network stalls; the math self-corrects on resume.

---

## 7. Related Files

| File | Purpose |
|---|---|
| `src/lib.rs` | Contract entry points; reads `env.ledger().timestamp()` |
| `src/types.rs` | `Stream` struct — `start_time`, `end_time`, `rate_per_second` fields |
| `src/contract.rs` | Accrual and claim logic |
| `README.md` | Project overview — links here for timestamp notes |
| `SECURITY.md` | Security considerations — references §2.3 of this doc |

---

## 8. Cargo Test Checklist

\`\`\`bash
cd StreamPay-Contracts
cargo test 2>&1 | tee test_snapshots/timestamp_test_output.txt
\`\`\`

- [ ] `claim` with `now < start_time` → 0 claimable
- [ ] `claim` with `now == start_time` → 0 claimable
- [ ] `claim` with `now` mid-stream → correct whole-second accrual
- [ ] `claim` with `now > end_time` → capped at full stream amount
- [ ] `claim` with `start_time == end_time` → 0 claimable
- [ ] `claim` after ledger stall (simulated timestamp jump) → bulk accrual correct
- [ ] `claim` with `rate == 0` → 0 transferred
- [ ] Overflow guard: large rate × large elapsed does not panic

> Target: >= 95% line coverage on touched accrual paths.

---

*Raise issues against `StreamPay-Org/StreamPay-Contracts`.*
