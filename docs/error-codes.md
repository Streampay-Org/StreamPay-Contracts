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
| `unauthorized stopper` | `stop_stream` | The caller is neither the payer nor a recipient with `recipient_can_stop = true`. |
| `batch too large` | `batch_settle` | Caller passed more than `MAX_BATCH_SETTLE_SIZE` (25) ids. |
| `stream still has balance` | `archive_stream` | Balance is not fully settled and withdrawn. |
| `stream still active` | `archive_stream` | Stream is still active; stop it first. |

## Why panic strings instead of error enums?

The contract is at v0.x and still iterating on the public surface; panic
strings keep refactors cheap. A v1.0 plan is tracked in
`docs/audit-readiness.md`, which proposes migrating to `#[contracterror]`
enums for stable client-side handling.

## Off-chain handling

Soroban surfaces panics to clients as `HostError` with the panic string
preserved in the diagnostic events. Front-ends should match on the strings
above and present user-friendly messages rather than echo the raw panic.
