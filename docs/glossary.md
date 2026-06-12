# Glossary

Common terms used across the `streampay-contracts` codebase and docs.

## Accrual

The continuous process of crediting tokens to the recipient over time at
`rate_per_second`. Implemented by `settle_stream`, which converts the
elapsed-since-last-settlement window into a discrete movement from
`balance` to `claimable_balance`.

## Active stream

A stream whose `is_active` flag is `true`. Only active streams accrue.
A stream becomes active when the payer calls `start_stream` and inactive
when anyone authorised calls `stop_stream`.

## Archive

Permanent removal of a fully-settled, fully-withdrawn stream's persistent
storage entry. Performed by `archive_stream` and irreversible.

## Claimable balance

Tokens that have already been accrued by the recipient but not yet
transferred on-ledger. Held inside the contract until `withdraw_stream` is
called.

## Dust tail

The fractional-second window between the last ledger close before
`end_time` and `end_time` itself. Because accrual is rounded to whole
seconds, this slice is never claimable by the recipient — it goes back to
the payer on close.

## Ledger close time

The validator-agreed Unix-seconds timestamp produced by SCP for each
closed ledger. Surfaced via `Env::ledger().timestamp()` and used as the
sole on-chain time source.

## Memo

Optional 32-byte string attached to a stream at creation for off-chain
correlation (e.g., an external invoice id). Immutable after creation.

## Settlement

Permissionless operation that moves accrued tokens from `balance` to
`claimable_balance`. Does not transfer tokens on-ledger.

## TTL

"Time to live" of a Soroban persistent storage entry, measured in ledgers.
The contract bumps a stream's TTL each time it is touched so frequently
used streams stay live.

## Withdrawal

Recipient-authorised on-ledger transfer of `claimable_balance` to the
recipient's account. Implicitly settles first.
