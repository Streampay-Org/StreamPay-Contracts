# Error & Panic Reference

`streampay-contracts` currently signals failures with `panic!` messages.
This is the canonical list of panic strings that may be observed on-chain
and what each one means for an integrator.

| Panic message | Triggered by | Root cause |
|---|---|---|
| `stream not found` | any read of a stream id with no entry | Wrong id, stream was archived, or entry expired and was GC'd. |
| `stream id overflow` | `create_stream` | The 1-based stream counter rolled over `u32::MAX`. Contract is exhausted. |
| `rate and balance must be positive` | `create_stream` | `rate_per_second <= 0` or `initial_balance <= 0`. |
| `memo exceeds 32 chars` | `create_stream` | The optional memo string is longer than 32 bytes. |
| `stream already active` | `start_stream` | Trying to start a stream whose `is_active` flag is already true. |
| `stream not active` | `stop_stream` | Trying to stop a stream that is already stopped. |
| `cannot cancel inactive stream` | `cancel_stream` | The payer attempted to cancel a stream that is not active. |
| `cannot pause inactive stream` | `pause_stream` | The payer attempted to pause a stream that is not active. |
| `stream already paused` | `pause_stream` | The stream is already paused. |
| `cannot resume inactive stream` | `resume_stream` | The payer attempted to resume a stream that is not active. |
| `stream is not paused` | `resume_stream` | The stream is active and not currently paused. |
| `batch too large` | `batch_settle` | Caller passed more than `MAX_BATCH_SETTLE_SIZE` (25) ids. |
| `cannot archive active stream` | `archive_stream` | Stream is still active; stop it first. |
| `cannot archive stream with unsettled balance` | `archive_stream` | Balance is not fully settled. |
| `cannot archive stream with unclaimed balance` | `archive_stream` | Recipient still has claimable balance to withdraw. |
| `rate must be positive` | `create_vesting_stream` | Vesting duration or rate configuration would create a non-positive stream rate. |
| `rate increase exceeds 10% limit` | rate update flow | Requested rate increase exceeds the contract safety bound. |

## Why panic strings instead of error enums?

The contract is at v0.x and still iterating on the public surface; panic
strings keep refactors cheap. A v1.0 plan is tracked in
`docs/audit-readiness.md`, which proposes migrating to `#[contracterror]`
enums for stable client-side handling.

## Off-chain handling

Soroban surfaces panics to clients as `HostError` with the panic string
preserved in the diagnostic events. Front-ends should match on the strings
above and present user-friendly messages rather than echo the raw panic.
