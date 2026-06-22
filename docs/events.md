# Contract Events

`streampay-contracts` emits events for every state-changing entry point so
off-chain indexers can rebuild stream history without polling
`get_stream_info`. This page lists the event topics, their payload shape,
and the canonical interpretation of each field.

## Topics

Every event uses a two-symbol topic tuple `(Symbol, Symbol)`. The first
symbol is the constant `"stream"`; the second is the action name.

| Topic | Emitted by | Payload |
|---|---|---|
| `(stream, created)` | `create_stream` | `(stream_id, payer, recipient, rate_per_second, initial_balance)` |
| `(stream, started)` | `start_stream` | `(stream_id, start_time)` |
| `(stream, stopped)` | `stop_stream` | `(stream_id, stopper, stop_time)` |
| `(stream, settled)` | `settle_stream` | `(stream_id, amount, new_balance, new_claimable)` |
| `(stream, withdrawn)` | `withdraw_stream` | `(stream_id, recipient, amount)` |
| `(stream, archived)` | `archive_stream` | `(stream_id, payer)` |
| `(stream, paused)` | `pause_stream` | `(stream_id, pause_time)` |
| `(stream, resumed)` | `resume_stream` | `(stream_id, resume_time)` |

## Field notes

- `stream_id` is the `u32` returned by `create_stream`.
- Time fields are Unix seconds matching `Env::ledger().timestamp()`.
- `amount`, `balance`, and `claimable` fields are `i128`.

## Indexing recommendations

- Maintain `(stream_id -> running_total_claimed)` by summing `withdrawn`
  amounts; never sum `settled` amounts, since settlements may be batched
  and overlap with the same withdrawal window.
- Track `claimable_balance` from settled/withdrawn events if the indexer needs
  pending recipient withdrawals.
- Use `(stream, archived)` as the signal to delete the stream from your
  active index. Once observed, no further events for that id can occur.
- Topic ordering inside a single transaction is deterministic and matches
  the contract call order, so consumers can rely on it for causality.
