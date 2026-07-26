# Rate Update Policy

This document describes the mid-stream rate update functionality and the policies that protect both payer and recipient interests.

## Overview

The `update_rate` function allows the payer to modify the payment rate of an existing stream, even while it's actively streaming. This provides flexibility for payers to adjust payment terms while maintaining recipient protections.

## Function Signature

```rust
pub fn update_rate(env: Env, stream_id: u32, new_rate: i128)
```

## Authorization

- **Payer-only**: Only the stream payer can update the rate
- **Requires authentication**: `payer.require_auth()` enforced
- **Recipient cannot modify**: Recipients have no control over rate changes

## Rate Change Policy

### Allowed Changes

1. **Unlimited decreases**: Payer can reduce rate to any positive value
   - Example: 1000/s → 100/s (90% decrease) ✓
   - Example: 100/s → 1/s (99% decrease) ✓

2. **Bounded increases**: Payer can increase rate up to 10% above current rate
   - Example: 100/s → 110/s (10% increase) ✓
   - Example: 100/s → 109/s (9% increase) ✓

### Rejected Changes

1. **Large increases**: Rate increases exceeding 10% are rejected
   - Example: 100/s → 120/s (20% increase) ✗ Panics
   - Example: 100/s → 111/s (11% increase) ✗ Panics

2. **Zero or negative rates**: Invalid rates are rejected
   - Example: 100/s → 0/s ✗ Panics
   - Example: 100/s → -50/s ✗ Panics

## Rationale

### Why Allow Decreases?

Payers may need to reduce payment rates due to:
- Budget constraints
- Changing business requirements
- Renegotiated terms with recipient
- Gradual wind-down of services

Unlimited decreases are safe because:
- Payer is already committed to the stream balance
- Recipient has already accrued rights to streamed amounts
- Lower rates extend the stream duration, giving recipient more time

### Why Limit Increases?

The 10% increase limit protects recipients from:
- **Unexpected balance depletion**: Large rate increases could drain the balance faster than recipient expects
- **Forced early settlement**: Recipient may plan withdrawals based on expected stream duration
- **Gaming the system**: Prevents payer from manipulating rates to create unfavorable conditions

The 10% threshold provides:
- **Flexibility**: Allows minor adjustments for inflation, bonuses, or corrections
- **Predictability**: Recipients can reasonably anticipate stream behavior
- **Safety**: Prevents dramatic changes that could harm recipient planning

## Settlement Behavior

### Active Streams

When updating the rate of an **active** stream:

1. **Automatic settlement**: Contract settles all accrued amount at the old rate
2. **Balance deduction**: Settled amount is deducted from stream balance
3. **Timestamp reset**: `start_time` is reset to current ledger time
4. **New rate applies**: All future accruals use the new rate

**Example:**
```
Initial: rate=100/s, balance=10,000, start_time=T0
At T0+10s: update_rate(50/s)
  → Settles: 10s × 100/s = 1,000
  → New balance: 9,000
  → New start_time: T0+10s
  → Future accruals: 50/s
```

### Inactive Streams

When updating the rate of an **inactive** stream:

1. **No settlement**: No accrual to settle
2. **Balance unchanged**: Stream balance remains the same
3. **Rate updated**: New rate will apply when stream is started

## Edge Cases and Security

### Multiple Rate Updates

Payers can update rates multiple times:
- Each update is evaluated against the **current** rate, not the original
- Example: 100/s → 110/s → 121/s (all valid — 10% compounding per call)
- The 10% limit applies to each individual change, so N successive increases
  compound multiplicatively (`1.10^N` relative to the starting rate of the chain)

### Rate Update During Settlement Window

If a rate update occurs between settlements:
- Old rate applies to the elapsed time before the update
- New rate applies to time after the update
- No funds are lost or double-counted

### Balance Exhaustion

If the stream balance is insufficient for the accrued amount:
- Settlement is capped at available balance (saturating arithmetic)
- Rate update still proceeds
- Stream can continue with new rate if balance remains

### Authorization Bypass Attempts

- **Recipient cannot update**: Only payer has authorization
- **Third parties blocked**: No other addresses can modify rates
- **Auth checked first**: Authorization verified before any state changes

## Usage Examples

### Decrease Rate (Unlimited)

```rust
// Payer wants to reduce payment rate by 50%
client.update_rate(&stream_id, &50_i128); // Old rate: 100/s
// ✓ Success: New rate is 50/s
```

### Small Increase (Within 10%)

```rust
// Payer wants to give a 5% raise
client.update_rate(&stream_id, &105_i128); // Old rate: 100/s
// ✓ Success: New rate is 105/s
```

### Large Increase (Rejected)

```rust
// Payer tries to double the rate
client.update_rate(&stream_id, &200_i128); // Old rate: 100/s
// ✗ Panics: "rate increase exceeds 10% limit"
```

### Active Stream Update

```rust
// Stream has been running for 10 seconds at 100/s
client.update_rate(&stream_id, &80_i128);
// → Automatically settles 1,000 (10s × 100/s)
// → Balance reduced by 1,000
// → New rate: 80/s applies going forward
```

## Integration Considerations

### Frontend Applications

When building UIs for rate updates:

1. **Show current rate**: Display the existing rate_per_second
2. **Calculate max increase**: `max_new_rate = current_rate * 1.1`
3. **Validate client-side**: Prevent invalid submissions
4. **Show settlement preview**: For active streams, show amount that will be settled
5. **Confirm with user**: Rate changes affect payment obligations

### Backend Services

When automating rate updates:

1. **Check stream state**: Query `get_stream_info` first
2. **Calculate impact**: Estimate settlement amount for active streams
3. **Respect policy**: Ensure new rate is within allowed bounds
4. **Handle errors**: Catch panics for invalid rate changes
5. **Log changes**: Track rate modifications for audit trails

## Testing Coverage

The implementation includes comprehensive tests:

- ✅ Update inactive stream (no settlement)
- ✅ Update active stream (automatic settlement)
- ✅ Accrual correctness across rate change
- ✅ Unlimited decreases allowed
- ✅ Small increases (≤10%) allowed
- ✅ Large increases (>10%) rejected
- ✅ Zero rate rejected
- ✅ Negative rate rejected
- ✅ Multiple sequential updates

All tests verify:
- Correct balance calculations
- Proper settlement timing
- Authorization enforcement
- Policy compliance

## Future Enhancements

Potential improvements for future versions:

1. **Configurable increase limit**: Allow contract deployer to set the 10% threshold
2. **Rate change events**: Emit events for off-chain tracking and notifications
3. **Rate history**: Store historical rates for audit and analytics
4. **Recipient approval**: Optional recipient consent for rate increases
5. **Time-locked changes**: Delay rate changes to give recipient notice

## Security Notes

- **No reentrancy risk**: All state changes are atomic within the function
- **Integer overflow protection**: Uses saturating arithmetic for all calculations
- **Authorization enforced**: Payer auth checked before any modifications
- **Recipient protection**: 10% increase limit prevents exploitation
- **Balance integrity**: Settlement ensures no funds are lost or double-counted
- **Storage consistency**: TTL extended on every update to prevent expiration

## Related Documentation

- [Soroban Resource Limits](docs/resource-limits.md) - Resource constraints and optimization
- [Factory Pattern](docs/factory-pattern.md) - Future architecture evolution
- [Release Process](docs/RELEASE.md) - Deployment and versioning
