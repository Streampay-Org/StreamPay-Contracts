# Operator Delegation

The StreamPay contract supports optional operator delegation, allowing payers to designate trusted addresses that can manage their payment streams on their behalf.

## Overview

- **Per-stream delegation**: Each stream can have its own operator, set by the payer.
- **Strict authorization**: Only the payer or designated operator can perform management actions (start, stop, settle).
- **Revocable**: Payers can change or remove operators at any time.
- **Secure**: All actions require authentication from the authorized party.

## Functions

### `set_operator(stream_id, operator)`

- **Caller**: Payer only
- **Purpose**: Set or revoke the operator for a specific stream
- **Parameters**:
  - `stream_id`: The stream to modify
  - `operator`: `Some(Address)` to set, `None` to revoke

### Management Functions

`start_stream`, `stop_stream`, and `settle_stream` now require an additional `caller` parameter:

- **Authorization check**: `caller == payer || caller == operator`
- **Authentication**: `caller.require_auth()` ensures the caller is authorized

## Usage

1. Payer creates a stream
2. Payer calls `set_operator(stream_id, Some(operator_address))`
3. Operator can now call `start_stream(operator_address, stream_id)`, etc.
4. Payer can revoke with `set_operator(stream_id, None)`

## Security Notes

- Operators have full control over stream management but cannot modify the stream configuration or archive streams
- Archive remains payer-only to protect recipient entitlements
- All delegations are opt-in per stream