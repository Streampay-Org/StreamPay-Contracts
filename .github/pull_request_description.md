## feat(contracts): optional recipient-initiated stream stop

### Summary

This PR implements issue #8 — configurable permission for the recipient to stop a payment stream. Previously only the payer could call `stop_stream`. With this change, the stream creator (payer) can opt-in at creation time to allow the recipient to also stop the stream. The default remains payer-only, so no existing behaviour is broken.

---

### Motivation

In many real-world streaming payment scenarios the recipient may need to terminate a stream themselves — for example, to stop receiving funds they no longer want, or to close out a contract early. Without this capability, the recipient is entirely dependent on the payer to act, which creates an unnecessary power imbalance. This feature gives recipients agency while keeping it strictly opt-in so payers retain full control by default.

---

### Changes

#### `src/lib.rs`

**`StreamInfo` struct**
- Added `recipient_can_stop: bool` field.
- Stored in persistent ledger storage alongside all other stream metadata.
- Serialised/deserialised automatically via `#[contracttype]`.

**`create_stream`**
- New parameter: `recipient_can_stop: bool`.
- Payer sets this flag at creation; it cannot be changed after the stream is created.
- Passing `false` (or omitting in future SDK wrappers) preserves the original payer-only behaviour.

**`stop_stream`**
- New parameter: `stopper: Address` — the address attempting to stop the stream.
- Auth is enforced unconditionally via `stopper.require_auth()` **before** any permission check, making it impossible to bypass.
- Permission logic after auth:
  - `stopper == payer` → always allowed.
  - `stopper == recipient && recipient_can_stop == true` → allowed (opt-in).
  - Anything else → panics with `"not authorised to stop stream"`.
- This is the idiomatic Soroban OR-auth pattern: require auth on the caller's address, then validate the caller is a permitted party.

**`README.md`**
- Updated contract interface documentation to reflect the new parameters on `create_stream`, `stop_stream`, and the new `recipient_can_stop` field in `get_stream_info`.

---

### Security notes

- `stopper.require_auth()` is called **before** the permission check. There is no code path that reaches the permission logic without a valid Soroban auth entry for `stopper`. Auth cannot be bypassed.
- The `recipient_can_stop` flag is immutable after creation. A payer cannot retroactively grant or revoke recipient stop-permission on a live stream.
- Third-party addresses (neither payer nor recipient) are always rejected, even if they provide a valid auth signature.
- The payer's ability to stop is unconditional and unaffected by the flag value.

---

### Tests

17 tests total, all passing (`cargo test`).

New tests added for this feature:

| Test | Description |
|------|-------------|
| `test_recipient_can_stop_when_flag_set` | Recipient successfully stops an active stream when `recipient_can_stop = true` |
| `test_payer_can_stop_when_recipient_flag_set` | Payer can still stop even when the recipient flag is enabled |
| `test_recipient_cannot_stop_when_flag_false` | Recipient is rejected when `recipient_can_stop = false` (should_panic) |
| `test_third_party_cannot_stop_stream` | Unrelated address is rejected even with valid auth (should_panic) |
| `test_recipient_can_stop_flag_stored` | Flag is correctly persisted for both `true` and `false` values |

Existing tests updated to pass the new `stopper` argument to `stop_stream` and the new `recipient_can_stop` argument to `create_stream`.

---

### Test output

```
running 17 tests
test test::test_version_is_positive ... ok
test test::test_version_matches_const ... ok
test test::test_version_returns_expected ... ok
test test::test_create_stream_extends_ttl ... ok
test test::test_create_stream_valid ... ok
test test::test_settle_returns_amount ... ok
test test::test_third_party_cannot_stop_stream - should panic ... ok
test test::test_stream_uses_persistent_storage ... ok
test test::test_archive_active_stream_panics - should panic ... ok
test test::test_archive_unsettled_stream_panics - should panic ... ok
test test::test_recipient_can_stop_when_flag_set ... ok
test test::test_start_and_stop_stream ... ok
test test::test_archive_settled_stream ... ok
test test::test_recipient_cannot_stop_when_flag_false - should panic ... ok
test test::test_recipient_can_stop_flag_stored ... ok
test test::test_archived_stream_not_found - should panic ... ok
test test::test_payer_can_stop_when_recipient_flag_set ... ok

test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

### Edge cases covered

- Recipient with flag `false` cannot stop (default payer-only behaviour preserved).
- Payer can always stop regardless of flag value.
- Third-party address rejected even with valid Soroban auth.
- Flag is stored and retrieved correctly for both `true` and `false`.
- All existing archive, settle, and TTL tests continue to pass unmodified in logic.

---

Closes issue #8
