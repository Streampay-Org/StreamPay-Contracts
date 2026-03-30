# Pause/Resume Streams

## Overview

Streams can now be temporarily paused and resumed without full termination. This allows payers to stop accrual while preserving the stream relationship and maintaining all schedule information.

## Semantics

### State Machine
```
[Inactive] → start_stream → [Active]
                               ↓ pause_stream
                            [Active, Paused]
                               ↑ resume_stream
                               ↓ stop_stream
                           [Inactive]
```

### Key Differences from stop_stream

| Operation | Effect | Reversible | Balance | Rate Reset |
|-----------|--------|-----------|---------|-----------|
| `pause_stream` | Stops accrual, preserves schedule | Yes | Frozen | No |
| `stop_stream` | Permanently stops stream | No (unless restarted via start_stream) | Frozen | Yes |

### Pause Stream

**Who**: Payer only (authorized via `require_auth()`)

**When**: Stream must be active and not already paused

**What happens**:
1. Settles all accrued amounts up to the pause point
   - Formula: `accrued = (now - start_time) * rate_per_second`
   - Capped at available balance

2. Marks stream as paused:
   - `paused_at = now` (saves pause timestamp)
   - `is_active` remains `true` (logically paused, not stopped)
   - `balance` is updated to reflect accrual

3. Time stops accruing:
   - No time passes while paused
   - Multiple pauses and resumes work independently

**Error Cases**:
- `"cannot pause inactive stream"` if stream is not active
- `"stream already paused"` if attempting to pause twice

### Resume Stream

**Who**: Payer only (authorized via `require_auth()`)

**When**: Stream must be active and paused (`paused_at > 0`)

**What happens**:
1. Clears the paused state:
   - `paused_at = 0`
   - `start_time = now` (reset to resume point for future accrual)
   - `is_active` remains `true`

2. Resumes accrual from the resume point

**Error Cases**:
- `"cannot resume inactive stream"` if stream is not active
- `"stream is not paused"` if attempting to resume a non-paused stream

### Settle While Paused

When `settle_stream` is called on a paused stream:
- Accrual window is `[start_time, paused_at]`
- No additional settlement occurs (timestamp is locked at `paused_at`)
- Returns 0 if no time has passed since last pause

This is distinct from settling an active stream, which uses the current timestamp.

## Examples

### Scenario 1: Pause and Resume
```rust
create_stream:   rate=100/s, balance=1000, at t=0
start_stream:    at t=0
advance:         5 seconds (t=5)
pause_stream:    accrued=500, remaining=500, paused_at=5

advance:         10 seconds (t=15) — no accrual during pause!
settle_stream:   returns 0 (paused_at unchanged)

resume_stream:   at t=15, start_time reset to 15
advance:         3 seconds (t=18)
settle_stream:   accrued=300 (3 * 100), balance becomes 200
```

### Scenario 2: Multiple Pause/Resume Cycles
```
Cycle 1: pause at t=2  → accrued=200,  balance=800
         resume at t=7 → (no accrual during [2,7])
         
Cycle 2: pause at t=12 → accrued=500,  balance=300
         resume at t=20 → (no accrual during [12,20])
         
Cycle 3: stop at t=25  → (stream terminates)
```

### Scenario 3: Settle While Paused
```
start:           t=0
advance:         5 seconds (t=5)
pause:           accrued=500, balance=500, paused_at=5
advance:         100 seconds (t=105) — time passes, stream doesn't accrue
settle:          returns 0 (uses paused_at=5, no new time)
get_stream_info: balance still 500, paused_at still 5
```

## Use Cases

1. **Cash Flow Management**: Pause when funds are low, resume when available
2. **Scheduled Breaks**: Pause during off-hours/weekends, resume for business hours
3. **Maintenance Windows**: Pause payment processing during system updates
4. **Dispute Periods**: Freeze payment stream during disagreement resolution
5. **Temporary Service Interruption**: Pause service without canceling relationship

## Implementation Details

### Data Structure
Pause state is tracked in `StreamInfo`:
```rust
pub struct StreamInfo {
    // ... other fields ...
    pub paused_at: u64,  // 0 if not paused; timestamp if paused
}
```

### Accrual Calculation
When paused:
- `settle_stream` uses `paused_at` instead of current timestamp
- After resume: `start_time = now` to reset the accrual window

### Safety Properties
1. **No lost accrual**: Paused accrual is immediately deducted from balance
2. **Non-negative time**: Accrual only happens when resumed
3. **Recipient protection**: Paused amounts are settled and unavailable to payer
4. **Idempotent resume**: Multiple resumes without intermediate pauses are safe

## API Reference

### pause_stream
```rust
pub fn pause_stream(env: Env, stream_id: u32)
```
Pauses an active stream. Accrual stops; balances preserved.

### resume_stream
```rust
pub fn resume_stream(env: Env, stream_id: u32)
```
Resumes a paused stream. Accrual restarts from the resume point.

## Edge Cases

### Pausing at Exact End Time
If `end_time` is set and pause occurs at or past `end_time`:
- Pause settles up to `end_time`
- Stream auto-deactivates if end_time reached

### Multiple Pauses Without Resume
```
pause() → error: "stream already paused"
```

### Pause Then Stop
```
pause_stream()  ✓
stop_stream()   ✓ (clears paused_at, marks inactive)
```

### Resume Past End Time
If `end_time` is set and stream is resumed past that point:
- Resume succeeds (`start_time` set to now)
- Next `settle_stream` immediately deactivates stream
