# Public Contract Error Codes Architecture & Taxonomy

This specification defines the public contract error codes, taxonomy, decoding semantics, and stability guarantees for StreamPay Soroban smart contracts.

---

## 1. Motivation

Clients and off-chain indexers require reliable, stable, and deterministic error classification across contract upgrades. By establishing a formalized `#[contracterror]` enum with disjoint category ranges, safe fallbacks, and golden vector guarantees, clients can build robust error handling pipelines that do not break when contracts evolve.

---

## 2. Error Taxonomy & Disjoint Numeric Ranges

Each error code is assigned a unique `u32` value allocated within a reserved numeric range according to its domain:

| Range | Domain | Description |
|-------|--------|-------------|
| `100..=199` | **Validation** | Input validation, parameter bounds, timestamp constraints, batch size limits. |
| `200..=299` | **Lifecycle** | Stream state transitions, active/inactive checks, pause/resume constraints. |
| `300..=399` | **Storage & Archiving** | Storage lookup failures, stream lifecycle archiving constraints, storage quotas. |
| `400..=499` | **Authorization** | Missing authentication, unauthorized callers, role or identity mismatch. |
| `500..=599` | **Settlement & Math** | Arithmetic overflow, balance insufficiency, zero settlement handling. |
| `900..=999` | **System & Fallback** | Unrecognized error codes, internal system faults, fallback handlers. |

---

## 3. Comprehensive Error Code Registry

| Numeric Code | Variant Identifier | Category | Canonical Message | Recoverability | Severity |
|:------------:|:-------------------|:---------|:------------------|:---------------|:---------|
| **101** | `RateAndBalanceMustBePositive` | Validation | `"rate and balance must be positive"` | `Retryable` | `Medium` |
| **102** | `EndTimeMustBeInFuture` | Validation | `"end_time must be in the future"` | `Retryable` | `Medium` |
| **103** | `BatchTooLarge` | Validation | `"batch too large"` | `Retryable` | `Medium` |
| **104** | `InvalidAmount` | Validation | `"invalid amount"` | `Retryable` | `Medium` |
| **105** | `ZeroAmount` | Validation | `"amount cannot be zero"` | `Retryable` | `Low` |
| **106** | `InvalidTimeRange` | Validation | `"invalid time range"` | `Retryable` | `Medium` |
| **201** | `StreamAlreadyActive` | Lifecycle | `"stream already active"` | `Terminal` | `Medium` |
| **202** | `StreamNotActive` | Lifecycle | `"stream not active"` | `Terminal` | `Medium` |
| **203** | `CannotCancelInactiveStream` | Lifecycle | `"cannot cancel inactive stream"` | `Terminal` | `Medium` |
| **204** | `CannotPauseInactiveStream` | Lifecycle | `"cannot pause inactive stream"` | `Retryable` | `Medium` |
| **205** | `StreamAlreadyPaused` | Lifecycle | `"stream already paused"` | `Retryable` | `Medium` |
| **206** | `CannotResumeInactiveStream` | Lifecycle | `"cannot resume inactive stream"` | `Retryable` | `Medium` |
| **207** | `StreamIsNotPaused` | Lifecycle | `"stream is not paused"` | `Retryable` | `Medium` |
| **208** | `StreamAlreadyTerminated` | Lifecycle | `"stream already terminated"` | `Terminal` | `Medium` |
| **301** | `StreamNotFound` | Storage | `"stream not found"` | `Retryable` | `Medium` |
| **302** | `CannotArchiveActiveStream` | Storage | `"cannot archive active stream"` | `Terminal` | `Medium` |
| **303** | `CannotArchiveStreamWithUnsettledBalance` | Storage | `"cannot archive stream with unsettled balance"` | `Retryable` | `Medium` |
| **304** | `StorageKeyNotFound` | Storage | `"storage key not found"` | `RequiresAdmin` | `Critical` |
| **305** | `StorageQuotaExceeded` | Storage | `"storage quota exceeded"` | `RequiresAdmin` | `Critical` |
| **401** | `Unauthorized` | Authorization | `"unauthorized caller"` | `Terminal` | `High` |
| **402** | `NotPayer` | Authorization | `"caller is not payer"` | `Terminal` | `High` |
| **403** | `NotRecipient` | Authorization | `"caller is not recipient"` | `Terminal` | `High` |
| **501** | `ArithmeticOverflow` | Settlement | `"arithmetic overflow"` | `Terminal` | `Critical` |
| **502** | `InsufficientBalance` | Settlement | `"insufficient balance"` | `Terminal` | `Medium` |
| **503** | `ZeroSettlement` | Settlement | `"zero settlement"` | `Terminal` | `Low` |
| **999** | `UnknownError` | System | `"unknown error"` | `Terminal` | `Critical` |

---

## 4. Decoding & Compatibility Invariants

1. **Uniqueness**: Every concrete `Error` variant maps to a distinct `u32` code.
2. **Safe Fallback**: `Error::decode(code)` returns `Error::UnknownError` for any unrecognized or future code, preventing client panics or undefined behavior.
3. **Legacy Sequential Backward Compatibility**: Legacy index codes `1..=13` are mapped safely to their corresponding semantic error variants during decoding.
4. **Canonical Messages**: `Error::message()` preserves exact compatibility with existing string messages and contract panic vectors.

---

## 5. Golden Vectors

Golden vector tests in `tests/error_stabilization_tests.rs` verify all public entrypoints, ensuring that:
- `create_stream` failures map to `Error::RateAndBalanceMustBePositive` (`101`).
- `start_stream` failures map to `Error::EndTimeMustBeInFuture` (`102`) and `Error::StreamAlreadyActive` (`201`).
- `stop_stream` failures map to `Error::StreamNotActive` (`202`).
- `batch_settle` oversized input maps to `Error::BatchTooLarge` (`103`).
- `cancel_stream` failures map to `Error::CannotCancelInactiveStream` (`203`).
- `pause_stream` failures map to `Error::CannotPauseInactiveStream` (`204`) and `Error::StreamAlreadyPaused` (`205`).
- `resume_stream` failures map to `Error::CannotResumeInactiveStream` (`206`) and `Error::StreamIsNotPaused` (`207`).
- `archive_stream` failures map to `Error::CannotArchiveActiveStream` (`302`) and `Error::CannotArchiveStreamWithUnsettledBalance` (`303`).
- `get_stream_info` missing ID maps to `Error::StreamNotFound` (`301`).
