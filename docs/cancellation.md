# Stream Cancellation (Issue #4)

## Overview

Streams can now be cancelled early by the payer using the `cancel_stream` function. This provides a mechanism for payers to exit streaming agreements while preserving fairness for recipients.

## Semantics

### Who Can Cancel
- **Payer only**: Only the stream payer (not the recipient) can initiate cancellation
- Authorization is enforced via `info.payer.require_auth()`

### When Can Cancel
- Stream must be **active** (`is_active == true`)
- Cannot cancel an inactive, stopped, or already-cancelled stream
- Panics with `"cannot cancel inactive stream"` if attempted on inactive stream

### What Happens on Cancellation

1. **Accrual Settlement**: All amounts accrued from `start_time` to the current moment are calculated
   - Formula: `accrued = min((now - start_time) * rate_per_second, balance)`
   - Protects against overflow via `saturating_mul` and capping against available balance

2. **Balance Deduction**: Accrued amount is deducted from the stream's balance
   - `balance -= accrued` (recipient gets the accrued amount)
   - Remaining balance stays in the stream (effectively refunded to payer when archived)

3. **Stream Deactivation**: Stream is immediately marked inactive
   - `is_active = false`
   - `end_time = now` (marks cancellation point)
   - `paused_at = 0` (clears any paused state)

### Fund Conservation

The cancellation operation preserves fund conservation:
- Total at creation: `balance_initial`
- After cancellation: `accrued (to recipient) + remaining_balance (refundable to payer)`
- Invariant: `accrued + remaining_balance == balance_initial` (before any settlement)

## Race Condition Safety

### The Issue
Between the time a `cancel` call is sent and executed, another `settle` call could race against it, causing double-settlement of the same time period.

### The Solution
The `cancel_stream` function is atomic:
1. Reads current stream state
2. Computes accrual to `now` inthe same transaction
3. Updates balance and `is_active` in a single storage write
4. No separate settle call is needed

This atomic pattern prevents the race because both `cancel` and `settle` use the blockchain's transactional ordering.

## Examples

### Scenario: Cancel at 50% accrual
```
Created: rate=100/s, balance=1000
Started: at timestamp 0
Advance: 5 seconds pass
Now: timestamp = 5
Accrued: 5 * 100 = 500
Remaining: 1000 - 500 = 500

After cancel_stream():
- Recipient has accrued: 500
- Payer retains: 500 (refunded when archived)
- Stream is inactive and ready for archival
```

### Scenario: Cancel near depletion
```
Created: rate=100/s, balance=1000
Started: at timestamp 0
Advance: 15 seconds pass (would drain 1500, but balance is only 1000)
Now: timestamp = 15
Accrued: min(15 * 100, 1000) = 1000
Remaining: 0

After cancel_stream():
- Recipient has accrued: 1000 (full balance)
- Payer retains: 0
- Stream is inactive
```

## Security Notes

1. **No overpayment risk**: Accrual is capped at available balance
2. **No underpayment race**: Atomic operation prevents missed settlements
3. **Payer signature required**: Only authorized payer can cancel
4. **Recipient compensation guaranteed**: Accrued amounts are immediately deducted and safe

## API Reference

```rust
pub fn cancel_stream(env: Env, stream_id: u32)
```

### Parameters
- `env`: Soroban environment
- `stream_id`: ID of the stream to cancel

### Behavior
- Requires payer authentication via `require_auth()`
- Settles all accrued amounts (moved to recipient)
- Deactivates the stream immediately
- Panics if stream is not active

### Returns
Nothing; succeeds silently on valid cancellation, panics on error.
