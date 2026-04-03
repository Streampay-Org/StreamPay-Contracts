# Token Callback & Reentrancy Security Analysis

**Document:** `docs/security-token-callbacks.md`  
**Contract:** `StreamPay-Contracts` (Soroban / Rust)  
**Version:** 0.2.0 (`VERSION = 2_000`)  
**Branch:** `security/token-reentrancy-analysis`  
**Status:** Normative security documentation

---

## 1. Executive Summary

StreamPay v0.2.0 is **not vulnerable to reentrancy attacks**. This conclusion
rests on two independent layers of protection:

1. **The Soroban host enforces single-contract execution per invocation** —
   there is no mechanism by which a callee can re-enter the calling contract
   within the same transaction.
2. **StreamPay does not call external token contracts** — `settle_stream`
   updates an internal accounting balance only; no `transfer` or
   `transfer_from` is invoked anywhere in the contract code.

The sections below document both layers in detail, identify the hypothetical
attack surface that would exist if token calls were ever added, and provide
guidance for future contributors.

---

## 2. Soroban Reentrancy Model

### 2.1 How Soroban prevents reentrancy at the host level

Soroban (Stellar's smart contract platform) enforces **call-frame isolation**
at the host layer. Each contract invocation runs inside a dedicated host
frame. The Soroban host tracks a set of *authorisation records* and *storage
footprints* that are scoped to the current transaction; a contract cannot
observe or modify another contract's in-progress state mid-execution.

Key properties from the Soroban specification
([docs.stellar.org/docs/learn/smart-contract-internals](https://docs.stellar.org/docs/learn/smart-contract-internals)):

- **No callback mechanism for token contracts.** The Soroban token interface
  (`soroban-token-interface`) exposes `transfer`, `transfer_from`, `approve`,
  etc. These are synchronous calls that return a value; they do **not** invoke
  any callback on the caller contract. There is no ERC-777-style
  `tokensReceived` hook, no ERC-721 `onERC721Received` equivalent, and no
  `receive`/`fallback` function concept in Soroban.

- **No delegatecall.** Soroban has no equivalent of Ethereum's `DELEGATECALL`
  opcode. A cross-contract call executes inside the *callee's* storage and
  code context, never the caller's.

- **Deterministic execution order.** Within a single transaction, calls are
  evaluated depth-first and return synchronously. There is no async callback
  queue that could be used to re-enter a suspended frame.

- **Storage writes are buffered.** The host accumulates storage mutations in a
  transaction-local write buffer. If a transaction panics at any point, all
  writes are rolled back atomically. A partially-executed reentrancy would
  produce a panic (the re-entered function would attempt to acquire auth that
  is already consumed), and the entire transaction would revert.

### 2.2 Comparison with Ethereum reentrancy

| Property | Ethereum (EVM) | Soroban |
|---|---|---|
| External call can re-enter caller | ✅ Yes (`CALL` mid-execution) | ❌ No (synchronous, no callbacks) |
| Token transfer hooks | ✅ ERC-777 `tokensReceived` | ❌ None in SEP-41 token interface |
| Delegatecall | ✅ `DELEGATECALL` | ❌ Not available |
| Partial state visible during call | ✅ Yes | ❌ No (write buffer) |
| Classic checks-effects-interactions required | ✅ Yes | ❌ Not required (but good practice) |

Reentrancy as it exists in Solidity is a **non-issue on Soroban** by design.

---

## 3. StreamPay Token Call Audit

### 3.1 Current token calls: none

A complete audit of `src/lib.rs` (v0.2.0) finds **zero invocations** of any
token contract:

| Function | External calls | Token calls |
|---|---|---|
| `create_stream` | None | None |
| `start_stream` | None | None |
| `stop_stream` | None | None |
| `settle_stream` | None | None |
| `archive_stream` | None | None |
| `set_max_rate` | None | None |
| `remove_max_rate` | None | None |
| `get_stream_info` | None | None |
| `get_max_rate` | None | None |
| `version` | None | None |

All storage operations are on `env.storage().persistent()` (stream records)
and `env.storage().instance()` (stream ID counter, max-rate ceiling). No
`env.invoke_contract` or `TokenClient` calls exist anywhere in the codebase.

### 3.2 Balance field semantics

`StreamInfo.balance` is a **pure accounting integer** — it represents the
payer's deposited token allocation tracked internally. It is not backed by an
on-chain token escrow in the current implementation. `settle_stream` deducts
from this integer:

```rust
info.balance = info.balance.saturating_sub(amount);
```

No tokens move. The actual disbursement of funds is handled at the application
layer outside this contract.

> **Implication for integrators:** Integrators who wrap StreamPay with an
> escrow layer (i.e. a contract that holds tokens and calls `settle_stream`
> before releasing them) should follow the pattern in §4 to ensure their
> wrapper remains safe if Soroban's token semantics ever evolve.

---

## 4. Hypothetical Attack Surface (future token integration)

If a future version adds real token transfers — for example:

```rust
// hypothetical future code
token_client.transfer(&env, &info.payer, &info.recipient, &amount);
```

— the following analysis would apply.

### 4.1 Why reentrancy is still structurally prevented

Even with a `transfer` call, reentrancy cannot occur because:

1. The SEP-41 token interface does not invoke callbacks on the recipient.
2. The Soroban host does not allow a callee to re-enter the original contract
   frame.

The call graph for a hypothetical `settle_stream` with token transfer would be:

```
User tx
  └─ settle_stream (StreamPay)
       ├─ read StreamInfo           [storage read]
       ├─ compute amount            [pure arithmetic]
       ├─ write updated StreamInfo  [storage write ← state committed BEFORE transfer]
       └─ token.transfer(...)       [cross-contract call, no callback possible]
```

### 4.2 Checks-Effects-Interactions as defence-in-depth

Even though reentrancy is not possible in Soroban, StreamPay should adopt the
**checks-effects-interactions (CEI)** pattern as a matter of defence-in-depth
and auditability. The CEI ordering for a future token-integrated
`settle_stream` would be:

```
1. CHECK  — is_active, compute elapsed/amount        (no state change)
2. EFFECT — write balance', start_time' to storage   (state committed)
3. INTERACT — token.transfer(payer → recipient)      (external call last)
```

This ordering ensures that even in a theoretical future Soroban environment
where callbacks were introduced, the state would already be consistent before
any external code runs.

The current implementation already follows CEI implicitly (there is no
interaction step), so no code changes are required now.

### 4.3 Integer overflow in token amounts

`settle_stream` uses `saturating_mul` and `saturating_sub`:

```rust
let amount = (elapsed as i128)
    .saturating_mul(info.rate_per_second)
    .min(info.balance);
info.balance = info.balance.saturating_sub(amount);
```

Both guards mean overflow cannot produce an incorrect token transfer amount.
The `min(info.balance)` cap ensures `amount ≤ balance` always holds, so
`saturating_sub` will never actually saturate in practice.

---

## 5. Auth & Access Control Review

Reentrancy is one class of callback attack; auth bypass via confused-deputy is
another. The table below audits every state-mutating function:

| Function | Auth required | Auth target | Risk |
|---|---|---|---|
| `create_stream` | ✅ | `payer` | None — payer must approve their own stream |
| `start_stream` | ✅ | `info.payer` | None — only payer can activate |
| `stop_stream` | ✅ | `info.payer` | None — only payer can deactivate |
| `settle_stream` | ❌ | — | **Intentional** — anyone may trigger settlement; state update is safe because amount is capped by balance and cannot increase beyond what is owed |
| `archive_stream` | ✅ | `info.payer` | None — payer-only, balance=0 guard protects recipient |
| `set_max_rate` | ✅ | `caller` | Low — caller is any address; see note below |
| `remove_max_rate` | ✅ | `caller` | Low — same |

> **Note on `set_max_rate` auth:** The `caller` pattern means any address can
> configure the ceiling for their own streams. There is no contract-level
> admin. This is intentional (see `lib.rs` module doc) but operators running
> shared deployments should consider adding an admin-address check if they
> need exclusive control over the ceiling.

### 5.1 `settle_stream` open-call analysis

`settle_stream` requires no auth by design — this is standard in streaming
payment protocols so that recipient (or anyone) can trigger disbursement
without relying on payer cooperation. The safety argument:

- Amount is derived entirely from on-chain state (`start_time`, `now`,
  `rate_per_second`, `balance`).
- Amount is capped at `balance` — cannot exceed deposited funds.
- State (`balance`, `start_time`) is updated atomically before any external
  interaction (and there are no external interactions currently).
- Repeated calls in the same ledger second return `amount = 0` (elapsed = 0),
  making it **idempotent within a ledger**.

No griefing vector exists: an adversary calling `settle_stream` repeatedly
can only accelerate the flow of funds toward the intended recipient.

---

## 6. Security Notes for SECURITY.md

The following text is suitable for inclusion in `SECURITY.md` or a top-level
security section:

---

### Reentrancy

StreamPay is not vulnerable to reentrancy. The Soroban host provides
structural reentrancy prevention: cross-contract calls are synchronous, the
SEP-41 token interface does not invoke callbacks, and there is no delegatecall
primitive. StreamPay v0.2.0 additionally makes zero external contract calls,
eliminating any interaction surface entirely.

Future versions that add token transfers must follow the
checks-effects-interactions pattern (state writes before `token.transfer`)
and document any new external call sites.

### Token Transfers

`settle_stream` updates an internal accounting balance only. No on-chain token
movement occurs inside this contract. Integrators implementing token escrow
wrappers are responsible for their own transfer safety.

### Integer Arithmetic

All arithmetic on stream amounts uses `saturating_mul` and `saturating_sub`.
The `min(balance)` cap guarantees `amount ≤ balance` invariant. No overflow,
underflow, or wrap-around is possible in amount calculations.

### Open Settlement

`settle_stream` is intentionally callable by any address. This cannot be
exploited: the amount is bounded by `balance`, and repeated calls within the
same ledger second produce zero. No funds can be misdirected.

---

## 7. Recommendations

| Priority | Recommendation |
|---|---|
| Low | Add `// SECURITY: CEI order maintained` comment above state writes in `settle_stream` and any future token-integrated functions |
| Low | Consider restricting `set_max_rate` to a designated admin address for shared deployments |
| Informational | When token transfers are added in a future version, add an integration test that verifies the `balance` field reaches zero before `transfer` is called (mock token client) |
| Informational | Reference this document in `SECURITY.md` under a "Reentrancy" heading |

---

## 8. References

- [Soroban Smart Contract Internals](https://developers.stellar.org/docs/learn/encyclopedia/contract-development/contract-interactions/stellar-transaction)
- [SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [Soroban Auth documentation](https://developers.stellar.org/docs/learn/encyclopedia/security/authorization)
- [Checks-Effects-Interactions pattern](https://docs.soliditylang.org/en/latest/security-considerations.html#reentrancy)
- StreamPay `src/lib.rs` v0.2.0

---

## 9. Version History

| Version | Change |
|---|---|
| 0.2.0 | Initial analysis; no token calls present; reentrancy structurally prevented by Soroban host |