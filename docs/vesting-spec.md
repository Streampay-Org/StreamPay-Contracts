# Vesting Stream Specification

`create_vesting_stream` creates a stream whose value unlocks linearly across a
fixed duration. It is intended for grant, payroll, and vesting-style payouts
where the recipient should not be able to withdraw the full balance at creation.

## Model

- The payer funds the stream up front.
- The contract records a vesting anchor and duration.
- As ledger time advances, the unlocked amount grows linearly.
- `settle_stream` moves unlocked value from `balance` into
  `claimable_balance`.
- `withdraw_stream` transfers the claimable amount to the recipient.

## Rounding

Linear vesting can leave a small remainder when the total amount does not divide
evenly by the duration. The contract tests verify that the final settlement
releases the rounding remainder when the duration has fully elapsed.

## Pause and resume

Pause/resume preserves already-vested value by settling before the stream is
paused. Resuming restarts the active accrual window while keeping the original
vesting schedule anchor intact.

## Indexer guidance

- Treat `settled` events as movement into `claimable_balance`, not as recipient
  payment.
- Treat `withdrawn` events as actual on-ledger recipient payment.
- Use the stream version and schema docs when decoding stored vesting state.
- Do not infer vesting completion from wall-clock time alone; use ledger
  timestamps and observed settlement/withdrawal events.

## Tests

The vesting behavior is covered in `src/lib.rs` by tests including:

- `test_vesting_unlocks_linearly_across_multiple_settlements`
- `test_vesting_releases_rounding_remainder_at_end`
- `test_vesting_schedule_anchor_persists_across_restart`
- `test_vesting_unlocks_full_balance_after_duration`
