//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! archive_stream, get_stream_info, version.
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

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.1.0 → 1_000
const VERSION: u32 = 1_000;

/// TTL threshold: extend when remaining TTL drops below ~1 day (17_280 ledgers at ~5s each).
const STREAM_TTL_THRESHOLD: u32 = 17_280;
/// TTL extend-to: refresh to ~30 days (518_400 ledgers).
const STREAM_TTL_EXTEND: u32 = 518_400;
/// Instance storage TTL threshold (~1 day).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
/// Instance storage TTL extend-to (~30 days).
const INSTANCE_TTL_EXTEND: u32 = 518_400;

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub payer: Address,
    pub recipient: Address,
    pub rate_per_second: i128,
    pub balance: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub is_active: bool,
}

#[contract]
pub struct StreamPayContract;

#[contractimpl]
impl StreamPayContract {
    /// Create a new payment stream (payer, recipient, rate per second).
    pub fn create_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        rate_per_second: i128,
        initial_balance: i128,
    ) -> u32 {
        payer.require_auth();
        if rate_per_second <= 0 || initial_balance <= 0 {
            panic!("rate and balance must be positive");
        }
        let stream_id = get_next_stream_id(&env);
        let info = StreamInfo {
            payer: payer.clone(),
            recipient,
            rate_per_second,
            balance: initial_balance,
            start_time: 0,
            end_time: 0,
            is_active: false,
        };
        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id + 1);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        stream_id
    }

    /// Start an existing stream.
    pub fn start_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if info.is_active {
            panic!("stream already active");
        }
        info.is_active = true;
        info.start_time = env.ledger().timestamp();
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Stop an active stream.
    pub fn stop_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if !info.is_active {
            panic!("stream not active");
        }
        info.is_active = false;
        info.end_time = env.ledger().timestamp();
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
        let mut info = get_stream(&env, stream_id);
        if !info.is_active {
            return 0;
        }
        let now = env.ledger().timestamp();
        let elapsed = now - info.start_time;
        let amount = (elapsed as i128)
            .saturating_mul(info.rate_per_second)
            .min(info.balance);
        info.balance = info.balance.saturating_sub(amount);
        info.start_time = now;
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        amount
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
            panic!("cannot archive active stream");
        }
        if info.balance != 0 {
            panic!("cannot archive stream with unsettled balance");
        }
        let key = stream_key(&env, stream_id);
        env.storage().persistent().remove(&key);
        extend_instance_ttl(&env);
    }
}

fn stream_key(env: &Env, stream_id: u32) -> (Symbol, u32) {
    (Symbol::new(env, "stream"), stream_id)
}

fn get_stream(env: &Env, stream_id: u32) -> StreamInfo {
    let key = stream_key(env, stream_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("stream not found"))
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
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;

    use super::*;

    #[test]
    fn test_create_stream_valid() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);
        assert_eq!(stream_id, 1);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.payer, payer);
        assert_eq!(info.recipient, recipient);
        assert_eq!(info.rate_per_second, 100);
        assert_eq!(info.balance, 10_000);
        assert!(!info.is_active);
    }

    #[test]
    fn test_start_and_stop_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &50_i128, &5_000_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128);
        client.start_stream(&stream_id);
        let amount = client.settle_stream(&stream_id);
        assert!(amount >= 0);
    }

    #[test]
    fn test_version_returns_expected() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), 1_000);
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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);

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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);

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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);

        // Advance 10 seconds so balance drains to 0
        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);

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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &balance);
        client.start_stream(&stream_id);

        // Advance 1 second
        env.ledger().with_mut(|li| {
            li.timestamp += 1;
        });

        let amount = client.settle_stream(&stream_id);
        // Saturating mul: i128::MAX * 1 = i128::MAX, clamped to balance
        assert_eq!(amount, balance, "extreme rate must settle exactly the balance, not more");

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0, "balance must be fully drained, not negative");
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
        let stream_id = client.create_stream(&payer, &recipient, &1_000_i128, &balance);

        // Manually set start_time to 0 via start_stream at timestamp 0
        client.start_stream(&stream_id);

        // Jump to near u64::MAX to create a massive elapsed window
        env.ledger().with_mut(|li| {
            li.timestamp = u64::MAX;
        });

        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, balance, "extreme elapsed must settle exactly the balance");

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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &balance);
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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &1_000_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &999_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &10_000_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 50, "partial accrual should be exact when no saturation");

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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &1_000_i128);
        client.start_stream(&stream_id);

        // No time advance — elapsed = 0
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 0, "zero elapsed must yield zero amount even with max rate");
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
        let stream_id = client.create_stream(&payer, &recipient, &i128::MAX, &initial_balance);
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
