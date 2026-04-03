# Stream Lifecycle State Machine

## Overview

This document describes the complete lifecycle of a payment stream in the StreamPay Soroban contract, including all possible states, transitions, and how balance accrual interacts with each state.

## State Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Stream Lifecycle                            │
└─────────────────────────────────────────────────────────────────────┘

    [create_stream]
           │
           ▼
    ┌──────────┐
    │ CREATED  │ ◄──────────────┐
    │          │                │
    │ inactive │                │
    │ balance  │                │
    └──────────┘                │
           │                    │
           │ [start_stream]     │
           ▼                    │
    ┌──────────┐                │
    │  ACTIVE  │                │
    │          │                │
    │ accruing │                │
    │ balance  │                │
    └──────────┘                │
           │                    │
           │ [stop_stream]      │
           ▼                    │
    ┌──────────┐                │
    │ STOPPED  │                │
    │          │                │
    │ inactive │                │
    │ balance  │                │
    └──────────┘                │
           │                    │
           │ [start_stream]     │
           └────────────────────┘
           │
           │ (balance = 0 && inactive)
           │
           │ [archive_stream]
           ▼
    ┌──────────┐
    │ ARCHIVED │
    │          │
    │ removed  │
    └──────────┘
```

## State Definitions

### 1. CREATED

**Description:** Initial state after `create_stream()` is called. The stream exists in storage but is not yet accruing value.

**Characteristics:**
- `is_active = false`
- `start_time = 0`
- `end_time = 0`
- `balance > 0` (set by `initial_balance` parameter)
- No accrual occurs

**Entry:** Called via `create_stream(payer, recipient, rate_per_second, initial_balance)`

**Requirements:**
- Payer must authorize the transaction
- `rate_per_second > 0`
- `initial_balance > 0`

**Storage:**
- Stream stored in persistent storage with key `("stream", stream_id)`
- TTL extended to ~30 days (`STREAM_TTL_EXTEND`)

### 2. ACTIVE

**Description:** Stream is running and accruing value from payer to recipient based on elapsed time.

**Characteristics:**
- `is_active = true`
- `start_time = ledger.timestamp()` (set when activated)
- `end_time` may be 0 or from previous stop
- `balance` decreases as `settle_stream()` is called
- **Accrual is active**

**Entry:** Called via `start_stream(stream_id)` from CREATED or STOPPED state

**Requirements:**
- Payer must authorize
- Stream must not already be active (panics if `is_active = true`)

**Accrual Mechanism:**

When `settle_stream()` is called on an ACTIVE stream:
```rust
elapsed = current_timestamp - start_time
amount = min(elapsed * rate_per_second, balance)
balance = balance - amount
start_time = current_timestamp  // Reset for next settlement
```

**Key Points:**
- Accrual is calculated on-demand via `settle_stream()`, not automatically
- Settlement is idempotent: calling multiple times in quick succession yields minimal additional accrual
- Balance cannot go negative (capped at available balance)
- After settlement, `start_time` is updated to prevent double-counting

### 3. STOPPED

**Description:** Stream has been paused. No accrual occurs, but the stream can be restarted.

**Characteristics:**
- `is_active = false`
- `start_time` reflects last activation time
- `end_time = ledger.timestamp()` (set when stopped)
- `balance` remains at last settled amount
- **No accrual occurs**

**Entry:** Called via `stop_stream(stream_id)` from ACTIVE state

**Requirements:**
- Payer must authorize
- Stream must be active (panics if `is_active = false`)

**Transitions:**
- Can return to ACTIVE via `start_stream()` (resets `start_time`)
- Can proceed to ARCHIVED if `balance = 0`

### 4. ARCHIVED

**Description:** Stream has been permanently removed from storage. This is a terminal state.

**Characteristics:**
- Stream data deleted from persistent storage
- Cannot be retrieved or reactivated
- Frees storage space and reduces contract state

**Entry:** Called via `archive_stream(stream_id)` from STOPPED or CREATED state

**Requirements:**
- Payer must authorize
- `is_active = false` (panics if active)
- `balance = 0` (panics if unsettled balance remains)

**Purpose:**
- Protects recipient entitlements (cannot archive with outstanding balance)
- Allows payer to clean up fully-settled streams
- Reduces on-chain storage costs

## State Transitions

| From State | Action | To State | Conditions |
|------------|--------|----------|------------|
| (none) | `create_stream()` | CREATED | Payer auth, rate > 0, balance > 0 |
| CREATED | `start_stream()` | ACTIVE | Payer auth, not already active |
| ACTIVE | `stop_stream()` | STOPPED | Payer auth, currently active |
| STOPPED | `start_stream()` | ACTIVE | Payer auth, not already active |
| CREATED | `archive_stream()` | ARCHIVED | Payer auth, inactive, balance = 0 |
| STOPPED | `archive_stream()` | ARCHIVED | Payer auth, inactive, balance = 0 |

## Accrual Interaction by State

### CREATED State
- **Accrual:** None
- **Balance:** Static at `initial_balance`
- **Rationale:** Stream has not been activated yet; no time has elapsed for payment

### ACTIVE State
- **Accrual:** Active (calculated on `settle_stream()` calls)
- **Balance:** Decreases with each settlement
- **Formula:** `accrued = (current_time - start_time) * rate_per_second`
- **Cap:** Accrual cannot exceed remaining balance
- **Rationale:** This is the only state where value transfers from payer to recipient

### STOPPED State
- **Accrual:** None
- **Balance:** Static at last settled amount
- **Rationale:** Stream is paused; no time-based accrual should occur
- **Note:** If restarted, accrual begins fresh from the new `start_time`

### ARCHIVED State
- **Accrual:** N/A (stream no longer exists)
- **Balance:** N/A (must be 0 before archival)
- **Rationale:** Terminal state; all obligations settled

## Settlement Mechanics

The `settle_stream()` function is the core mechanism for accrual:

```rust
pub fn settle_stream(env: Env, stream_id: u32) -> i128 {
    let mut info = get_stream(&env, stream_id);
    
    // Only active streams accrue
    if !info.is_active {
        return 0;
    }
    
    let now = env.ledger().timestamp();
    let elapsed = now - info.start_time;
    
    // Calculate accrued amount, capped at remaining balance
    let amount = (elapsed as i128)
        .saturating_mul(info.rate_per_second)
        .min(info.balance);
    
    // Deduct from balance and reset start_time
    info.balance = info.balance.saturating_sub(amount);
    info.start_time = now;
    
    set_stream(&env, stream_id, &info);
    extend_stream_ttl(&env, stream_id);
    extend_instance_ttl(&env);
    
    amount
}
```

**Key Properties:**
- Returns `0` for inactive streams (CREATED, STOPPED, ARCHIVED)
- Uses saturating arithmetic to prevent overflow/underflow
- Resets `start_time` to prevent double-counting on subsequent calls
- Extends TTL to keep active streams alive in storage

## Edge Cases and Security Considerations

### 1. Rapid Start/Stop Cycles
**Scenario:** Payer repeatedly starts and stops stream to manipulate accrual

**Protection:** Each `start_stream()` resets `start_time` to current timestamp. Accrual only counts time while ACTIVE.

### 2. Delayed Settlement
**Scenario:** Recipient delays calling `settle_stream()` to accrue more value

**Protection:** Accrual is capped at remaining `balance`. Once balance reaches 0, no further accrual occurs regardless of elapsed time.

### 3. Premature Archival
**Scenario:** Payer attempts to archive stream with outstanding balance

**Protection:** `archive_stream()` panics if `balance != 0`, protecting recipient entitlements.

### 4. Double Settlement
**Scenario:** Multiple `settle_stream()` calls in quick succession

**Protection:** `start_time` is updated after each settlement, so subsequent calls only accrue for the time since last settlement.

### 5. TTL Expiration
**Scenario:** Stream expires from storage before settlement

**Protection:** All state-modifying operations extend TTL. Active streams should be settled regularly to maintain storage presence.

## Example Lifecycle Flow

```
1. Payer creates stream: 100 tokens/second, 10,000 token balance
   State: CREATED
   Balance: 10,000
   Accrual: None

2. Payer starts stream at t=0
   State: ACTIVE
   Balance: 10,000
   start_time: 0
   Accrual: Active

3. Recipient settles at t=50 seconds
   Accrued: 50 * 100 = 5,000 tokens
   Balance: 10,000 - 5,000 = 5,000
   start_time: 50 (reset)
   State: ACTIVE

4. Payer stops stream at t=75 seconds
   State: STOPPED
   Balance: 5,000 (no automatic settlement on stop)
   end_time: 75
   Accrual: None

5. Recipient settles (stream is stopped)
   Accrued: 0 (inactive)
   Balance: 5,000 (unchanged)

6. Payer restarts stream at t=100
   State: ACTIVE
   start_time: 100 (reset)
   Balance: 5,000
   Accrual: Active

7. Recipient settles at t=150
   Accrued: (150 - 100) * 100 = 5,000 tokens
   Balance: 5,000 - 5,000 = 0
   State: ACTIVE (but balance depleted)

8. Payer stops stream
   State: STOPPED
   Balance: 0

9. Payer archives stream
   State: ARCHIVED (removed from storage)
```

## Testing Coverage

The contract includes comprehensive tests for state transitions:

- `test_create_stream_valid` — CREATED state initialization
- `test_start_and_stop_stream` — CREATED → ACTIVE → STOPPED transitions
- `test_settle_returns_amount` — Accrual calculation in ACTIVE state
- `test_archive_settled_stream` — STOPPED → ARCHIVED with balance = 0
- `test_archive_unsettled_stream_panics` — Protection against premature archival
- `test_archive_active_stream_panics` — Cannot archive ACTIVE streams
- `test_archived_stream_not_found` — ARCHIVED state is terminal

Run tests with:
```bash
cargo test
```

## Future Considerations

### Automatic Settlement on Stop
Currently, `stop_stream()` does not automatically settle. Consider adding:
```rust
let accrued = Self::settle_stream(env.clone(), stream_id);
// Then stop the stream
```

**Trade-off:** Increases gas cost of stop operation but ensures recipient gets all accrued value.

### Cancellation vs. Stop
Consider adding a `cancel_stream()` function that:
- Settles accrued amount
- Returns remaining balance to payer
- Archives stream in one transaction

**Use case:** Mutual agreement to terminate stream early.

### Partial Withdrawals
Allow recipient to withdraw accrued amount without full settlement:
```rust
pub fn withdraw(env: Env, stream_id: u32, amount: i128) -> i128
```

**Benefit:** More flexible fund access for recipients.

## References

- Main contract implementation: `src/lib.rs`
- Factory pattern design: `docs/factory-pattern.md`
- Release process: `docs/RELEASE.md`
- Soroban storage documentation: https://soroban.stellar.org/docs/learn/persisting-data

## Changelog

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2026-03-30 | Initial stream lifecycle documentation |
