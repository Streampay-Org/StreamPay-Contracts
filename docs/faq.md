# Frequently Asked Questions

## Why are entry-point panics still strings?

The contract is at v0.x. Panic strings keep the implementation easy to
refactor without invalidating client error-handling code. A v1.0 milestone
plans to migrate to a `#[contracterror]` enum so callers can match
exhaustively.

## Does StreamPay support multiple token types?

Yes. The token contract address is captured at stream creation time; each
stream is denominated in exactly one token. There is no cross-token
conversion.

## Why does `batch_settle` cap at 25 streams?

The cap is a defensive value that keeps the worst-case Soroban instruction
and storage budget well under the network's per-transaction limit. The
exact ceiling depends on token implementation and ledger conditions; 25
leaves comfortable headroom. Off-chain processors can chunk large payroll
runs across multiple transactions.

## Can a stream be edited after creation?

Only `rate_per_second` may change (see `docs/rate-update-policy.md`).
`payer`, `recipient`, `memo`, and `end_time` are immutable. To change any
other field, archive the stream after settlement and create a new one.

## What happens if the recipient never withdraws?

`claimable_balance` accumulates on the stream until it is withdrawn or
the stream is archived. Archive requires the recipient to have withdrawn
their full claimable balance, so abandoned streams can never lock funds
from the payer's side.

## Why are timestamps in seconds rather than ledger numbers?

Seconds are validator-set and monotonically increasing, which is what
accrual needs. Ledger numbers advance at roughly 5-second cadence and are
not a stable time source for streams that may outlive several protocol
upgrades.

## Where do I report bugs and security issues?

- Bugs: open a GitHub issue with a reproduction.
- Security: see `SECURITY.md` for private disclosure.
