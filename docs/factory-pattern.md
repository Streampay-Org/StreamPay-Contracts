# Factory Pattern Design: Factory-Deployed Children vs Singleton Multi-Stream

**Issue:** #46 — Stream factory pattern: design document
**Status:** Proposal
**Author:** StreamPay Contributors
**Date:** 2026-03-24

---

## 1. Problem Statement

StreamPay currently stores all payment streams in Soroban **instance storage**.
Instance storage is a single ledger entry shared across the entire contract
instance — every stream's data is bundled together.

This creates three scaling concerns:

| Concern | Detail |
|---------|--------|
| **Storage ceiling** | Instance storage has a practical size limit (~64 KB). Each `StreamInfo` is ~200 bytes, giving a ceiling of roughly 300 concurrent streams before storage pressure. |
| **TTL coupling** | Renewing the contract instance TTL renews *all* streams, including inactive or fully-settled ones. The contract pays to keep dead data alive. |
| **Blast radius** | A storage-level bug or corrupt entry affects the entire contract state — there is no per-stream isolation. |

### Current storage layout

```
Instance Storage (single ledger entry):
  Symbol("next_id")                    → u32
  (Symbol("stream"), 1u32)             → StreamInfo
  (Symbol("stream"), 2u32)             → StreamInfo
  …
  (Symbol("stream"), Nu32)             → StreamInfo
```

## 2. Approaches Evaluated

Three architectures were considered.

### 2.1 Singleton with Instance Storage (status quo)

Keep the current single contract. All streams remain in instance storage under
composite keys `("stream", id)`.

**Pros:**
- Simplest model — already implemented and tested.
- Cheapest at low stream counts (one ledger entry, one TTL renewal).
- One upgrade touches all streams.

**Cons:**
- ~300-stream ceiling before instance storage pressure.
- TTL renewal is all-or-nothing.
- No per-stream isolation.

### 2.2 Factory + Child Contracts

A factory contract deploys a new child contract per stream. The factory
maintains a registry mapping `stream_id → contract_address`.

**Pros:**
- Full isolation — each stream is a separate contract with independent storage
  and TTL.
- Streams are first-class contract addresses, composable with other protocols
  (e.g., a stream used as collateral).
- Per-stream upgrade control (with proxy pattern or re-deploy).

**Cons:**
- Highest cost per stream — contract deploy is ~100 000 stroops plus per-
  contract storage overhead.
- Significant complexity: WASM upload, `env.deployer()`, registry, cross-
  contract auth delegation.
- Upgrade coordination is harder — existing children keep old logic unless
  individually re-deployed.

### 2.3 Singleton with Persistent Storage (recommended)

Keep the single contract but migrate stream data from instance storage to
**persistent storage**. Each stream becomes its own ledger entry with an
independent TTL.

**Pros:**
- Per-stream ledger entries with independent TTLs — active streams stay alive,
  inactive ones can expire.
- Practically unlimited scaling (no shared-entry size limit).
- Minimal code change — swap storage accessor, add TTL extension calls.
- Upgrade path unchanged — one contract upgrade covers all streams.

**Cons:**
- Slightly higher per-entry cost (persistent storage rate vs instance rate).
- Shared contract logic — a logic bug still affects all streams (same as 2.1).
- Streams are IDs, not contract addresses — not directly composable.

## 3. Trade-off Matrix

| Criterion | Instance (2.1) | Persistent (2.3) ★ | Factory (2.2) |
|---|---|---|---|
| Stream isolation | None — shared entry | Per-stream entries, shared logic | Full — separate contracts |
| TTL management | All-or-nothing | Per-stream TTL | Per-contract TTL |
| Upgrade path | One upgrade, all streams | One upgrade, all streams | Per-child upgrade or re-deploy |
| Deploy cost | One contract | One contract | Factory + WASM + per-stream deploy |
| Storage cost | Cheapest at <50 streams | Slightly higher (persistent rate) | Highest (contract overhead) |
| Scaling ceiling | ~300 streams | Practically unlimited | Practically unlimited |
| Implementation complexity | Lowest (current code) | Low (storage swap + TTL) | High (deployer, registry, auth) |
| Failure blast radius | All streams | All streams (shared logic) | Single stream |
| Composability | Streams are IDs | Streams are IDs | Streams are contract addresses |

## 4. Recommendation

**Adopt Approach 2.3 — Singleton with Persistent Storage** as the primary
architecture for StreamPay v1.

### Rationale

1. StreamPay is at v0.1.0 — the contract surface is small and the singleton
   model is proven by existing tests.
2. Moving from instance to persistent storage removes the scaling bottleneck
   with a minimal diff.
3. Per-stream TTL management is the single highest-value improvement for
   production readiness.
4. The factory pattern adds deployer/registry/cross-contract complexity that is
   not justified at the current stage.

### Decision framework — "choose this when…"

| Choose… | When… |
|---------|-------|
| Singleton (Instance) | Prototyping, <50 concurrent streams, simplicity is the priority. |
| **Singleton (Persistent) ★** | Production use, moderate scale, per-stream TTL without factory complexity. |
| Factory + Children | Per-stream upgradability, multi-tenant isolation, or streams as first-class composable addresses. |

## 5. Recommended Architecture

### Target storage layout

```
Instance Storage:
  Symbol("next_id")                    → u32     ← small, always needed

Persistent Storage:
  (Symbol("stream"), 1u32)             → StreamInfo   ← independent ledger entry, own TTL
  (Symbol("stream"), 2u32)             → StreamInfo   ← independent ledger entry, own TTL
  …
  (Symbol("stream"), Nu32)             → StreamInfo   ← each is its own ledger entry
```

### What changes

- `env.storage().instance()` → `env.storage().persistent()` for all stream
  read/write operations.
- Add `env.storage().persistent().extend_ttl()` calls on stream mutation so
  active streams stay alive (see TTL strategy below).
- Add `env.storage().instance().extend_ttl()` on every mutating call to keep
  the contract instance itself alive (instance storage still holds `next_id`).
- Add an optional `archive_stream()` entry point to let payers explicitly
  remove fully-settled stream data via `env.storage().persistent().remove()`.
- `next_id` stays in instance storage (small, always needed).

### What stays the same

- All existing entry points (`create_stream`, `start_stream`, `stop_stream`,
  `settle_stream`, `get_stream_info`, `version`) — identical signatures,
  identical behavior.
- `StreamInfo` struct — no changes.
- Stream ID scheme — auto-incrementing `u32`.
- Single contract deployment model.

### Key behaviors

| Behavior | Detail |
|----------|--------|
| TTL per stream | `create_stream()`, `start_stream()`, `stop_stream()`, and `settle_stream()` extend the target stream's TTL. `get_stream_info()` is read-only and does not extend TTL. Inactive streams that are never touched naturally expire. |
| Instance TTL | Every mutating entry point also calls `env.storage().instance().extend_ttl()` to keep the contract itself alive. |
| Storage cost | ~200 bytes per stream at the persistent storage rate. Callers (or a keeper) extend TTLs as needed. |
| Deletion | Settled/completed streams can be explicitly archived via `env.storage().persistent().remove()` or left to expire via TTL. |
| Migration | StreamPay is pre-production (v0.1.0) — no on-chain migration needed; clean deploy. |

### TTL strategy

Soroban's `extend_ttl(threshold, extend_to)` API extends the entry's TTL to
`extend_to` ledgers only if the current TTL is below `threshold`. Recommended
defaults:

| Constant | Value | Rationale |
|----------|-------|-----------|
| `STREAM_TTL_THRESHOLD` | 17 280 ledgers (~1 day) | Extend before the last day of life. |
| `STREAM_TTL_EXTEND` | 518 400 ledgers (~30 days) | Reasonable default; active streams refresh on every interaction. |
| `INSTANCE_TTL_THRESHOLD` | 17 280 ledgers (~1 day) | Keep contract instance alive. |
| `INSTANCE_TTL_EXTEND` | 518 400 ledgers (~30 days) | Match stream TTL. |

These can be tuned per deployment. The key invariant: any stream that is
actively used (started, stopped, or settled) will never expire unexpectedly.

### Authorization model

| Entry point | `require_auth` | Notes |
|-------------|---------------|-------|
| `create_stream` | payer | Payer authorizes stream creation and initial funding. |
| `start_stream` | payer | Only payer can activate the stream. |
| `stop_stream` | payer | Only payer can deactivate the stream. |
| `settle_stream` | **none** | Intentionally permissionless — allows recipients, keepers, or bots to trigger settlement. **Note:** this also extends the stream's TTL, meaning anyone can keep a stream alive. This is acceptable because settlement only moves funds toward the recipient and extending TTL preserves the recipient's ability to claim. |
| `get_stream_info` | none | Read-only, no state changes, no TTL extension. |
| `archive_stream` (new) | payer | Payer can remove settled stream data. Stream must be inactive (is_active = false) **and** have zero balance (balance = 0) before archival — protects recipient entitlements. |
| `version` | none | Read-only. |

## 6. Future Factory Architecture (Phase 2)

When graduation triggers are hit (see §6.2), the factory architecture can be
layered on top.

### 6.1 Factory design

```
┌─────────────────────┐
│   StreamPayFactory   │
│                      │
│  - deploy_stream()   │──deploys──▶ ┌──────────────────┐
│  - registry: Map<    │              │  StreamPayChild   │
│      u32, Address>   │              │                   │
│  - wasm_hash         │              │  - payer          │
│  - next_id           │              │  - recipient      │
└─────────────────────┘              │  - rate, balance  │
        │                             │  - start/stop/    │
        │                             │    settle         │
        ├──deploys──▶                 └──────────────────┘
        │              ┌──────────────────┐
        └──deploys──▶  │  StreamPayChild   │
                       │      …            │
                       └──────────────────┘
```

**Factory contract responsibilities:**

- Store uploaded WASM hash for the child contract template.
- `deploy_stream()` — deploy child via `env.deployer()`, register in map,
  return `(stream_id, contract_address)`.
- `get_stream_address(stream_id)` — look up child contract address.
- `upgrade_child(stream_id, new_wasm_hash)` — admin-gated, upgrade a specific
  child.

**Child contract:**

- Same streaming logic as the singleton, but for a single stream.
- No `next_id`, no multi-stream storage — just its own `StreamInfo`.
- Callable directly by contract address (composable with other protocols).

### 6.2 Graduation triggers

Consider migrating to the factory pattern when any of these conditions is true:

- StreamPay needs per-stream access control beyond payer authorization.
- Streams need to be independently composable contract addresses (e.g., used as
  collateral in lending protocols).
- Per-stream upgrade control is required (e.g., regulatory or compliance
  reasons).
- Contract logic diverges for different stream types (e.g., linear vs
  cliff-vesting streams).

### 6.3 Migration strategy (Phase 1 → Phase 2)

1. Deploy the factory contract alongside the existing singleton.
2. New streams are created through the factory; existing streams remain in the
   singleton until fully settled.
3. No big-bang migration — dual-path operation until the singleton drains.

## 7. Testing Strategy

| Test | Purpose |
|------|---------|
| Existing test suite | Must continue to pass — storage layer is an implementation detail. |
| TTL extension | Verify `env.ledger().with_mut()` scenarios: stream TTL extends on access. |
| Expired stream access | Verify that accessing an expired stream returns a clear error (e.g., `"stream not found"` panic, consistent with current behavior) rather than undefined state. |
| `archive_stream()` | Verify payer can remove settled stream data. |
| Factory PoC (optional) | Minimal branch validating `env.deployer()` flow for future reference. |

**Coverage target:** 95 %+ on touched contract code, per project guidelines.

## 8. Security Considerations

- **TTL expiry:** A stream expiring while active could make unsettled balance
  inaccessible. Note: the current contract performs balance accounting only (no
  token transfers yet). Once token transfers are added, this becomes a funds-at-
  risk concern. Mitigation: all mutating operations extend TTL; `settle_stream`
  is permissionless so recipients/keepers can always trigger it; integrators
  should monitor TTL and extend proactively.
- **Archive authorization:** Only the payer (or an admin) should be able to
  archive a stream. The recipient must be able to settle before archival.
- **Storage exhaustion:** Persistent storage is not free — ensure documentation
  warns deployers about ongoing TTL renewal costs.
- **Upgrade atomicity:** A contract upgrade applies to all streams
  simultaneously. Test upgrade scenarios to ensure in-flight streams are not
  corrupted by struct layout changes.

## 9. Scope: Phase 1 Excludes

The following are explicitly **not** in scope for the persistent storage
migration (Phase 1):

- **Token transfers** — the contract remains accounting-only; token integration
  is a separate workstream.
- **Event emission** — events for stream lifecycle are desirable but deferred to
  a follow-up issue.
- **Factory deployment** — Phase 2; see §6.

## 10. Open Questions

1. Should `archive_stream()` require the stream to be fully settled, or allow
   the payer to force-close with a final settlement?
2. Should the TTL defaults (30-day extend, 1-day threshold) be configurable per
   deployment, or hardcoded as constants?
3. Should the contract emit events on stream creation, settlement, and archival
   for indexer consumption? (Deferred from Phase 1 but worth deciding early.)
