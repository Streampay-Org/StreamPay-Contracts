# Error & Panic Reference

`streampay-contracts` currently signals failures with `panic!` messages.
This is the canonical list of panic strings that may be observed on-chain
and what each one means for an integrator.

| Panic message | Triggered by | Root cause |
|---|---|---|
| `stream not found` | any read of a stream id with no entry | Wrong id, stream was archived, or entry expired and was GC'd. |
| `stream id overflow` | `create_stream` | The 1-based stream counter rolled over `u32::MAX`. Contract is exhausted. |
| `rate and balance must be positive` | `create_stream`, `create_vesting_stream` | `rate_per_second < MIN_RATE_PER_SECOND` or `initial_balance < MIN_INITIAL_BALANCE`. |
| `vesting duration must be positive` | `create_vesting_stream` | `duration_seconds == 0`. |
| `memo exceeds 32 chars` | `create_stream_with_options` | The memo string is longer than 32 bytes. |
| `stream already active` | `start_stream` | Trying to start a stream whose `is_active` flag is already true. |
| `stream not active` | `stop_stream` | Trying to stop a stream that is already stopped. |
| `unauthorized stopper` | `stop_stream` | `stopper` is neither the payer nor an authorised recipient (`recipient_can_stop = true`). |
| `end_time must be in the future` | `start_stream` | `end_time` was set at creation but is not after the current ledger timestamp. |
| `batch too large` | `batch_settle` | Caller passed more than `MAX_BATCH_SETTLE_SIZE` (25) ids. |
| `cannot archive active stream` | `archive_stream` | Stream is still active; stop it first. |
| `cannot archive stream with unsettled balance` | `archive_stream` | `balance != 0`. |
| `cannot archive stream with unclaimed balance` | `archive_stream` | `claimable_balance != 0`; recipient must `withdraw_stream` first. |
| `rate must be positive` | `update_rate` | `new_rate <= 0`. |
| `rate increase exceeds 10% limit` | `update_rate` | `new_rate` exceeds `old_rate + old_rate / 10`. |
| `cannot cancel inactive stream` | `cancel_stream` | Stream is not active. |
| `cannot pause inactive stream` | `pause_stream` | Stream is not active. |
| `stream already paused` | `pause_stream` | `paused_at > 0`. |
| `cannot resume inactive stream` | `resume_stream` | Stream is not active. |
| `stream is not paused` | `resume_stream` | `paused_at == 0`. |

## Why panic strings instead of error enums?

The contract is at v0.x and still iterating on the public surface; panic
strings keep refactors cheap. A v1.0 plan is tracked in
`docs/audit-readiness.md`, which proposes migrating to `#[contracterror]`
enums for stable client-side handling.

## Off-chain handling

Soroban surfaces panics to clients as `HostError` with the panic string
preserved in the diagnostic events. Front-ends should match on the strings
above and present user-friendly messages rather than echo the raw panic.
