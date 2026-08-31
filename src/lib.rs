//! # streampay-contracts
//!
//! Soroban smart contracts for StreamPay — continuous token streaming on Stellar.
//!
//! ---
//!
//! ## Ledger Timestamp Assumptions
//!
//! All time-based logic in this crate relies on **`Env::ledger().timestamp()`**.
//!
//! Key properties:
//!
//! - **Whole seconds only.** `u64` Unix timestamp, no sub-second resolution.
//!   Accrual is always truncated to complete seconds.
//!
//! - **~5–6 s ledger cadence.** Timestamp does not advance between ledger closes.
//!   All transactions in the same ledger share the *same* timestamp.
//!
//! - **Validator-set, not caller-set.** Agreed by SCP quorum. No transaction
//!   sender can influence it. Timestamp-manipulation attacks are not possible.
//!
//! - **Monotonic.** Protocol rules guarantee `new >= previous`.
//!
//! - **Dust tail.** The fractional-second gap between the last ledger close
//!   before `end_time` and `end_time` itself is never claimable by the recipient.
//!   It is reclaimed by the sender on stream close.
//!
//! ### Off-chain UX recommendation
//!
//! Derive elapsed time from the **last confirmed ledger close time** (Horizon/RPC),
//! not the device wall clock. Wall-clock interpolation overstates claimable balance.
//!
//! See [`docs/timestamp-accrual.md`](../docs/timestamp-accrual.md) for full detail.
//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! batch_settle, max_batch_settle_size, archive_stream, get_stream_info, version.
//!
//! # Integer Safety — i128 Saturation Semantics
//!
//! All accrual arithmetic uses **saturating** operations to guarantee no silent
//! wrap-around, regardless of how extreme `rate_per_second` or `elapsed` become.
//!
//! ## Why saturation instead of checked/wrapping?
//! * Wrapping would silently produce a wrong (possibly negative) amount, which
//!   could drain the payer's balance incorrectly or credit the recipient nothing.
//! * Panicking on overflow would make the contract un-settleable for legitimate
//!   high-value streams.
//! * Saturating clamps the intermediate product at `i128::MAX` and then the
//!   `.min(balance)` guard ensures the final settled amount never exceeds the
//!   deposited balance — the worst case is the recipient receives exactly what
//!   was deposited, which is the correct economic outcome.
//!
//! ## Stellar / Soroban timestamp limits
//! Soroban ledger timestamps are `u64` Unix seconds.  The practical ceiling on
//! Stellar today is well under 2^32 seconds (~136 years from epoch), but the
//! contract casts `elapsed: u64` to `i128` before multiplying, so even a
//! pathological elapsed value of `u64::MAX` (~1.8 × 10^19 s) combined with
//! `i128::MAX` rate saturates to `i128::MAX` rather than wrapping.
//!
//! ## Invariants upheld by `settle_stream`
//! 1. `amount >= 0` — saturation of non-negative operands stays non-negative.
//! 2. `amount <= balance` — enforced by `.min(info.balance)`.
//! 3. `new_balance >= 0` — `balance.saturating_sub(amount)` where `amount <=
//!    balance` always yields a non-negative result.

pub mod error;
pub use error::{Error, ErrorCategory, ErrorSeverity, Recoverability};

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.2.0 → 2_000
const VERSION: u32 = 2_000;

/// TTL threshold: extend when remaining TTL drops below ~1 day (17_280 ledgers at ~5s each).
const STREAM_TTL_THRESHOLD: u32 = 17_280;
/// TTL extend-to: refresh to ~30 days (518_400 ledgers).
const STREAM_TTL_EXTEND: u32 = 518_400;
/// Instance storage TTL threshold (~1 day).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
/// Instance storage TTL extend-to (~30 days).
const INSTANCE_TTL_EXTEND: u32 = 518_400;
/// Hard cap for batch settlement to keep Soroban resource usage predictable.
const MAX_BATCH_SETTLE_SIZE: u32 = 25;

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub payer: Address,
    pub recipient: Address,
    pub rate_per_second: i128,
    pub balance: i128,
    pub start_time: u64,
    pub end_time: u64, // Max duration: stream auto-deactivates at this time
    pub is_active: bool,
    pub paused_at: u64, // 0 if not paused; timestamp of pause if paused
}

/// Event data emitted when a new stream is created.
///
/// Topics: `["stream_created", stream_id]`
/// Data:   `StreamCreatedEvent { payer, recipient, rate_per_second, initial_balance }`
///
/// Indexers can filter on topic[0] == "stream_created" and topic[1] == stream_id.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamCreatedEvent {
    pub payer: Address,
    pub recipient: Address,
    pub rate_per_second: i128,
    pub initial_balance: i128,
}

#[contract]
pub struct StreamPayContract;

#[contractimpl]
impl StreamPayContract {
    /// Create a new payment stream (payer, recipient, rate per second, optional end_time).
    /// If end_time is 0, stream has no time limit (must be stopped manually).
    /// If end_time > 0, must satisfy end_time > implicit start time (enforced at start_stream).
    pub fn create_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        rate_per_second: i128,
        initial_balance: i128,
        end_time: u64, // 0 = no limit; otherwise must be > start_time (validated at start)
    ) -> u32 {
        payer.require_auth();
        if rate_per_second <= 0 || initial_balance <= 0 {
            panic!("{}", Error::RateAndBalanceMustBePositive.message());
        }
        let stream_id = get_next_stream_id(&env);
        let info = StreamInfo {
            payer: payer.clone(),
            recipient,
            rate_per_second,
            balance: initial_balance,
            start_time: 0,
            end_time,
            is_active: false,
            paused_at: 0,
        };
        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id + 1);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        emit_stream_created(
            &env,
            stream_id,
            &payer,
            &info.recipient,
            rate_per_second,
            initial_balance,
        );
        stream_id
    }

    /// Start an existing stream.
    /// If end_time was set at creation, validates that end_time > current timestamp.
    pub fn start_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if info.is_active {
            panic!("{}", Error::StreamAlreadyActive.message());
        }
        let now = env.ledger().timestamp();

        // Validate end_time constraint if set
        if info.end_time > 0 && info.end_time <= now {
            panic!("{}", Error::EndTimeMustBeInFuture.message());
        }

        info.is_active = true;
        info.start_time = now;
        info.paused_at = 0; // Clear paused state
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Stop an active stream, atomically settling its exact earned amount.
    pub fn stop_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if !info.is_active {
            panic!("{}", Error::StreamNotActive.message());
        }

        terminalize_stream(&mut info, env.ledger().timestamp());
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Settle stream: compute streamed amount since start and deduct from balance.
    ///
    /// # Saturation semantics
    ///
    /// The accrual formula is:
    /// ```text
    /// amount = (elapsed as i128).saturating_mul(rate_per_second).min(balance)
    /// ```
    ///
    /// Both operands are non-negative (`elapsed` is a `u64` difference cast to
    /// `i128`; `rate_per_second` is validated `> 0` at creation time), so the
    /// saturating multiply clamps at `i128::MAX` rather than wrapping.  The
    /// subsequent `.min(balance)` ensures the settled amount never exceeds the
    /// deposited balance, preserving the invariant `new_balance >= 0`.
    ///
    /// This means:
    /// * An astronomically large `rate_per_second` (e.g. `i128::MAX`) will
    ///   settle at most the full remaining balance — no funds are conjured.
    /// * An astronomically long `elapsed` window (e.g. `u64::MAX` seconds,
    ///   far beyond any real Stellar ledger timestamp) is handled identically.
    /// * There is **no silent wrap** at any point in the computation.
    pub fn settle_stream(env: Env, stream_id: u32) -> i128 {
        let amount = settle_stream_amount(&env, stream_id);
        if amount.is_none() {
            return 0;
        }
        extend_instance_ttl(&env);

        amount.unwrap()
    }

    /// Settle multiple streams in a single invocation.
    ///
    /// Failure behavior is all-or-nothing: if any item panics, the entire call
    /// reverts and no settlement state is committed. Callers should chunk larger
    /// workloads into batches of `MAX_BATCH_SETTLE_SIZE` or fewer ids.
    pub fn batch_settle(env: Env, stream_ids: Vec<u32>) -> Vec<i128> {
        if stream_ids.len() > MAX_BATCH_SETTLE_SIZE {
            panic!("{}", Error::BatchTooLarge.message());
        }

        let mut settled_amounts = Vec::new(&env);
        let mut touched_active_stream = false;

        for stream_id in stream_ids.iter() {
            match settle_stream_amount(&env, stream_id) {
                Some(amount) => {
                    touched_active_stream = true;
                    settled_amounts.push_back(amount);
                }
                None => settled_amounts.push_back(0),
            }
        }

        if touched_active_stream {
            extend_instance_ttl(&env);
        }

        settled_amounts
    }

    /// Returns the configured maximum number of stream ids allowed in one
    /// `batch_settle` invocation.
    pub fn max_batch_settle_size(_env: Env) -> u32 {
        MAX_BATCH_SETTLE_SIZE
    }

    /// Cancel a stream early (payer-only).
    /// Immediately settles all accrued amounts to the recipient.
    /// Remaining unaccrued balance is retained by the payer.
    /// Atomic operation: prevents race conditions with settle.
    ///
    /// For a bounded stream the natural configured `end_time` takes
    /// precedence over a late cancellation: accrual is capped at
    /// `min(now, end_time)` and the stored terminal boundary stays the
    /// configured end instead of moving forward to the cancellation time.
    pub fn cancel_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();

        if !info.is_active {
            panic!("{}", Error::CannotCancelInactiveStream.message());
        }

        terminalize_stream(&mut info, env.ledger().timestamp());

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Pause an active stream (payer-only).
    /// Stops accrual without full termination; preserves balance and schedule.
    /// Can be resumed with resume_stream.
    /// Distinct from stop_stream (which is final).
    pub fn pause_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();

        if !info.is_active {
            panic!("{}", Error::CannotPauseInactiveStream.message());
        }
        if info.paused_at > 0 {
            panic!("{}", Error::StreamAlreadyPaused.message());
        }

        let now = env.ledger().timestamp();
        let boundary = accrual_bound(now, info.end_time, info.paused_at);

        // Settle accrued amount up to the end-capped pause point.
        settle_info_to_boundary(&mut info, boundary);

        if reached_natural_end(now, info.end_time) {
            info.is_active = false;
            info.paused_at = 0;
        } else {
            // Keep is_active true while paused so the stream can be resumed.
            info.paused_at = boundary;
        }

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Resume a paused stream (payer-only).
    /// Restarts accrual from the pause point.
    /// Is_active remains true; paused_at is cleared.
    pub fn resume_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();

        if !info.is_active {
            panic!("{}", Error::CannotResumeInactiveStream.message());
        }
        if info.paused_at == 0 {
            panic!("{}", Error::StreamIsNotPaused.message());
        }

        let now = env.ledger().timestamp();

        if reached_natural_end(now, info.end_time) {
            // A stream whose configured schedule ended while paused is terminal.
            // Resuming must never create a post-end accrual window.
            info.is_active = false;
            info.paused_at = 0;
            set_stream(&env, stream_id, &info);
            extend_stream_ttl(&env, stream_id);
            extend_instance_ttl(&env);
            return;
        }

        // Resume: reset start_time to account for paused duration and clear paused state
        info.start_time = now;
        info.paused_at = 0;

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Get stream info (read-only).
    pub fn get_stream_info(env: Env, stream_id: u32) -> StreamInfo {
        get_stream(&env, stream_id)
    }

    /// Returns the contract version as a u32 (see VERSION encoding).
    pub fn version(_env: Env) -> u32 {
        VERSION
    }

    /// Archive (remove) a fully-settled, inactive stream. Payer-only.
    /// Stream must be inactive and have zero balance to protect recipient entitlements.
    pub fn archive_stream(env: Env, stream_id: u32) {
        let info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if info.is_active {
            panic!("{}", Error::CannotArchiveActiveStream.message());
        }
        if info.balance != 0 {
            panic!(
                "{}",
                Error::CannotArchiveStreamWithUnsettledBalance.message()
            );
        }
        let key = stream_key(&env, stream_id);
        env.storage().persistent().remove(&key);
        extend_instance_ttl(&env);
    }
}

/// Emit a `stream_created` contract event.
///
/// Topics (indexer-friendly, low-cost):
///   - `"stream_created"` — event discriminator
///   - `stream_id`        — numeric stream identifier
///
/// Data payload: [`StreamCreatedEvent`] containing payer, recipient,
/// rate_per_second, and initial_balance.
fn emit_stream_created(
    env: &Env,
    stream_id: u32,
    payer: &Address,
    recipient: &Address,
    rate_per_second: i128,
    initial_balance: i128,
) {
    let topics = (Symbol::new(env, "stream_created"), stream_id);
    let data = StreamCreatedEvent {
        payer: payer.clone(),
        recipient: recipient.clone(),
        rate_per_second,
        initial_balance,
    };
    env.events().publish(topics, data);
}

fn stream_key(env: &Env, stream_id: u32) -> (Symbol, u32) {
    (Symbol::new(env, "stream"), stream_id)
}

fn get_stream(env: &Env, stream_id: u32) -> StreamInfo {
    let key = stream_key(env, stream_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("{}", Error::StreamNotFound.message()))
}

fn set_stream(env: &Env, stream_id: u32, info: &StreamInfo) {
    let key = stream_key(env, stream_id);
    env.storage().persistent().set(&key, info);
}

fn get_next_stream_id(env: &Env) -> u32 {
    let key = Symbol::new(env, "next_id");
    env.storage().instance().get(&key).unwrap_or(1)
}

fn set_next_stream_id(env: &Env, id: u32) {
    let key = Symbol::new(env, "next_id");
    env.storage().instance().set(&key, &id);
}

fn settle_stream_amount(env: &Env, stream_id: u32) -> Option<i128> {
    let mut info = get_stream(env, stream_id);
    if !info.is_active {
        return None;
    }

    let now = env.ledger().timestamp();
    let settlement_time = accrual_bound(now, info.end_time, info.paused_at);
    let amount = settle_info_to_boundary(&mut info, settlement_time);
    if reached_natural_end(now, info.end_time) {
        // A bounded stream that reached its configured end is terminal:
        // further settlements must not move value.
        info.is_active = false;
        info.paused_at = 0;
    }
    set_stream(env, stream_id, &info);
    extend_stream_ttl(env, stream_id);

    Some(amount)
}

/// Effective accrual boundary for a stream observed at ledger time `now`.
///
/// Unlimited streams (`end_time == 0`) settle at the current ledger
/// timestamp; bounded streams never accrue past their configured
/// `end_time`, so the boundary is `min(now, end_time)`.
fn end_bound(now: u64, end_time: u64) -> u64 {
    if end_time != 0 && now > end_time {
        end_time
    } else {
        now
    }
}

/// Effective accrual boundary, additionally capped at the pause timestamp.
fn accrual_bound(now: u64, end_time: u64, paused_at: u64) -> u64 {
    let boundary = end_bound(now, end_time);
    if paused_at == 0 {
        boundary
    } else {
        boundary.min(paused_at)
    }
}

/// Cursor from which a value-moving operation may accrue.
///
/// Before the paused-accrual fix, `pause_stream` deducted value through
/// `paused_at` without advancing `start_time`. Persisted paused streams from
/// that implementation therefore use `paused_at` as their already-accounted
/// cursor. Fresh states have `start_time == paused_at`, so the same rule is
/// compatible with both shapes and prevents replaying the pre-pause interval.
fn accrual_cursor(start_time: u64, paused_at: u64) -> u64 {
    if paused_at == 0 {
        start_time
    } else {
        start_time.max(paused_at)
    }
}

/// Deduct the amount earned between the effective cursor and `boundary`, then
/// persist the boundary as the new cursor so no later path can replay it.
fn settle_info_to_boundary(info: &mut StreamInfo, boundary: u64) -> i128 {
    let cursor = accrual_cursor(info.start_time, info.paused_at);
    let elapsed = boundary.saturating_sub(cursor);
    let amount = (elapsed as i128)
        .saturating_mul(info.rate_per_second)
        .min(info.balance);
    info.balance = info.balance.saturating_sub(amount);
    info.start_time = boundary;
    amount
}

/// Settle through the same pause/end-aware boundary used by permissionless
/// settlement, then make the stream terminal and clear stale pause state.
fn terminalize_stream(info: &mut StreamInfo, now: u64) {
    let terminal_boundary = end_bound(now, info.end_time);
    let boundary = accrual_bound(now, info.end_time, info.paused_at);
    settle_info_to_boundary(info, boundary);
    info.is_active = false;
    info.end_time = terminal_boundary;
    info.paused_at = 0;
}

/// Whether a bounded stream has reached its configured natural end by
/// ledger time `now`.
fn reached_natural_end(now: u64, end_time: u64) -> bool {
    end_time != 0 && now >= end_time
}

fn extend_stream_ttl(env: &Env, stream_id: u32) {
    let key = stream_key(env, stream_id);
    env.storage()
        .persistent()
        .extend_ttl(&key, STREAM_TTL_THRESHOLD, STREAM_TTL_EXTEND);
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

#[cfg(test)]
mod test {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;

    use super::*;

    /// Advances the test ledger timestamp by `seconds` so accrual scenarios
    /// can assert deterministic elapsed-time behavior.
    fn advance_ledger_time(env: &Env, seconds: u64) {
        env.ledger().with_mut(|li| {
            li.timestamp += seconds;
        });
    }

    #[test]
    fn test_create_stream_valid() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);
        assert_eq!(stream_id, 1);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.payer, payer);
        assert_eq!(info.recipient, recipient);
        assert_eq!(info.rate_per_second, 100);
        assert_eq!(info.balance, 10_000);
        assert!(!info.is_active);
        assert_eq!(info.paused_at, 0);
    }

    /// Verify that `create_stream` emits exactly one `stream_created` event
    /// with the correct topics and data payload.
    #[test]
    fn test_create_stream_emits_event() {
        use soroban_sdk::testutils::Events as _;

        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);

        let events = env.events().all();
        // Exactly one event should have been emitted
        assert_eq!(events.len(), 1);

        let (emitting_contract, topics, data) = events.get(0).unwrap();
        assert_eq!(emitting_contract, contract_id);

        // topic[0] == "stream_created", topic[1] == stream_id
        let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
        let topic1: u32 = soroban_sdk::FromVal::from_val(&env, &topics.get(1).unwrap());
        assert_eq!(topic0, Symbol::new(&env, "stream_created"));
        assert_eq!(topic1, stream_id);

        // Data payload carries all four fields
        let event_data: StreamCreatedEvent = soroban_sdk::FromVal::from_val(&env, &data);
        assert_eq!(event_data.payer, payer);
        assert_eq!(event_data.recipient, recipient);
        assert_eq!(event_data.rate_per_second, 100);
        assert_eq!(event_data.initial_balance, 10_000);
    }

    /// Only `create_stream` emits an event; start/stop must not emit
    /// spurious `stream_created` events.
    #[test]
    fn test_no_spurious_stream_created_events() {
        use soroban_sdk::testutils::Events as _;

        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);

        // start / stop must not add more stream_created events
        client.start_stream(&stream_id);
        client.stop_stream(&stream_id);

        let events = env.events().all();
        assert!(events.len() <= 1);
    }

    #[test]
    fn test_start_and_stop_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &50_i128, &5_000_i128, &0_u64);
        client.start_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert!(info.is_active);
        client.stop_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert!(!info.is_active);
    }

    #[test]
    fn test_settle_returns_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 100);
    }

    #[test]
    fn test_batch_settle_empty_vec() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let stream_ids = Vec::new(&env);
        let amounts = client.batch_settle(&stream_ids);

        assert_eq!(amounts.len(), 0);
    }

    #[test]
    fn test_batch_settle_inactive_stream_returns_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);

        let mut stream_ids = Vec::new(&env);
        stream_ids.push_back(stream_id);

        let amounts = client.batch_settle(&stream_ids);

        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts.get(0).unwrap(), 0);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 1_000);
        assert!(!info.is_active);
    }

    #[test]
    fn test_batch_settle_single_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });

        let mut stream_ids = Vec::new(&env);
        stream_ids.push_back(stream_id);

        let amounts = client.batch_settle(&stream_ids);

        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts.get(0).unwrap(), 100);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 900);
        assert_eq!(info.start_time, env.ledger().timestamp());
    }

    #[test]
    fn test_batch_settle_multiple_streams() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient_a = Address::generate(&env);
        let recipient_b = Address::generate(&env);
        let first_stream_id =
            client.create_stream(&payer, &recipient_a, &10_i128, &1_000_i128, &0_u64);
        let second_stream_id =
            client.create_stream(&payer, &recipient_b, &5_i128, &1_000_i128, &0_u64);
        client.start_stream(&first_stream_id);
        client.start_stream(&second_stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });

        let mut stream_ids = Vec::new(&env);
        stream_ids.push_back(first_stream_id);
        stream_ids.push_back(second_stream_id);

        let amounts = client.batch_settle(&stream_ids);

        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts.get(0).unwrap(), 100);
        assert_eq!(amounts.get(1).unwrap(), 50);

        let first_info = client.get_stream_info(&first_stream_id);
        let second_info = client.get_stream_info(&second_stream_id);
        assert_eq!(first_info.balance, 900);
        assert_eq!(second_info.balance, 950);
    }

    #[test]
    fn test_batch_settle_missing_id_reverts_all() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });

        let original_info = client.get_stream_info(&stream_id);

        let mut stream_ids = Vec::new(&env);
        stream_ids.push_back(stream_id);
        stream_ids.push_back(999_u32);

        let result = catch_unwind(AssertUnwindSafe(|| client.batch_settle(&stream_ids)));
        assert!(result.is_err());

        let info_after = client.get_stream_info(&stream_id);
        assert_eq!(info_after.balance, original_info.balance);
        assert_eq!(info_after.start_time, original_info.start_time);
    }

    #[test]
    #[should_panic(expected = "batch too large")]
    fn test_batch_settle_too_large_panics() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let mut stream_ids = Vec::new(&env);
        for stream_id in 1..=(MAX_BATCH_SETTLE_SIZE + 1) {
            stream_ids.push_back(stream_id);
        }

        client.batch_settle(&stream_ids);
    }

    #[test]
    fn test_max_batch_settle_size_matches_constant() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        assert_eq!(client.max_batch_settle_size(), MAX_BATCH_SETTLE_SIZE);
    }

    #[test]
    fn test_version_returns_expected() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), 2_000);
    }

    #[test]
    fn test_version_matches_const() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), VERSION);
    }

    #[test]
    fn test_version_is_positive() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert!(client.version() > 0);
    }

    #[test]
    fn test_stream_uses_persistent_storage() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);

        // Verify stream is retrievable (storage works)
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 10_000);
    }

    #[test]
    fn test_create_stream_extends_ttl() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);

        // Advance ledger by a modest amount — stream should still be alive
        // because create_stream extended its TTL
        env.ledger().with_mut(|li| {
            li.sequence_number += 1_000;
            li.timestamp += 5_000;
        });

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 10_000);
    }

    #[test]
    fn test_archive_settled_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=100/s, balance=1000 → fully drained after 10s
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Advance 10 seconds so balance drains to 0
        advance_ledger_time(&env, 10);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 1_000);

        client.stop_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
        assert!(!info.is_active);

        // Now archive — stream is stopped and fully settled
        client.archive_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_archive_unsettled_stream_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);

        // Stream is inactive but has balance > 0 — should panic
        // to protect recipient's entitlement
        client.archive_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_archive_active_stream_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Should panic — stream is active
        client.archive_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_archived_stream_not_found() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Create, start, drain, stop, then archive
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id);
        client.archive_stream(&stream_id);

        // Should panic — stream was archived (removed from storage)
        client.get_stream_info(&stream_id);
    }

    // -------------------------------------------------------------------------
    // i128 saturation tests
    // -------------------------------------------------------------------------

    /// Extreme rate: i128::MAX rate_per_second with a 1-second window.
    /// The product saturates at i128::MAX, but .min(balance) clamps it to the
    /// deposited balance.  No funds are conjured; no wrap occurs.
    #[test]
    fn test_settle_extreme_rate_saturates_to_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let balance = 1_000_000_i128;
        // Use i128::MAX as rate — any elapsed > 0 would overflow without saturation
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &balance, &0_u64);
        client.start_stream(&stream_id);

        // Advance 1 second
        env.ledger().with_mut(|li| {
            li.timestamp += 1;
        });

        let amount = client.settle_stream(&stream_id);
        // Saturating mul: i128::MAX * 1 = i128::MAX, clamped to balance
        assert_eq!(
            amount, balance,
            "extreme rate must settle exactly the balance, not more"
        );

        let info = client.get_stream_info(&stream_id);
        assert_eq!(
            info.balance, 0,
            "balance must be fully drained, not negative"
        );
    }

    /// Extreme elapsed: simulate a very long window (u64::MAX seconds) with a
    /// normal rate.  The product saturates at i128::MAX, clamped to balance.
    #[test]
    fn test_settle_extreme_elapsed_saturates_to_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let balance = 500_i128;
        let stream_id = client.create_stream(&payer, &recipient, &1_000_i128, &balance, &0_u64);

        // Manually set start_time to 0 via start_stream at timestamp 0
        client.start_stream(&stream_id);

        // Jump to near u64::MAX to create a massive elapsed window
        env.ledger().with_mut(|li| {
            li.timestamp = u64::MAX;
        });

        let amount = client.settle_stream(&stream_id);
        assert_eq!(
            amount, balance,
            "extreme elapsed must settle exactly the balance"
        );

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0, "balance must reach zero, not go negative");
    }

    /// Both rate and elapsed at maximum: double-extreme case.
    /// saturating_mul(i128::MAX, i128::MAX) = i128::MAX, clamped to balance.
    #[test]
    fn test_settle_extreme_rate_and_elapsed_saturates_to_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let balance = 42_i128;
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &balance, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp = u64::MAX;
        });

        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, balance);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
    }

    /// Settled amount is always non-negative — invariant check.
    #[test]
    fn test_settle_amount_never_negative() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 100;
        });

        let amount = client.settle_stream(&stream_id);
        assert!(amount >= 0, "settled amount must never be negative");
    }

    /// Balance never goes negative after settle — invariant check.
    #[test]
    fn test_balance_never_negative_after_settle() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &999_i128, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp = u64::MAX;
        });

        client.settle_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert!(info.balance >= 0, "balance must never go negative");
    }

    /// Partial accrual: rate * elapsed < balance — only partial amount settled.
    #[test]
    fn test_settle_partial_accrual_no_saturation() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=10/s, balance=10_000, elapsed=5s → amount=50
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &10_000_i128, &0_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        let amount = client.settle_stream(&stream_id);
        assert_eq!(
            amount, 50,
            "partial accrual should be exact when no saturation"
        );

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 9_950);
    }

    /// Zero elapsed (settle immediately after start) — amount must be 0.
    #[test]
    fn test_settle_zero_elapsed_returns_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // No time advance — elapsed = 0
        let amount = client.settle_stream(&stream_id);
        assert_eq!(
            amount, 0,
            "zero elapsed must yield zero amount even with max rate"
        );
    }

    /// Multiple sequential settles with extreme rate — each settle drains
    /// remaining balance; total never exceeds initial deposit.
    #[test]
    fn test_settle_multiple_calls_total_capped_at_initial_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let initial_balance = 300_i128;
        let stream_id =
            client.create_stream(&payer, &recipient, &i128::MAX, &initial_balance, &0_u64);
        client.start_stream(&stream_id);

        let mut total_settled = 0_i128;

        for tick in [1_u64, 1, 1] {
            env.ledger().with_mut(|li| {
                li.timestamp += tick;
            });
            total_settled += client.settle_stream(&stream_id);
        }

        assert_eq!(
            total_settled, initial_balance,
            "total settled across multiple calls must equal initial balance"
        );
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
    }
}

/// Regression coverage for issue #153: cancellation and natural end-time
/// settlement must be exact, deterministic, once-only, and capped at the
/// configured `end_time` of bounded streams.
///
/// Boundary witnesses follow the ContractGraph-QA oracle for this issue:
/// `T_before = end_time - 1`, `T_exact = end_time`, `T_after > end_time`.
///
/// Invariants:
///   I1 — a temporal interval is paid at most once;
///   I2 — accrual never passes the configured end time of a bounded stream;
///   I3 — initial_balance = cumulative_settled + remaining_balance;
///   I4 — reaching the natural end is terminal; repeated operations move
///        no additional value;
///   I5 — identical stored state plus ledger timestamp yields identical
///        results;
///   I6 — payer authorization and failure modes are preserved.
#[cfg(test)]
mod end_time_settlement_tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;

    use super::*;

    /// Rate used by every scenario: 10 units per second.
    const RATE: i128 = 10;
    /// Initial deposited balance used by every scenario.
    const BALANCE: i128 = 1_000;
    /// Configured end_time for bounded streams (start is always t=0).
    const END_TIME: u64 = 10;

    fn setup(env: &Env, end_time: u64) -> (StreamPayContractClient<'_>, u32) {
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(env, &contract_id);
        let payer = Address::generate(env);
        let recipient = Address::generate(env);
        let stream_id = client.create_stream(&payer, &recipient, &RATE, &BALANCE, &end_time);
        client.start_stream(&stream_id);
        (client, stream_id)
    }

    fn advance(env: &Env, seconds: u64) {
        env.ledger().with_mut(|li| {
            li.timestamp += seconds;
        });
    }

    /// A settle strictly before the end pays through the witness and keeps
    /// the stream active (control scenario).
    #[test]
    fn test_settle_before_end_accrues_through_now_and_stays_active() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME - 1);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 90, "I2: must pay exactly rate x (end-1)");

        let info = client.get_stream_info(&stream_id);
        assert!(info.is_active, "before the end the stream stays active");
        assert_eq!(info.balance, BALANCE - 90);
        assert_eq!(info.start_time, END_TIME - 1);
    }

    /// Settling exactly at the configured end pays the final interval and
    /// makes the stream terminal.
    #[test]
    fn test_settle_exactly_at_end_is_final_and_terminal() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, RATE * END_TIME as i128);

        let info = client.get_stream_info(&stream_id);
        assert!(
            !info.is_active,
            "I4: settling at the natural end must make the stream terminal"
        );
        assert_eq!(info.balance, BALANCE - RATE * END_TIME as i128);
    }

    /// A settle invoked after the configured end must not accrue beyond it.
    #[test]
    fn test_settle_after_end_capped_at_configured_end_time() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME + 10);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(
            amount,
            RATE * END_TIME as i128,
            "I2: accrual must be capped at the configured end_time"
        );

        let info = client.get_stream_info(&stream_id);
        assert!(!info.is_active, "I4: late settle ends the stream");
        assert_eq!(info.balance, BALANCE - RATE * END_TIME as i128);
    }

    /// Repeated settlement of a naturally ended stream moves no further
    /// value regardless of how much time passes.
    #[test]
    fn test_repeated_settle_after_end_moves_no_value() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME + 5);
        let first = client.settle_stream(&stream_id);
        assert_eq!(first, RATE * END_TIME as i128, "I2: capped at end_time");

        advance(&env, 100);
        let second = client.settle_stream(&stream_id);
        assert_eq!(second, 0, "I4: terminal stream must not accrue again");

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, BALANCE - RATE * END_TIME as i128);
        assert!(!info.is_active);
    }

    /// Cancelling strictly before the end pays through the cancellation
    /// witness, which becomes the stored terminal boundary.
    #[test]
    fn test_cancel_before_end_uses_cancellation_witness_as_boundary() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME - 3);
        client.cancel_stream(&stream_id);

        let info = client.get_stream_info(&stream_id);
        assert!(!info.is_active);
        assert_eq!(info.balance, BALANCE - RATE * (END_TIME as i128 - 3));
        assert_eq!(info.end_time, END_TIME - 3, "I5: boundary is deterministic");
    }

    /// Cancelling exactly at the end matches the natural-end economics.
    #[test]
    fn test_cancel_exactly_at_end_matches_natural_end_result() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME);
        client.cancel_stream(&stream_id);

        let info = client.get_stream_info(&stream_id);
        assert!(!info.is_active);
        assert_eq!(info.balance, BALANCE - RATE * END_TIME as i128);
        assert_eq!(info.end_time, END_TIME);
    }

    /// A cancellation invoked after the configured end must respect the
    /// natural end: accrual is capped and the boundary does not move.
    #[test]
    fn test_cancel_after_end_natural_end_takes_precedence() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME + 10);
        client.cancel_stream(&stream_id);

        let info = client.get_stream_info(&stream_id);
        assert!(
            !info.is_active,
            "late cancellation of an ended stream is terminal"
        );
        assert_eq!(
            info.balance,
            BALANCE - RATE * END_TIME as i128,
            "I2: no accrual past the configured end"
        );
        assert_eq!(
            info.end_time, END_TIME,
            "natural end wins over the late cancellation witness"
        );
    }

    /// Once a stream reached its natural end, repeated terminal operations
    /// cannot move value: extra settles are inert and cancel is rejected.
    #[test]
    fn test_terminal_operations_cannot_move_value_twice() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, END_TIME);

        advance(&env, END_TIME + 1);
        assert_eq!(client.settle_stream(&stream_id), RATE * END_TIME as i128);
        let settled_info = client.get_stream_info(&stream_id);

        // Cancel after natural end must be rejected (failure mode preserved).
        let result = catch_unwind(AssertUnwindSafe(|| {
            client.cancel_stream(&stream_id);
        }));
        assert!(result.is_err(), "cancelling a terminal stream must fail");

        advance(&env, 50);
        assert_eq!(
            client.settle_stream(&stream_id),
            0,
            "I4: repeated settle after natural end moves nothing"
        );

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, settled_info.balance);
        assert_eq!(info.start_time, settled_info.start_time);
        assert!(!info.is_active);
    }

    /// Value conservation holds across every boundary scenario.
    #[test]
    fn test_value_conservation_across_boundary_scenarios() {
        // settle before / at / after the end
        for &(witness, expected_paid) in &[
            (END_TIME - 1, RATE * (END_TIME as i128 - 1)),
            (END_TIME, RATE * END_TIME as i128),
            (END_TIME + 7, RATE * END_TIME as i128),
        ] {
            let env = Env::default();
            let (client, stream_id) = setup(&env, END_TIME);
            advance(&env, witness);
            let paid = client.settle_stream(&stream_id);
            assert_eq!(paid, expected_paid);
            let info = client.get_stream_info(&stream_id);
            assert_eq!(
                paid + info.balance,
                BALANCE,
                "I3: settle at {witness} conserves value"
            );
        }

        // cancel before / at / after the end
        for witness in [END_TIME - 2, END_TIME, END_TIME + 9] {
            let env = Env::default();
            let (client, stream_id) = setup(&env, END_TIME);
            advance(&env, witness);
            client.cancel_stream(&stream_id);
            let info = client.get_stream_info(&stream_id);
            let paid = BALANCE - info.balance;
            assert!(
                paid <= RATE * END_TIME as i128,
                "I2: cancel at {witness} must not exceed the end-time cap"
            );
            assert_eq!(
                paid + info.balance,
                BALANCE,
                "I3: cancel at {witness} conserves value"
            );
        }
    }

    /// Unlimited streams keep accruing through the current ledger time.
    #[test]
    fn test_unlimited_stream_keeps_current_time_behavior() {
        let env = Env::default();
        let (client, stream_id) = setup(&env, 0);

        advance(&env, 50);
        assert_eq!(client.settle_stream(&stream_id), 500);
        assert!(
            client.get_stream_info(&stream_id).is_active,
            "unlimited streams do not become terminal on settle"
        );

        advance(&env, 10);
        assert_eq!(client.settle_stream(&stream_id), 100);
    }
}

/// Property-based tests verifying accrual upper-bound invariants.
///
/// **Invariants:**
///   I1 (Balance bound):  `settle_stream` result ≤ stream balance before settlement.
///   I2 (Rate bound):     `settle_stream` result ≤ `rate_per_second × elapsed_seconds`.
///   I3 (Non-negative):   balance after every settlement is ≥ 0 (no overdraft).
///   I4 (Cumulative):     sum of all `settle_stream` results over a stream's lifetime ≤ original balance.
///
/// Each test iterates over `SEEDS` — a fixed set of deterministic 64-bit seeds — so the
/// suite is fully reproducible in CI without flakiness. The LCG drives parameter selection
/// only; no non-determinism is introduced at runtime.
#[cfg(test)]
mod property_tests {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;

    use super::*;

    // ── Deterministic PRNG ────────────────────────────────────────────────────

    /// Linear congruential generator — Knuth multiplicative parameters.
    /// Chosen for simplicity and zero external dependencies.
    struct Lcg(u64);

    impl Lcg {
        /// Seed and warm up (8 rounds) to reduce seed-value correlation.
        fn new(seed: u64) -> Self {
            let mut g = Self(seed);
            for _ in 0..8 {
                g.next_u64();
            }
            g
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005_u64)
                .wrapping_add(1_442_695_040_888_963_407_u64);
            self.0
        }

        /// Uniform sample from `[lo, hi)`. Panics if `lo >= hi`.
        fn in_range(&mut self, lo: u64, hi: u64) -> u64 {
            assert!(lo < hi, "in_range: lo must be < hi");
            lo + self.next_u64() % (hi - lo)
        }
    }

    // ── Deterministic seed table ──────────────────────────────────────────────

    /// Fixed seeds covering a range of bit patterns.
    /// Add seeds here when a new edge case is discovered in the wild.
    const SEEDS: &[u64] = &[
        0x0000_0000_0000_0001, // minimal
        0x0000_0000_0000_0002,
        0x0000_0000_0000_0010,
        0xDEAD_BEEF_CAFE_BABE,
        0x1234_5678_9ABC_DEF0,
        0x7FFF_FFFF_7FFF_FFFF, // near max signed
        0x8000_0000_0000_0000, // high bit set
        0x5A5A_5A5A_5A5A_5A5A, // alternating nibbles
        0xA3B2_C1D0_E9F8_0712,
        0x0BAD_F00D_1337_C0DE,
        0x0000_0000_0000_0000, // zero (degenerate)
        0x0102_0304_0506_0708,
        0xFEDC_BA98_7654_3210,
        0x1111_1111_1111_1111,
        0x9999_9999_9999_9999,
        0x0000_0001_0000_0001,
        0x1000_0000_0000_0000,
        0xCAFE_BABE_DEAD_BEEF,
        0x4242_4242_4242_4242,
        0xF0F0_F0F0_F0F0_F0F0,
    ];

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    /// Derive `(rate, balance, elapsed)` deterministically from a seed.
    ///
    /// Ranges chosen to cover pre-drain, near-drain, and post-drain scenarios
    /// while keeping `cargo test` run time acceptable.
    fn params(seed: u64) -> (i128, i128, u64) {
        let mut rng = Lcg::new(seed);
        // rate: 1..1_000_001
        let rate = rng.in_range(1, 1_000_001) as i128;
        // balance: 1..1_000_000_001
        let balance = rng.in_range(1, 1_000_000_001) as i128;
        // elapsed: 0..=(balance/rate + 100), capped to avoid u64 overflow
        let drain_secs = (balance / rate) as u64;
        let max_elapsed = drain_secs.saturating_add(100).min(10_000_000);
        let elapsed = rng.in_range(0, max_elapsed + 1);
        (rate, balance, elapsed)
    }

    // ── Property tests ────────────────────────────────────────────────────────

    /// I1 + I2: A single settlement must satisfy both upper bounds.
    ///
    /// - I1: `accrual ≤ balance_before`
    /// - I2: `accrual ≤ rate_per_second × elapsed`
    #[test]
    fn prop_single_settle_within_bounds() {
        for &seed in SEEDS {
            let (rate, balance, elapsed) = params(seed);
            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);

            env.ledger().with_mut(|li| {
                li.timestamp += elapsed;
            });
            let accrual = client.settle_stream(&sid);

            // I1
            assert!(
                accrual <= balance,
                "seed=0x{seed:016X} I1: accrual {accrual} > balance {balance} \
                 (rate={rate}, elapsed={elapsed})"
            );
            // I2 — use saturating_mul to mirror contract arithmetic
            let rate_x_elapsed = (elapsed as i128).saturating_mul(rate);
            assert!(
                accrual <= rate_x_elapsed,
                "seed=0x{seed:016X} I2: accrual {accrual} > rate×elapsed {rate_x_elapsed} \
                 (rate={rate}, elapsed={elapsed})"
            );
        }
    }

    /// I3: Balance after each settlement is always ≥ 0 (no overdraft possible).
    #[test]
    fn prop_balance_non_negative_after_settle() {
        for &seed in SEEDS {
            let (rate, balance, elapsed) = params(seed);
            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);

            env.ledger().with_mut(|li| {
                li.timestamp += elapsed;
            });
            client.settle_stream(&sid);

            let info = client.get_stream_info(&sid);
            assert!(
                info.balance >= 0,
                "seed=0x{seed:016X} I3: balance {} < 0 after settle \
                 (rate={rate}, elapsed={elapsed})",
                info.balance
            );
        }
    }

    /// I4: Cumulative accrual over multiple settlements never exceeds original balance.
    #[test]
    fn prop_cumulative_settle_within_original_balance() {
        for &seed in SEEDS {
            let (rate, balance, _) = params(seed);
            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);

            // Settle 5 times at varied intervals derived from the seed
            let mut rng = Lcg::new(seed.wrapping_add(0xDEAD));
            let mut cumulative: i128 = 0;
            for _ in 0..5 {
                let step = rng.in_range(0, 101); // 0..=100 s
                env.ledger().with_mut(|li| {
                    li.timestamp += step;
                });
                cumulative += client.settle_stream(&sid);
            }

            assert!(
                cumulative <= balance,
                "seed=0x{seed:016X} I4: cumulative {cumulative} > original balance {balance}"
            );
        }
    }

    /// Edge: elapsed = 0 must always produce accrual = 0.
    #[test]
    fn prop_zero_elapsed_yields_zero_accrual() {
        for &seed in SEEDS {
            let (rate, balance, _) = params(seed);
            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);
            // No ledger advancement — elapsed is 0
            let accrual = client.settle_stream(&sid);

            assert_eq!(
                accrual, 0,
                "seed=0x{seed:016X} zero-elapsed: accrual {accrual} != 0 (rate={rate})"
            );
        }
    }

    /// Saturation: when rate × elapsed > balance the contract caps at balance (full drain).
    #[test]
    fn prop_saturated_settle_caps_at_balance() {
        for &seed in SEEDS {
            let mut rng = Lcg::new(seed);
            // Deliberately small ranges to guarantee rate × elapsed >> balance in every case
            let rate = rng.in_range(1, 1_001) as i128; // 1..=1_000
            let balance = rng.in_range(1, 10_001) as i128; // 1..=10_000

            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);

            // Advance past full drain: drain_secs + 1 guarantees elapsed × rate > balance
            let drain_secs = (balance / rate) as u64 + 1;
            env.ledger().with_mut(|li| {
                li.timestamp += drain_secs;
            });
            let accrual = client.settle_stream(&sid);

            assert_eq!(
                accrual, balance,
                "seed=0x{seed:016X} saturation: accrual {accrual} != balance {balance} \
                 (rate={rate}, drain_secs={drain_secs})"
            );
        }
    }

    /// Overflow safety: extreme rate/elapsed/balance values are handled by saturating arithmetic.
    /// I1 must still hold even when rate × elapsed would overflow i128.
    #[test]
    fn prop_large_values_satisfy_balance_bound() {
        // (rate, balance, elapsed): manually crafted cases where rate × elapsed overflows
        let cases: &[(i128, i128, u64)] = &[
            // rate × elapsed overflows → saturating_mul yields i128::MAX → min(i128::MAX, balance) = balance
            (i128::MAX / 2, i128::MAX - 1, 3),
            // huge elapsed, tiny rate → product < balance → partial drain
            (1, 1_000_000_000_000_i128, 500_000_000_000_u64),
            // product >> balance → full drain
            (1_000_000, 1_000_000_000_000_i128, 1_000_001),
            // near-overflow product
            (i128::MAX / 1_000, i128::MAX - 1, 999),
        ];

        for &(rate, balance, elapsed) in cases {
            let env = make_env();
            let cid = env.register(StreamPayContract, ());
            let client = StreamPayContractClient::new(&env, &cid);

            let payer = Address::generate(&env);
            let recipient = Address::generate(&env);
            let sid = client.create_stream(&payer, &recipient, &rate, &balance, &0_u64);
            client.start_stream(&sid);

            env.ledger().with_mut(|li| {
                li.timestamp += elapsed;
            });
            let accrual = client.settle_stream(&sid);

            assert!(
                accrual >= 0,
                "overflow I1a: accrual {accrual} < 0 (rate={rate}, balance={balance}, elapsed={elapsed})"
            );
            assert!(
                accrual <= balance,
                "overflow I1b: accrual {accrual} > balance {balance} (rate={rate}, elapsed={elapsed})"
            );
        }
    }
}
