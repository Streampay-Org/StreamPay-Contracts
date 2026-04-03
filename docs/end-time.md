# Maximum Duration / End Time (Issue #7)

## Overview

Streams can now have an optional `end_time` parameter that automatically terminates the stream at a specified timestamp. This prevents streams from running indefinitely by accident and provides predictability for both parties.

## Semantics

### End Time Concept
- `end_time` is an optional timestamp when the stream must end
- `end_time = 0` means no time limit (stream runs until manually stopped or balance depleted)
- `end_time > 0` means stream auto-deactivates when settlement reaches that point

### At Creation (create_stream)
```rust
pub fn create_stream(
    env: Env,
    payer: Address,
    recipient: Address,
    rate_per_second: i128,
    initial_balance: i128,
    end_time: u64,  // 0 = no limit; otherwise must be > start_time (validated at start)
) -> u32
```

**Parameter**: 
- `end_time` is specified at stream creation
- Value `0` = unlimited duration
- Value `> 0` = stream auto-terminates at this timestamp

**Validation**:
- `end_time > 0` is allowed at creation
- `end_time <= start_time` is detected and panics at `start_stream()` time, not creation
- This allows flexibility: set end_time at creation, validate when starting

### At Start (start_stream)
If `end_time > 0`, the function validates:
```rust
if info.end_time > 0 && info.end_time <= now {
    panic!("end_time must be in the future");
}
```

**Validation Error**: `"end_time must be in the future"`
- Prevents starting streams that have already expired
- Checked at start time (not creation), allowing streams to be created with future end_time

### During Settlement (settle_stream)

The settlement logic respects end_time:

```rust
// Determine settlement time
let settlement_time = if info.paused_at > 0 {
    // Paused: use pause point
    info.paused_at
} else if info.end_time > 0 && now > info.end_time {
    // Past end_time: cap at end_time
    info.end_time
} else {
    // Normal: use current time
    now
};

// Calculate elapsed time (capped)
let elapsed = settlement_time - info.start_time;

// Auto-deactivate if end reached
if info.end_time > 0 && settlement_time >= info.end_time {
    info.is_active = false;
    info.end_time = settlement_time;
}
```

**Behavior**:
1. Accrual time is capped at `end_time`
2. No settlement occurs past `end_time`
3. Stream auto-deactivates when `settlement_time >= end_time`
4. Subsequent settlements return 0 (stream is inactive)

### Fund Conservation with End Time

Total payout = `min((end_time - start_time) * rate_per_second, initial_balance)`

The invariant holds:
- Accrued (to recipient) + Remaining (to payer) = Initial Balance
- Final balance ≤ Initial Balance (never exceeds)
- Stream can't drain more than initial balance, even if rate and duration allow

## Examples

### Example 1: Unlimited Duration (end_time = 0)
```
create_stream: rate=100/s, balance=5000, end_time=0
start_stream:  at t=0
settle at t=10: 1000 accrued, balance=4000
settle at t=40: 3000 more accrued (40*100=4000, capped at 4000), balance=0
settle at t=50: 0 accrued (balance already 0)
```

### Example 2: Limited Duration with end_time
```
create_stream: rate=100/s, balance=2000, end_time=15
start_stream:  at t=0
settle at t=5:  500 accrued (5 * 100), balance=1500
settle at t=10: 500 accrued (5 * 100), balance=1000
settle at t=20: 500 accrued (10 * 100, capped to 5), balance=500
                auto-deactivates (settlement_time >= end_time)
settle at t=30: 0 accrued (stream is inactive)

Total payout: 1500 (15 * 100)
Remaining: 500
```

### Example 3: End Time Reached Mid-Settlement
```
create_stream: rate=100/s, balance=3000, end_time=12
start_stream:  at t=0
settle at t=8: 800 accrued, balance=2200
settle at t=20: elapsed capped at (12-0)=12
                accrued: min(1200, 2200) = 1200 (but already settled 800)
                additional: 400
                balance=1800
                auto-deactivates
settle at t=30: 0 accrued (inactive)
```

### Example 4: Create with Future end_time
```
create_stream: rate=100/s, balance=1000, end_time=1000 (some future timestamp)
start_stream at t=50: succeeds (50 < 1000)
settle at t=60: accrued=1000, balance=0 (fully drained before end_time)
settle at t=1000: 0 accrued (already inactive due to balance)
```

## Use Cases

### 1. Time-Limited Subscriptions
```
Service subscription: $100/month = $3.33/day ≈ $0.000038/second
duration: 30 days
end_time = start_time + 30 * 24 * 3600
```

### 2. Escrow/Bond Releases
```
Locked funds released linearly over 6 months:
rate = total / (6 * 30 * 24 * 3600)
end_time = start + 6 months
```

### 3. Wage Payment Schedules
```
Employee payment: bi-weekly
end_time = start_time + 14 days
Repeatable: create new stream each period
```

### 4. Rental Agreements
```
Monthly rent stream with 1-month duration
auto-renew pattern: create new stream each month
```

### 5. Vesting Schedules
```
Token vesting: 4-year cliff with linear vesting
end_time = start_time + 4 years
```

## Security Analysis

### Prevention of Indefinite Streams
- **Accidental forever streams** prevented by using `end_time`
- **Runaway payments** capped by both rate and duration
- **Balance limits**: total payout ≤ balance (saturation prevents overflow)

### Race Conditions
End time doesn't introduce race conditions:
- Both `settle` and `cancel` respect end_time
- Auto-deactivation is idempotent
- Multiple settlements past end_time all return 0

### Fund Safety
- Recipient can't exceed: `(end_time - start_time) * rate_per_second`
- Payer retains: `initial_balance - accrued`
- No overflow: saturating arithmetic throughout

### Validation
- `end_time <= now` at start triggers panic (prevents expired streams)
- Stream auto-deactivates (no manual cleanup required)
- Final settlement amount is bounded

## Implementation Details

### Invalid Combinations

**Attempting to start a stream with end_time in the past**:
```rust
create_stream(payer, recipient, 100, 1000, 100)  // end_time = 100
advance to t = 200
start_stream(&stream_id)  // Panics: end_time (100) <= now (200)
```

**Setting end_time to 0 (unlimited)**:
```rust
create_stream(payer, recipient, 100, 10000, 0)
// Stream runs until manually stopped or balance depleted
```

**Past end_time behavior**:
- `settle_stream` automatically deactivates stream when reached
- No special "grace period"; deactivation is immediate
- Subsequent settlements return 0

### Interaction with Pause/Resume
1. Pause before end_time: works normally
2. Resume before end_time: accrual resumes
3. Resume after end_time: stream auto-deactivates on next settle
4. Pause at end_time: settles up to end_time, deactivates

## API Reference

### create_stream
```rust
pub fn create_stream(
    env: Env,
    payer: Address,
    recipient: Address,
    rate_per_second: i128,
    initial_balance: i128,
    end_time: u64,  // 0 = unlimited; > 0 = auto-terminate at this timestamp
) -> u32
```

### start_stream
```rust
pub fn start_stream(env: Env, stream_id: u32)
```
Validates `end_time > now` if set; panics otherwise.

### settle_stream
```rust
pub fn settle_stream(env: Env, stream_id: u32) -> i128
```
Respects end_time; auto-deactivates when reached.

## Testing Coverage

1. **Boundary Tests**:
   - Settle at exact end_time
   - Settle one second before
   - Settle one second after

2. **Integration Tests**:
   - Unlimited (end_time=0)
   - Limited (end_time > 0)
   - Reached before balance depleted
   - Balance depleted before end_time

3. **Error Cases**:
   - Start with past end_time → panic
   - Resume past end_time → auto-deactivation on settle

## Changelog

**Version 0.2.0**:
- Added `end_time` parameter to `create_stream`
- Added `paused_at` field to `StreamInfo`
- Enhanced `settle_stream` to respect end_time
- Enhanced `start_stream` to validate end_time
- Added `cancel_stream`, `pause_stream`, `resume_stream`
