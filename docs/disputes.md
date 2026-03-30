# Dispute Resolution & Snapshot Guidance

> **Status:** Draft · **Issue:** #48 · **Author:** *(contributor name)*
> **Scope:** Documentation only — no on-chain change required unless a follow-up is approved.

## 1. Problem Statement

StreamPay streams are continuous: a payer commits a rate and balance, and the
recipient earns value every second the stream is active. Disputes arise when
either party contests the *amount earned* at a specific point in time — for
example:

- Recipient claims more was earned than the payer acknowledges.
- Payer claims the stream was stopped before a settlement was triggered.
- A third party (e.g., an auditor or lending protocol) needs to verify the
  stream's state at a past ledger.

The core challenge is that **Soroban persistent storage is not a history log**.
`get_stream_info()` returns the *current* state of a stream. Once `settle_stream()`
updates `balance` and `start_time`, the previous values are gone from on-chain
storage. There is no built-in rewind.

This document describes:

1. What on-chain data is available and what its limits are.
2. How an off-chain indexer bridges the gap.
3. How to reconstruct a historical snapshot for dispute resolution.
4. The recommended dispute workflow.
5. Security considerations and edge cases.

---

## 2. Soroban Limitations

Understanding what Soroban *cannot* do is the foundation of the dispute strategy.

### 2.1 No Historical State Queries

Soroban contracts run inside the Stellar ledger. The ledger stores only the
*current* value of each storage entry. There is no equivalent of Ethereum's
`eth_getStorageAt(block)` — you cannot query what `StreamInfo.balance` was at
ledger N if the current ledger is N+1000.

### 2.2 No Native Event Log (yet)

The Soroban SDK provides `env.events().publish()` for emitting contract events.
StreamPay v0.1.0 does **not** yet emit events (deferred from Phase 1 per
`docs/factory-pattern.md` section 9). Until events are added, the only on-chain
evidence of a state transition is the transaction itself (visible in Horizon /
RPC history).

### 2.3 TTL Expiry Destroys State

Persistent storage entries expire when their TTL reaches zero. An archived or
expired stream leaves no on-chain trace. Any dispute process must complete
before the relevant stream entry expires — or rely on off-chain snapshots taken
before expiry.

### 2.4 What Is Available On-Chain

| Source | What it provides | Limitation |
|--------|-----------------|------------|
| `get_stream_info(stream_id)` | Current `StreamInfo` struct | Current state only; no history |
| Horizon transaction history | Sequence of invocations with timestamps | Arguments visible; return values not always indexed |
| Stellar RPC `getLedgerEntries` | Current ledger entry value | No historical queries |
| Stellar RPC `getEvents` | Contract events (once emitted) | StreamPay v0.1.0 emits none yet |
| Stellar RPC `getTransactions` | Transaction metadata and result XDR | Requires XDR parsing; no semantic indexing |

---

## 3. The Indexer's Role

Because on-chain history is limited, StreamPay relies on an **off-chain indexer**
to maintain a queryable, time-ordered record of every stream state transition.

### 3.1 What the Indexer Tracks

The indexer listens to Stellar's event/transaction stream (via Horizon or the
Stellar RPC `getTransactions` / `getEvents` endpoints) and records:

| Event | Data captured | Trigger |
|-------|--------------|---------|
| `create_stream` | `stream_id`, `payer`, `recipient`, `rate_per_second`, `initial_balance`, `ledger_sequence`, `timestamp` | Transaction invocation |
| `start_stream` | `stream_id`, `start_time` (ledger timestamp), `ledger_sequence` | Transaction invocation |
| `stop_stream` | `stream_id`, `end_time`, `ledger_sequence` | Transaction invocation |
| `settle_stream` | `stream_id`, `amount_settled`, `balance_after`, `new_start_time`, `ledger_sequence` | Transaction invocation + return value |
| `archive_stream` | `stream_id`, `ledger_sequence` | Transaction invocation |

Each record is stored with its `ledger_sequence` as the primary ordering key,
making point-in-time reconstruction deterministic.

### 3.2 Snapshot Construction

A **snapshot** is the reconstructed `StreamInfo` at a given `ledger_sequence L`.

**Algorithm:**

```
snapshot(stream_id, L):
  events = indexer.get_events(stream_id, ledger <= L), ordered by ledger asc

  state = initial StreamInfo from create_stream event

  for each event in events:
    apply event to state   // see section 3.3

  if state.is_active:
    // compute accrued-but-unsettled amount up to L
    ledger_timestamp_at_L = indexer.get_timestamp(L)
    elapsed = ledger_timestamp_at_L - state.start_time
    accrued = min(elapsed * state.rate_per_second, state.balance)
    state.balance -= accrued   // virtual settlement

  return state
```

This gives the *effective* stream state at ledger L, including value that had
accrued but not yet been settled on-chain.

### 3.3 Event Application Rules

| Event | State mutation |
|-------|---------------|
| `create_stream` | Initialize `StreamInfo` with `is_active = false`, `balance = initial_balance`, `start_time = 0`, `end_time = 0` |
| `start_stream` | Set `is_active = true`, `start_time = event.timestamp` |
| `stop_stream` | Set `is_active = false`, `end_time = event.timestamp` |
| `settle_stream` | Set `balance = balance_after`, `start_time = event.new_start_time` |
| `archive_stream` | Mark stream as archived; further queries return "not found" |

### 3.4 Indexer Alignment with On-Chain State

The indexer must stay aligned with the canonical ledger. Key invariants:

1. **Ledger sequence is the source of truth.** Timestamps from `env.ledger().timestamp()` are derived from ledger close time — use `ledger_sequence` as the primary key, not wall-clock time.
2. **Idempotent ingestion.** Re-processing a ledger must produce the same state. Use `ledger_sequence` as a deduplication key.
3. **No gaps.** The indexer must process every ledger in order. A gap means a missed `settle_stream` call, which would corrupt the reconstructed balance.
4. **Reorg handling.** Stellar does not have reorgs in the Ethereum sense, but the indexer should validate that each ingested ledger's `previous_ledger_hash` matches the stored previous ledger to detect any unexpected forks.

---

## 4. Dispute Workflow

### 4.1 Parties

| Role | Description |
|------|-------------|
| Payer | Created the stream; authorized start/stop. |
| Recipient | Earns from the stream; can trigger `settle_stream`. |
| Arbitrator | Optional trusted third party that adjudicates disputes. |
| Indexer | Off-chain service maintaining the event log and snapshot API. |

### 4.2 Step-by-Step Process

```
Step 1 — Dispute Raised
  Either party identifies a contested ledger range [L_start, L_end].
  They record the stream_id and the specific claim
  (e.g., "balance at L=5000000 should be X, not Y").

Step 2 — Snapshot Request
  Either party queries the indexer:
    GET /snapshot?stream_id=42&ledger=5000000
  The indexer returns the reconstructed StreamInfo at that ledger.

Step 3 — Evidence Gathering
  Both parties independently reconstruct the snapshot using the
  public event log (see section 3.2). The reconstruction is deterministic —
  given the same event log, both parties must arrive at the same state.

Step 4 — On-Chain Verification (where possible)
  If the stream is still live, call get_stream_info() and compare
  against the snapshot projected forward to the current ledger.
  Discrepancies indicate either an indexer gap or an unrecorded transaction.

Step 5 — Arbitration
  If parties cannot agree, submit the event log and snapshots to the
  arbitrator. The arbitrator verifies:
    a. Each event hash matches a real Stellar transaction hash.
    b. The snapshot algorithm was applied correctly.
    c. No events are missing (ledger sequence is contiguous).

Step 6 — Resolution
  Arbitrator issues a ruling. If an on-chain action is needed
  (e.g., force-settle or adjust balance), a follow-up contract
  change is required (out of scope for this document).
```

### 4.3 Dispute Evidence Checklist

A well-formed dispute submission includes:

- [ ] `stream_id`
- [ ] Contested ledger range `[L_start, L_end]`
- [ ] Indexer snapshot at `L_start` and `L_end`
- [ ] List of all transaction hashes for events in the range
- [ ] Stellar transaction XDR for each event (verifiable against Horizon)
- [ ] Claimed vs. actual balance at the contested ledger
- [ ] Any `settle_stream` return values from transaction result XDR

---

## 5. Event Emission (Recommended Follow-Up)

StreamPay v0.1.0 does not emit Soroban events. Adding events is the single
highest-leverage improvement for dispute resolution and indexer reliability.

### 5.1 Proposed Events

```rust
// In src/lib.rs — proposed additions (not yet implemented)

// Emitted by create_stream
env.events().publish(
    (Symbol::new(&env, "stream_created"), stream_id),
    (payer.clone(), recipient.clone(), rate_per_second, initial_balance),
);

// Emitted by start_stream
env.events().publish(
    (Symbol::new(&env, "stream_started"), stream_id),
    env.ledger().timestamp(),
);

// Emitted by stop_stream
env.events().publish(
    (Symbol::new(&env, "stream_stopped"), stream_id),
    env.ledger().timestamp(),
);

// Emitted by settle_stream
env.events().publish(
    (Symbol::new(&env, "stream_settled"), stream_id),
    (amount, info.balance, env.ledger().timestamp()),
);

// Emitted by archive_stream
env.events().publish(
    (Symbol::new(&env, "stream_archived"), stream_id),
    (),
);
```

### 5.2 Why Events Matter for Disputes

Without events, the indexer must parse raw transaction invocation arguments and
result XDR — fragile and error-prone. With events:

- The indexer subscribes to `getEvents` filtered by contract address and topic.
- Each event is cryptographically tied to a ledger and transaction hash.
- Dispute evidence is a list of event hashes, each independently verifiable
  against the Stellar network.

**Recommendation:** Implement event emission as a follow-up to this document
(separate issue). No on-chain state change is required — events are fire-and-
forget and do not affect contract logic or storage.

---

## 6. Edge Cases

### 6.1 Stream Expired Before Dispute

If a stream's persistent storage entry has expired (TTL reached zero), the
on-chain state is gone. The indexer snapshot is the *only* source of truth.

Mitigation:
- Indexer must snapshot stream state before TTL expiry.
- Integrators should monitor TTL and call `settle_stream` (permissionless) to
  extend TTL on active streams.
- `archive_stream` should only be called after all disputes are resolved.

### 6.2 Indexer Gap (Missed Ledger)

If the indexer missed a `settle_stream` call, its reconstructed balance will be
higher than the actual on-chain balance. This is detectable by comparing the
indexer snapshot against `get_stream_info()` on a live stream.

Mitigation:
- Indexer must process ledgers in strict sequence with gap detection.
- On gap detection, backfill from Horizon before serving dispute snapshots.
- Expose an indexer health endpoint that reports the last processed ledger.

### 6.3 Concurrent Settlements

`settle_stream` is permissionless — anyone can call it. Multiple parties may
race to settle. Each settlement is a separate transaction with a unique ledger
sequence. The indexer must record all of them in order; the snapshot algorithm
applies them sequentially.

### 6.4 Clock Skew Between Ledger Timestamp and Wall Clock

`env.ledger().timestamp()` is the Stellar network's consensus timestamp, not
the caller's wall clock. Disputes about "what time was it" should always
reference the ledger timestamp, not any off-chain clock.

### 6.5 Archived Stream Dispute

Once `archive_stream` removes the stream from persistent storage, `get_stream_info()`
panics with "stream not found". The indexer snapshot is the sole evidence.
Parties should ensure all disputes are resolved before archiving.

---

## 7. Security Considerations

| # | Concern | Mitigation |
|---|---------|------------|
| S1 | **Indexer tampering** — a malicious indexer serves a false snapshot | Each event in the snapshot must be verifiable against a real Stellar transaction hash. Parties should independently verify event hashes via Horizon. |
| S2 | **Snapshot replay** — an old snapshot is presented as current | Snapshots must include the `ledger_sequence` they were computed at. Verifiers check that the ledger sequence is consistent with the event log. |
| S3 | **Missing settle events** — indexer omits a settlement to inflate recipient's apparent balance | Gap detection (section 6.2) and independent reconstruction from raw Horizon data catches this. |
| S4 | **TTL expiry during dispute** — stream expires mid-dispute, destroying on-chain evidence | Permissionless `settle_stream` extends TTL. Dispute parties should call it proactively. Indexer snapshots before expiry are the fallback. |
| S5 | **Fabricated transaction hashes** — a party submits fake evidence | All transaction hashes are verifiable against the public Stellar ledger via Horizon or RPC. |
| S6 | **Rate manipulation** — payer claims `rate_per_second` was different | `rate_per_second` is set at `create_stream` and immutable. The `create_stream` transaction hash is the canonical proof. |

---

## 8. Relationship to Other Design Documents

| Document | Relationship |
|----------|-------------|
| `docs/factory-pattern.md` | Defines the persistent storage model that streams use. Dispute snapshots depend on the per-stream ledger entry structure defined there. |
| `docs/collateral.md` | Collateral and lockup escrow introduce additional fields (`lockup_amount`, `release_policy`) that must be included in dispute snapshots when implemented. |

---

## 9. Open Questions

- [ ] Should StreamPay implement an on-chain dispute entry point (e.g., `raise_dispute(stream_id, evidence_hash)`) or keep dispute resolution fully off-chain?
- [ ] What is the minimum indexer retention period for event history? (Regulatory requirements may mandate multi-year retention.)
- [ ] Should the indexer expose a public API, or is it a private backend service per deployment?
- [ ] When event emission is added (section 5), should events include the full `StreamInfo` struct or only the delta?
- [ ] How should disputes interact with `archive_stream`? Should archival be blocked while a dispute flag is active?

---

## 10. References

- [StreamPay Factory Pattern Design](factory-pattern.md) — storage architecture and TTL strategy.
- [StreamPay Collateral & Lockup Design](collateral.md) — escrow fields relevant to future dispute snapshots.
- [Soroban Events Docs](https://soroban.stellar.org/docs/learn/events) — `env.events().publish()` API.
- [Stellar Horizon API](https://developers.stellar.org/api/horizon) — transaction and event history.
- [Stellar RPC `getEvents`](https://developers.stellar.org/docs/data/rpc/api-reference/methods/getEvents) — contract event subscription.
- [Sablier V2 Dispute Model](https://docs.sablier.com/) — prior art for streaming payment disputes.
