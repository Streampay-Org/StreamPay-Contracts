# Contract Events

`streampay-contracts` emits events for state-changing entry points so
off-chain indexers can rebuild stream history without polling
`get_stream_info`. This page lists the event topics, their payload shape,
and the canonical interpretation of each field.

## Topics

Events use a two-element topic tuple `(Symbol, u32)`:

| Topic[0] | Topic[1] | Emitted by | Payload |
|---|---|---|---|
| `stream_created` | `stream_id` | `create_stream` | `StreamCreatedEvent { payer, recipient, rate_per_second, initial_balance }` |

Additional lifecycle events (`started`, `stopped`, `settled`, `withdrawn`,
`archived`) are planned; v0.2.0 currently emits `stream_created` only.

## Field notes

- `stream_id` is the `u32` returned by `create_stream`.
- `rate_per_second` and `initial_balance` are `i128` token units.
- Addresses are standard Soroban `Address` values.

## Indexing recommendations

- Use `stream_created` as the signal to open a new stream record.
- Sum `withdraw_stream` return values off-chain (or watch future `withdrawn`
  events) to track total disbursed — do not sum `settle_stream` amounts alone,
  since settlement does not transfer tokens on-ledger.
- Once `archive_stream` is called, the persistent entry is removed; no further
  events for that id can occur.
