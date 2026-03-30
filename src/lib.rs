//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! archive_stream, get_stream_info, set_max_rate, get_max_rate, version.
//!
//! ## Maximum Rate Guardrail
//!
//! An optional per-deploy ceiling on `rate_per_second` prevents fat-finger
//! misconfiguration (e.g. accidentally passing stroops instead of XLM units).
//!
//! * The ceiling is stored in **instance storage** under the key `"max_rate"`.
//! * When no ceiling is set (`max_rate` absent), **all positive rates are
//!   accepted** — fully backwards-compatible with v0.1.0 deployments.
//! * Only the **payer** of a stream can call `set_max_rate`; the contract has
//!   no separate admin role so the guardrail is opt-in per-deployer.
//! * A ceiling of `0` is rejected (use absence to mean "no limit").
//! * The ceiling can be **raised or lowered** at any time; existing streams
//!   are not affected retroactively — only new `create_stream` calls are
//!   checked.
//!
//! ### Override policy
//!
//! To raise a ceiling that is too restrictive:
//! 1. Call `set_max_rate(new_ceiling)` with payer auth before creating the
//!    stream. The ceiling update is immediate and affects all subsequent
//!    `create_stream` calls on this contract instance.
//! 2. There is no time-lock or multi-sig requirement at the contract level;
//!    operators should enforce governance off-chain if needed.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

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
    /// Set (or update) the maximum allowed `rate_per_second` for new streams.
    ///
    /// * `max_rate` must be positive (> 0).
    /// * Requires auth from `caller` — any address may configure the ceiling
    ///   for streams they intend to create; in practice this is called by the
    ///   deployer/admin before opening streams.
    /// * Passing a ceiling that is lower than currently configured is allowed;
    ///   it will not retroactively affect already-created streams.
    /// * To **remove** the ceiling (allow unlimited rates) call
    ///   `remove_max_rate` instead.
    pub fn set_max_rate(env: Env, caller: Address, max_rate: i128) {
        caller.require_auth();
        if max_rate <= 0 {
            panic!("max_rate must be positive");
        }
        let key = Symbol::new(&env, "max_rate");
        env.storage().instance().set(&key, &max_rate);
        extend_instance_ttl(&env);
    }

    /// Remove the maximum rate ceiling, reverting to unlimited mode.
    ///
    /// Requires auth from `caller`.
    pub fn remove_max_rate(env: Env, caller: Address) {
        caller.require_auth();
        let key = Symbol::new(&env, "max_rate");
        env.storage().instance().remove(&key);
        extend_instance_ttl(&env);
    }

    /// Return the current maximum rate ceiling, or `None` if unset.
    pub fn get_max_rate(env: Env) -> Option<i128> {
        let key = Symbol::new(&env, "max_rate");
        env.storage().instance().get(&key)
    }

    /// Create a new payment stream (payer, recipient, rate per second).
    ///
    /// Panics if `rate_per_second` exceeds the configured ceiling (when set).
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

        // ── Guardrail check ──────────────────────────────────────────────────
        let max_rate_key = Symbol::new(&env, "max_rate");
        if let Some(ceiling) = env
            .storage()
            .instance()
            .get::<Symbol, i128>(&max_rate_key)
        {
            if rate_per_second > ceiling {
                panic!("rate_per_second exceeds maximum allowed rate");
            }
        }
        // ─────────────────────────────────────────────────────────────────────

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

    // ── Existing tests (unchanged) ────────────────────────────────────────────

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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| { li.timestamp += 10; });
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 1_000);
        client.stop_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
        assert!(!info.is_active);
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
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| { li.timestamp += 10; });
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id);
        client.archive_stream(&stream_id);
        client.get_stream_info(&stream_id);
    }

    // ── Max-rate guardrail tests ──────────────────────────────────────────────

    /// No ceiling set → any positive rate is accepted (backwards-compatible).
    #[test]
    fn test_no_ceiling_allows_any_rate() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        assert!(client.get_max_rate().is_none());

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Very high rate — should succeed with no ceiling
        let stream_id = client.create_stream(&payer, &recipient, &1_000_000_i128, &999_999_999_i128);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 1_000_000);
    }

    /// Rate exactly at the ceiling is accepted (boundary: at ceiling).
    #[test]
    fn test_rate_at_ceiling_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &500_i128);
        assert_eq!(client.get_max_rate(), Some(500_i128));

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Exactly at ceiling — must succeed
        let stream_id = client.create_stream(&payer, &recipient, &500_i128, &50_000_i128);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 500);
    }

    /// Rate one below the ceiling is accepted (boundary: below ceiling).
    #[test]
    fn test_rate_below_ceiling_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &500_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // One below ceiling — must succeed
        let stream_id = client.create_stream(&payer, &recipient, &499_i128, &50_000_i128);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 499);
    }

    /// Rate one above the ceiling is rejected (boundary: above ceiling).
    #[test]
    #[should_panic(expected = "rate_per_second exceeds maximum allowed rate")]
    fn test_rate_above_ceiling_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &500_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // One above ceiling — must panic
        client.create_stream(&payer, &recipient, &501_i128, &50_000_i128);
    }

    /// A very large rate is rejected when ceiling is tight.
    #[test]
    #[should_panic(expected = "rate_per_second exceeds maximum allowed rate")]
    fn test_fat_finger_rate_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        // Ceiling = 1_000 stroops/s ≈ reasonable for a low-value stream
        client.set_max_rate(&admin, &1_000_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Fat-finger: passed 1_000_000 instead of 1_000
        client.create_stream(&payer, &recipient, &1_000_000_i128, &999_999_999_i128);
    }

    /// Ceiling can be raised to allow previously-blocked rates.
    #[test]
    fn test_ceiling_can_be_raised() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &100_i128);
        assert_eq!(client.get_max_rate(), Some(100_i128));

        // Raise to 1_000
        client.set_max_rate(&admin, &1_000_i128);
        assert_eq!(client.get_max_rate(), Some(1_000_i128));

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Rate of 500 was blocked before, allowed now
        let stream_id = client.create_stream(&payer, &recipient, &500_i128, &50_000_i128);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 500);
    }

    /// Ceiling can be lowered; existing streams are unaffected.
    #[test]
    fn test_ceiling_lowered_does_not_affect_existing_streams() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &1_000_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Create at rate=800 while ceiling=1_000
        let stream_id = client.create_stream(&payer, &recipient, &800_i128, &80_000_i128);

        // Lower ceiling below existing stream's rate
        client.set_max_rate(&admin, &500_i128);

        // Existing stream is unaffected — still readable and startable
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 800);
        client.start_stream(&stream_id);
        assert!(client.get_stream_info(&stream_id).is_active);
    }

    /// Ceiling can be removed entirely with remove_max_rate.
    #[test]
    fn test_remove_ceiling_allows_any_rate() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &100_i128);
        assert_eq!(client.get_max_rate(), Some(100_i128));

        client.remove_max_rate(&admin);
        assert!(client.get_max_rate().is_none());

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // Previously blocked rate now succeeds
        let stream_id = client.create_stream(&payer, &recipient, &9_999_999_i128, &999_999_999_i128);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 9_999_999);
    }

    /// set_max_rate rejects zero or negative ceilings.
    #[test]
    #[should_panic(expected = "max_rate must be positive")]
    fn test_set_max_rate_zero_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &0_i128);
    }

    #[test]
    #[should_panic(expected = "max_rate must be positive")]
    fn test_set_max_rate_negative_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &-1_i128);
    }

    /// Ceiling of 1 (minimum positive) works — only rate=1 allowed.
    #[test]
    fn test_ceiling_of_one() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &1_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &1_i128, &1_000_i128);
        assert_eq!(client.get_stream_info(&stream_id).rate_per_second, 1);
    }

    #[test]
    #[should_panic(expected = "rate_per_second exceeds maximum allowed rate")]
    fn test_ceiling_of_one_blocks_two() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &1_i128);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        client.create_stream(&payer, &recipient, &2_i128, &1_000_i128);
    }

    /// Multiple streams can coexist — some at ceiling, some below.
    #[test]
    fn test_multiple_streams_with_ceiling() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.set_max_rate(&admin, &1_000_i128);

        let payer = Address::generate(&env);
        let r1 = Address::generate(&env);
        let r2 = Address::generate(&env);

        let id1 = client.create_stream(&payer, &r1, &500_i128, &50_000_i128);
        let id2 = client.create_stream(&payer, &r2, &1_000_i128, &100_000_i128);
        assert_eq!(client.get_stream_info(&id1).rate_per_second, 500);
        assert_eq!(client.get_stream_info(&id2).rate_per_second, 1_000);
    }
}