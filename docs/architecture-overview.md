# Architecture Overview

`streampay-contracts` is a single Soroban smart-contract crate. It is
deliberately small so that the surface area auditors must verify stays
manageable. This page summarizes the layout so a new contributor can find
their way around in a few minutes.

## Crate layout

```
src/
  lib.rs       Public contract entry points, version constant, top-level docs.
  stream.rs    Stream storage primitives: StreamInfo type and helpers.
```

The crate is `crate-type = ["cdylib", "rlib"]` so it can be consumed both
as a deployable WASM and as a Rust library by integration tests.

## Module boundaries

- `lib.rs` is the only module that calls `#[contractimpl]` methods.
- `stream.rs` is the only module that builds the `("stream", id)`
  persistent-storage key. All reads and writes flow through `get_stream`,
  `set_stream` and `extend_stream_ttl`.
- Event emission, auth checks, and rate-update policy live in `lib.rs`
  for now. As they grow, splitting them into `events.rs`, `auth.rs`, and
  `rate.rs` is the planned refactor (tracked in `docs/audit-readiness.md`).

## Data flow

```
            create_stream                          withdraw_stream
                 |                                       |
  payer ---> StreamInfo{is_active=false} ---> start_stream ---> is_active=true
                                                      |
                                                      v
                                              settle_stream (anyone)
                                                      |
                                                      v
                                              balance --> claimable_balance
                                                                |
                                                                v
                                                         recipient receives
```

`stop_stream` flips `is_active` back to `false` at any point; settlement
becomes a no-op until `start_stream` is invoked again.

## External surface

| Entry point | Auth | Side effect |
|---|---|---|
| `create_stream` | payer | Allocates a new id, writes `StreamInfo`. |
| `start_stream` | payer | Sets `start_time`, `is_active = true`. |
| `stop_stream` | payer (or recipient if flagged) | Sets `is_active = false`. |
| `settle_stream` | none | Moves accrued tokens to `claimable_balance`. |
| `batch_settle` | none | Settle up to 25 streams atomically. |
| `withdraw_stream` | recipient | Settles + transfers `claimable_balance` on-ledger. |
| `archive_stream` | payer | Deletes the persistent entry once fully drained. |
| `get_stream_info` | none | Returns the stored `StreamInfo`. |
| `version` | none | Returns the packed `u32` semver constant. |
