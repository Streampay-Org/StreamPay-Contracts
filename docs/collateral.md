# Collateral & Lockup Escrow Design

> **Status:** Draft · **Issue:** #51 · **Author:** *(contributor name)*
> **Scope:** Design documentation only — no MVP code changes unless explicitly approved.

## 1. Motivation

StreamPay enables continuous payment streaming between a payer and recipient.
The current contract (`v0.1.0`) holds a `balance` field inside each `StreamInfo`
struct that decrements as the stream settles. This balance is the *only* economic
guarantee backing the stream.

For **high-risk flows** — large-value B2B payments, cross-border payroll,
protocol-to-protocol composability — a bare stream balance is insufficient:

| Risk | Current Mitigation | Gap |
|------|--------------------|-----|
| Payer tops up less than committed | None — stream drains to zero and halts | No penalty or pre-commitment |
| Recipient depends on future flow | Trust assumption on payer solvency | No collateral backing |
| Stream used as collateral by third party | Not possible — streams are IDs, not addresses | Requires factory pattern (Phase 2) |
| Dispute between payer and recipient | No on-chain arbitration | No escrow release mechanism |

This document proposes collateral and lockup escrow patterns that close these
gaps in a phased approach aligned with the storage migration roadmap
(`docs/factory-pattern.md`).

## 2. Escrow Models

### 2.1 Lockup Deposit (Phase 1 — Singleton)

A **lockup deposit** is additional funds the payer locks at stream creation time,
held separately from the stream's operational balance. The lockup is returned to
the payer only when the stream completes its full committed duration or is
mutually cancelled.

**Mechanism (conceptual — no code change in MVP):**

1. `create_stream()` accepts an optional `lockup_amount: i128` parameter.
2. `lockup_amount` is transferred to the contract and stored alongside the stream.
3. On `settle_stream()` after `end_time`: lockup is released back to payer.
4. On premature `stop_stream()` by payer: lockup is forfeited to recipient as
   compensation (partial or full, per policy).
5. On mutual cancellation (future entry point): lockup returned to payer minus
   any pro-rata penalty.

**Storage impact:** One additional `i128` field per `StreamInfo`. Compatible with
the planned persistent-storage migration — no architectural change needed.

```
┌─────────────────────────────────────────┐
│              StreamInfo (v2)             │
├─────────────────────────────────────────┤
│  payer: Address                         │
│  recipient: Address                     │
│  rate_per_second: i128                  │
│  balance: i128         ← operational    │
│  lockup_amount: i128   ← NEW: escrow   │
│  start_time: u64                        │
│  end_time: u64                          │
│  is_active: bool                        │
└─────────────────────────────────────────┘
```

**When to use:** Medium-risk flows where payer commitment assurance is needed but
full escrow isolation is overkill (e.g., freelancer payroll, recurring SaaS fees).

### 2.2 Full Escrow with Release Conditions (Phase 1 — Singleton)

A **full escrow** locks the *entire* stream value upfront. The stream's
`balance` equals the total committed payout. Settlement releases funds to the
recipient progressively; the payer cannot withdraw uncommitted funds until the
stream concludes or is cancelled under defined conditions.

**Mechanism (conceptual):**

1. `create_stream()` requires `initial_balance >= rate_per_second * duration`.
2. Contract enforces that `balance` cannot be reduced by the payer except through
   settlement.
3. A `release_policy` enum governs cancellation:
   - `NonRefundable` — payer forfeits remaining balance to recipient on cancel.
   - `ProRata` — settled amount to recipient, remainder to payer.
   - `Arbitrated` — third-party address must co-sign cancellation.

**Storage impact:** One `release_policy` enum field per stream. Fits singleton
persistent storage.

**When to use:** High-risk flows — large B2B payments, cross-border payroll
where recipient needs hard guarantees.

### 2.3 Composable Collateral via Factory (Phase 2)

When the factory pattern is adopted (see `docs/factory-pattern.md` §Phase 2),
each stream becomes its own contract address. This unlocks:

- **Stream-as-collateral:** A lending protocol can accept a stream contract
  address as collateral, query its remaining value, and liquidate on default.
- **Transferable escrow:** Stream ownership (payer/recipient roles) can be
  reassigned, enabling invoice factoring.
- **Multi-party escrow:** Milestone-based releases where an oracle or DAO
  triggers partial settlements.

**Dependency:** Requires factory pattern deployment. Out of scope for Phase 1.

## 3. Risk Analysis

### 3.1 Threat Model

| # | Threat | Severity | Affected Model | Mitigation | Residual Risk |
|---|--------|----------|----------------|------------|---------------|
| T1 | **Payer drains lockup via re-entrancy** — malicious token callback re-enters `settle_stream` to double-claim | Critical | 2.1, 2.2 | Soroban's execution model is single-threaded per invocation; no re-entrancy possible in current runtime. Validate on future SDK upgrades. | Low — runtime guarantee, not code-level. |
| T2 | **Lockup griefing** — payer creates stream with lockup but never starts it, locking recipient expectation | Medium | 2.1 | Add expiry: if stream not started within `max_start_delay` ledgers, lockup auto-refunds to payer and stream is archived. | Low with expiry. |
| T3 | **Oracle manipulation (Arbitrated policy)** — compromised arbitrator co-signs fraudulent cancellation | High | 2.2 (Arbitrated) | Require M-of-N multi-sig for arbitrated releases. Recommend time-locked dispute window before release. | Medium — depends on arbitrator trust model. |
| T4 | **Collateral valuation drift (Phase 2)** — stream's remaining value drops below collateral requirements between oracle updates | High | 2.3 | Lending protocol must implement health-factor checks with sufficient margin. StreamPay exposes `get_stream_info()` for real-time queries — no stale oracle needed for on-chain value. | Medium — protocol-level, not StreamPay-level. |
| T5 | **Lockup amount overflow** — `lockup_amount + balance` exceeds `i128::MAX` | Low | 2.1, 2.2 | Validate `lockup_amount + balance` does not overflow at `create_stream()`. Use checked arithmetic. | Negligible with validation. |
| T6 | **Denial-of-service via dust lockups** — attacker creates thousands of minimum-lockup streams to bloat storage | Medium | 2.1, 2.2 | Enforce minimum lockup threshold. Persistent storage TTL auto-expires inactive streams. | Low with TTL + minimum. |
| T7 | **Front-running cancellation** — payer cancels stream just before a large settlement to recover lockup | High | 2.1, 2.2 | `stop_stream()` must settle all accrued value *before* processing cancellation. Lockup forfeiture on unilateral payer cancel. | Low with settle-first invariant. |
| T8 | **Phantom lockup (accounting-only gap)** — `lockup_amount` field is set but no actual token transfer occurs, giving false collateral guarantees | High | 2.1, 2.2 | Current contract is accounting-only; token integration is a separate workstream. `lockup_amount` MUST NOT be trusted as collateral until token custody is implemented. Document this limitation prominently. | High until token transfers land. |

### 3.2 Security Invariants

The following invariants MUST hold for any collateral/escrow implementation:

1. **Settle-before-cancel:** Any cancellation path must first settle all accrued
   value to the recipient. No path may allow the payer to recover funds that
   have already been earned by elapsed time.

2. **Lockup isolation:** Lockup funds are not part of the settleable balance.
   `settle_stream()` draws from `balance` only; `lockup_amount` is released or
   forfeited exclusively through terminal stream events (completion, cancellation,
   expiry).

3. **No unilateral recipient withdrawal of lockup:** The recipient receives
   lockup funds only through defined forfeiture rules, never by direct claim.

4. **Arithmetic safety:** All operations on `balance`, `lockup_amount`, and
   `rate_per_second * elapsed` use checked arithmetic. Overflow panics the
   transaction (Soroban default) rather than wrapping.

5. **Authorization consistency:** Lockup and escrow operations follow the same
   authorization model as existing entry points (see `docs/factory-pattern.md`
   §5 — Authorization model subsection). `create_stream` requires payer auth; `settle_stream`
   remains permissionless; cancellation requires payer auth (or multi-sig for
   arbitrated policy).

## 4. Phase Roadmap

| Phase | Milestone | Escrow Capability | Dependency |
|-------|-----------|-------------------|------------|
| **0 (current)** | Singleton + instance storage | None — `balance` only | — |
| **1a** | Persistent storage migration | Lockup deposit field added to `StreamInfo` | `docs/factory-pattern.md` §5 (Recommended Architecture) |
| **1b** | Release policy enum | Full escrow with `NonRefundable` / `ProRata` / `Arbitrated` | Phase 1a |
| **2** | Factory pattern | Composable collateral — stream-as-address | Factory deployment |

### Graduation Triggers (Phase 1 → Phase 2)

Collateral features graduate from singleton to factory when:

- A partner protocol requests stream-address composability for lending/collateral.
- Lockup usage exceeds 30% of active streams (signal of high-risk flow demand).
- The factory pattern is deployed and battle-tested for non-collateral streams.

These align with the graduation triggers in `docs/factory-pattern.md` §6.2 (Graduation triggers).

## 5. Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-24 | Collateral is documentation-only for MVP | Risk analysis complete; no code until Phase 1a persistent storage lands. Avoids premature abstraction. |
| 2026-03-24 | Lockup is a separate field, not part of `balance` | Isolation invariant — prevents settle logic from touching escrowed funds. Simpler to audit. |
| 2026-03-24 | Three release policies (NonRefundable, ProRata, Arbitrated) | Covers spectrum from no-trust to partial-trust flows without over-engineering. |
| 2026-03-24 | Phase 2 composable collateral deferred to factory | Streams must be contract addresses for third-party collateral use. Singleton IDs are not composable. |

## 6. Open Questions

- [ ] Should `lockup_amount` be denominated in the stream's token or a separate
  stablecoin? (Affects cross-asset risk.)
- [ ] What is the minimum lockup threshold to prevent dust griefing? (Needs gas
  cost analysis on Stellar.)
- [ ] Should the `Arbitrated` release policy support on-chain dispute evidence
  (e.g., hash of off-chain ruling)?
- [ ] How should lockup interact with `archive_stream()`? (Auto-refund on
  archive, or require explicit claim?)
- [ ] Token custody: `lockup_amount` is currently accounting-only (no token
  transfers). When token integration lands, lockup must require actual token
  custody before being treated as collateral. (See T8 in threat model.)

## 7. References

- [StreamPay Factory Pattern Design](factory-pattern.md) — storage architecture
  and Phase 2 factory roadmap.
- [Soroban Storage Docs](https://soroban.stellar.org/docs/storage) — persistent
  vs. instance storage semantics.
- [Sablier V2 Protocol](https://docs.sablier.com/) — prior art for on-chain
  payment streaming with lockups.
- [Superfluid Protocol](https://docs.superfluid.finance/) — real-time finance
  streaming patterns.
