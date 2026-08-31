# StreamPay API v0.2.0 - Quick Reference

## Core Operations

### create_stream
Create a new payment stream with optional time limit.

```rust
pub fn create_stream(
    env: Env,
    payer: Address,
    recipient: Address,
    rate_per_second: i128,
    initial_balance: i128,
    end_time: u64  // 0 = unlimited; > 0 = auto-terminate timestamp
) -> u32
```

**Requires**: Payer authentication
**Returns**: Stream ID (u32)
**Errors**: Panics if rate or balance is non-positive

**Example**:
```rust
// Unlimited stream
let stream_id = client.create_stream(
    &payer, &recipient, &100, &10_000, &0
);

// Time-limited (7 day subscription)
let end_time = current_time + 7 * 24 * 3600;
let stream_id = client.create_stream(
    &payer, &recipient, &100, &10_000, &end_time
);
```

---

### start_stream
Start an inactive stream.

```rust
pub fn start_stream(env: Env, stream_id: u32)
```

**Requires**: Payer authentication
**Returns**: Nothing
**Errors**: 
- `"stream already active"` - Cannot start active stream
- `"end_time must be in the future"` - end_time in past

**Behavior**:
- Sets `start_time = now`
- Sets `is_active = true`
- Clears `paused_at` (if paused)
- Validates `end_time > now` if set

**Example**:
```rust
client.start_stream(&stream_id);  // Begin accrual
```

---

### settle_stream
Calculate and deduct accrued amounts (internal accounting).

```rust
pub fn settle_stream(env: Env, stream_id: u32) -> i128
```

**Requires**: No authorization
**Returns**: Amount settled (i128)
**Errors**: Panics if stream not found

**Behavior**:
- Computes accrued: `(time_elapsed) * rate_per_second`
- Caps at balance (no overpayment)
- Deducts from balance
- Respects `paused_at` (uses pause time if paused)
- Respects `end_time` (caps accrual, auto-deactivates at end)

**Example**:
```rust
let accrued = client.settle_stream(&stream_id);
println!("Amount settled: {}", accrued);  // In base units
```

**Paused Behavior**:
```rust
client.pause_stream(&stream_id);
env.advance_time(1000);  // Pass time
let accrued = client.settle_stream(&stream_id);
assert_eq!(accrued, 0);  // No new accrual while paused
```

---

## New: Cancel Stream (Issue #4)

### cancel_stream
Cancel active stream and settle accrued amounts immediately.

```rust
pub fn cancel_stream(env: Env, stream_id: u32)
```

**Requires**: Payer authentication
**Returns**: Nothing  
**Errors**: `"cannot cancel inactive stream"`

**Behavior**:
- Immediately settles accrued amounts
- Recipient receives accrued amount
- Payer retains remaining balance
- Stream marked inactive
- Atomic operation (prevents races)

**Example**:
```rust
// 10s into a 100/s stream
client.cancel_stream(&stream_id);  // Settles 1000, leaves 9000
let info = client.get_stream_info(&stream_id);
assert!(!info.is_active);
assert_eq!(info.balance, 9000);
```

---

## New: Pause/Resume

### pause_stream
Freeze accrual without terminating the stream.

```rust
pub fn pause_stream(env: Env, stream_id: u32)
```

**Requires**: Payer authentication
**Returns**: Nothing
**Errors**: 
- `"cannot pause inactive stream"`
- `"stream already paused"`

**Behavior**:
- Settles accrued amount to pause point
- Sets `paused_at = now`
- Keeps `is_active = true`
- Accrual stops until resumed

**Example**:
```rust
client.start_stream(&stream_id);
env.advance_time(5);
client.pause_stream(&stream_id);  // Settles 500

// 1 hour passes, stream doesn't accrue
env.advance_time(3600);

client.resume_stream(&stream_id);
env.advance_time(2);
let accrued = client.settle_stream(&stream_id);  // Only 200
```

---

### resume_stream
Restart accrual from a paused stream.

```rust
pub fn resume_stream(env: Env, stream_id: u32)
```

**Requires**: Payer authentication
**Returns**: Nothing
**Errors**: 
- `"cannot resume inactive stream"`
- `"stream is not paused"`

**Behavior**:
- Clears `paused_at`
- Resets `start_time = now` (for future accrual calculation)
- Keeps `is_active = true`
- Accrual resumes

**Example**:
```rust
// After pause
client.resume_stream(&stream_id);
env.advance_time(3);
let accrued = client.settle_stream(&stream_id);  // 300 accrued
```

---

## Existing Operations (Enhanced)

### stop_stream
Permanently stop an active stream.

```rust
pub fn stop_stream(env: Env, stream_id: u32)
```

**Enhanced in v0.2.0**:
- Now clears `paused_at` when stopping

**Example**:
```rust
client.stop_stream(&stream_id);  // Final stop
let info = client.get_stream_info(&stream_id);
assert!(!info.is_active);
```

---

### get_stream_info
Retrieve stream details (read-only).

```rust
pub fn get_stream_info(env: Env, stream_id: u32) -> StreamInfo
```

**Returns**: `StreamInfo` struct

**StreamInfo Fields**:
```rust
pub struct StreamInfo {
    pub payer: Address,              // Payer address
    pub recipient: Address,          // Recipient address
    pub rate_per_second: i128,       // Payment rate (base units/s)
    pub balance: i128,               // Remaining balance
    pub start_time: u64,             // Stream start or resume timestamp
    pub end_time: u64,               // Auto-terminate time (0 = unlimited)
    pub is_active: bool,             // Active vs. stopped
    pub paused_at: u64,              // Pause timestamp (0 = not paused)
}
```

**Example**:
```rust
let info = client.get_stream_info(&stream_id);
if info.paused_at > 0 {
    println!("Stream paused at {}", info.paused_at);
} else if info.is_active {
    println!("Stream active, balance: {}", info.balance);
} else {
    println!("Stream stopped");
}
```

---

### archive_stream
Remove a fully-settled, inactive stream from storage.

```rust
pub fn archive_stream(env: Env, stream_id: u32)
```

**Requires**: Payer authentication  
**Returns**: Nothing
**Errors**: 
- `"cannot archive active stream"`
- `"cannot archive stream with unsettled balance"`

**Behavior**:
- Removes stream from persistent storage
- Must be stopped and fully settled (balance = 0)
- Protects recipient entitlements

**Example**:
```rust
client.stop_stream(&stream_id);
let accrued = client.settle_stream(&stream_id);  // Drain balance
client.archive_stream(&stream_id);  // Remove from storage

// get_stream_info now panics (stream not found)
```

---

### version
Get contract version.

```rust
pub fn version(_env: Env) -> u32
```

**Returns**: Version as u32 (major*1M + minor*1k + patch)
**Current**: 2_000 (v0.2.0)

**Example**:
```rust
let v = client.version();
assert_eq!(v, 2_000);  // v0.2.0
```

---

## State Transitions

### Valid Transitions

```
[Created]
    ↓ start_stream
[Active]
    ├─→ pause_stream → [Active, Paused]
    │                      ↓ resume_stream → [Active]
    ├─→ settle_stream → [Active or Inactive] (depending on balance/end_time)
    ├─→ cancel_stream → [Cancelled/Inactive]
    └─→ stop_stream → [Inactive]

[Inactive]
    ├─→ start_stream → [Active]  (if not created yet)
    └─→ archive_stream → [Archived/Removed]
```

---

## Error Guide & Stabilized Error Codes

StreamPay provides stabilized, versioned error codes with numeric identifiers and category ranges:
- **100–199**: Validation & Input
- **200–299**: Lifecycle & State
- **300–399**: Storage & Archiving
- **400–499**: Authorization & Access Control
- **500–599**: Settlement & Arithmetic
- **900–999**: System & Fallback

| Code | Variant | Category | Error Message | Function | Recoverability | Fix |
|------|---------|----------|---------------|----------|----------------|-----|
| `101` | `RateAndBalanceMustBePositive` | Validation | `"rate and balance must be positive"` | create_stream | Retryable | Use positive values (`> 0`) |
| `102` | `EndTimeMustBeInFuture` | Validation | `"end_time must be in the future"` | start_stream | Retryable | Use future timestamp (`end_time > now`) |
| `103` | `BatchTooLarge` | Validation | `"batch too large"` | batch_settle | Retryable | Limit batch size to ≤ 25 stream IDs |
| `104` | `InvalidAmount` | Validation | `"invalid amount"` | Any | Retryable | Pass valid positive amount |
| `105` | `ZeroAmount` | Validation | `"amount cannot be zero"` | Any | Retryable | Specify non-zero quantity |
| `106` | `InvalidTimeRange` | Validation | `"invalid time range"` | create_stream | Retryable | Ensure schedule is valid |
| `201` | `StreamAlreadyActive` | Lifecycle | `"stream already active"` | start_stream | Terminal | Call start_stream once per stream |
| `202` | `StreamNotActive` | Lifecycle | `"stream not active"` | stop_stream | Terminal | Stream is stopped or unstarted |
| `203` | `CannotCancelInactiveStream` | Lifecycle | `"cannot cancel inactive stream"` | cancel_stream | Terminal | Stream must be active to cancel |
| `204` | `CannotPauseInactiveStream` | Lifecycle | `"cannot pause inactive stream"` | pause_stream | Retryable | Start stream before pausing |
| `205` | `StreamAlreadyPaused` | Lifecycle | `"stream already paused"` | pause_stream | Retryable | Resume stream first |
| `206` | `CannotResumeInactiveStream` | Lifecycle | `"cannot resume inactive stream"` | resume_stream | Retryable | Start stream first |
| `207` | `StreamIsNotPaused` | Lifecycle | `"stream is not paused"` | resume_stream | Retryable | Pause stream before resuming |
| `208` | `StreamAlreadyTerminated` | Lifecycle | `"stream already terminated"` | Any | Terminal | Stream reached end time / stopped |
| `301` | `StreamNotFound` | Storage | `"stream not found"` | Any | Retryable | Verify stream ID in persistent storage |
| `302` | `CannotArchiveActiveStream` | Storage | `"cannot archive active stream"` | archive_stream | Terminal | Stop and settle stream before archiving |
| `303` | `CannotArchiveStreamWithUnsettledBalance` | Storage | `"cannot archive stream with unsettled balance"` | archive_stream | Retryable | Settle balance to 0 before archiving |
| `304` | `StorageKeyNotFound` | Storage | `"storage key not found"` | Storage | RequiresAdmin | Check storage key configuration |
| `305` | `StorageQuotaExceeded` | Storage | `"storage quota exceeded"` | Storage | RequiresAdmin | Manage storage TTL and footprint |
| `401` | `Unauthorized` | Authorization | `"unauthorized caller"` | Any | Terminal | Provide valid cryptographic signature |
| `402` | `NotPayer` | Authorization | `"caller is not payer"` | Any | Terminal | Only stream payer may invoke operation |
| `403` | `NotRecipient` | Authorization | `"caller is not recipient"` | Any | Terminal | Caller must match recipient address |
| `501` | `ArithmeticOverflow` | Settlement | `"arithmetic overflow"` | Settlement | Terminal | Input exceeds numeric ceiling |
| `502` | `InsufficientBalance` | Settlement | `"insufficient balance"` | Settlement | Terminal | Increase deposited balance |
| `503` | `ZeroSettlement` | Settlement | `"zero settlement"` | Settlement | Terminal | Elapsed duration yielded zero value |
| `999` | `UnknownError` | System | `"unknown error"` | System | Terminal | Safe fallback for unrecognized codes |

---

## Common Patterns

### Pattern 1: Simple Unlimited Stream
```rust
let stream_id = client.create_stream(&payer, &recipient, &100, &1000, &0);
client.start_stream(&stream_id);
env.advance_time(5);
let accrued = client.settle_stream(&stream_id);  // 500
client.stop_stream(&stream_id);
client.archive_stream(&stream_id);
```

### Pattern 2: Pause/Resume Workflow
```rust
// Setup
let stream_id = client.create_stream(&payer, &recipient, &50, &500, &0);
client.start_stream(&stream_id);

// Pause during off-hours
client.pause_stream(&stream_id);

// Hours later...
client.resume_stream(&stream_id);

// Cleanup
client.stop_stream(&stream_id);
client.settle_stream(&stream_id);
client.archive_stream(&stream_id);
```

### Pattern 3: Time-Limited Subscription
```rust
let end_time = now + 30 * 24 * 3600;  // 30 days
let stream_id = client.create_stream(&payer, &recipient, &3_33, &3_000, &end_time);
client.start_stream(&stream_id);

// Settles over 30 days
// After 30 days, auto-deactivates
env.advance_time(30 * 24 * 3600);
let accrued = client.settle_stream(&stream_id);  // ~3000
let info = client.get_stream_info(&stream_id);
assert!(!info.is_active);  // Auto-deactivated
```

### Pattern 4: Early Cancellation with Refund
```rust
let stream_id = client.create_stream(&payer, &recipient, &100, &1000, &0);
client.start_stream(&stream_id);
env.advance_time(3);
let balance_before = client.get_stream_info(&stream_id).balance;

client.cancel_stream(&stream_id);  // Atomic: settle + deactivate

let info = client.get_stream_info(&stream_id);
assert_eq!(info.balance, balance_before - 300);  // Recipient got 300
assert!(!info.is_active);
```

---

## Version History

| Version | Features | Breaking Changes |
|---------|----------|------------------|
| 0.1.0   | Basic streaming | None (initial) |
| 0.2.0   | Cancel, Pause/Resume, End Time | `create_stream` signature |

---

**Documentation**: See `docs/` for detailed specifications
**Tests**: Run `cargo test --lib` for 27 comprehensive tests
