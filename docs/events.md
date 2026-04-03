# Contract Events

This document describes the events emitted by the StreamPay contract.

## Settle Event

Emitted when `settle_stream` moves value from the stream balance to the recipient.

- **Topics**: `("settle",)`
- **Data**: `SettleEvent`
  - `stream_id: u32` - The ID of the settled stream
  - `amount: i128` - The amount settled (deducted from balance)
  - `post_balance: i128` - The stream balance after settlement

This event helps backends reconcile stream settlements without sensitive data beyond what's already on-chain.