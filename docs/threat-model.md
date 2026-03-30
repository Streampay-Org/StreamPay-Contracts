# Threat Model: StreamPay-Contracts

This document outlines the threat model for the StreamPay Soroban smart contracts. It identifies potential attack vectors, their impact, and the corresponding mitigations and residual risks.

## Overview

StreamPay provides continuous payment streaming. The focus of this threat model is on the **accounting and state management** of payment streams.

> [!IMPORTANT]
> **Accounting-Only Logic**: The current contract version tracks stream balances but does **not** handle actual token transfers. Users must NOT deposit tokens directly to this contract address until token integration is officially supported.

## Threat Analysis

| Threat                         | Impact                                                                                  | Mitigation                                                                                                                                                | Residual Risk                                                                               |
| :----------------------------- | :-------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------ |
| **Auth Spoofing**              | Unauthorized creation, starting, stopping, or archiving of streams.                     | Critical functions (`create`, `start`, `stop`, `archive`) use `require_auth()` for the payer.                                                             | Low, provided Soroban's authorization model is correctly implemented.                       |
| **Payer Griefing** (Stopping)  | Payer stops the stream prematurely, depriving the recipient of expected future income.  | This is an inherent feature of the streaming model (pay-as-you-go).                                                                                       | Expected behavior; recipient should monitor stream status.                                  |
| **Payer Griefing** (Archiving) | Payer attempts to remove a stream before the recipient has settled their entitlements.  | `archive_stream` requires `is_active == false` AND `balance == 0`. Settlement must happen before balance reaches zero.                                    | Low. Recipient can settle at any time to claim their portion before archival.               |
| **Recipient Griefing**         | Recipient is unable to settle or manage their entitlements.                             | `settle_stream` is permissionless (no auth required), allowing anyone (recipient, keeper, or bot) to trigger settlement.                                  | Low. Public settlement ensures the recipient can always claim their share.                  |
| **Settlement Ordering**        | Race conditions or manipulation of settlement timestamps to over-claim or double-claim. | Soroban's sequential transaction execution and state-based settlement logic (`start_time` updates) prevent double-claiming. Uses `saturating` arithmetic. | Negligible. State updates are atomic within a transaction.                                  |
| **Storage Expiry**             | Stream data expires from ledger, leading to loss of accounting and unsettled balances.  | `extend_ttl` is called in every state-modifying function and archival process.                                                                            | Medium. Long-inactive streams must be settled/interacted with periodically to avoid expiry. |
| **User Misuse** (Funds Loss)   | Users sending tokens directly to the contract address.                                  | Clear documentation and "Accounting-Only" scope.                                                                                                          | **HIGH**. The contract lacks token withdrawal logic. Users must be warned.                  |

## Mitigation Status

| Feature                 | Status     | Notes                                                            |
| :---------------------- | :--------- | :--------------------------------------------------------------- |
| **Authorization**       | ✅ Active  | Enforced via `require_auth()`.                                   |
| **Griefing Protection** | ✅ Active  | Public `settle_stream` and strict `archive_stream` checks.       |
| **Arithmetic Safety**   | ✅ Active  | Uses `saturating_mul` and `saturating_sub`.                      |
| **Storage Safety**      | ✅ Active  | TTL extension strategy implemented for all mutations.            |
| **Asset Security**      | ⚠️ Pending | Contract currently purely accounting; token logic is in Phase 2. |

---

_For vulnerability reporting, please refer to [SECURITY.md](../SECURITY.md)._
