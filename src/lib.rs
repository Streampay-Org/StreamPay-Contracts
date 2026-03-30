//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! batch_settle, archive_stream, get_stream_info, version.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

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
            panic!("batch too large");
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

fn settle_stream_amount(env: &Env, stream_id: u32) -> Option<i128> {
    let mut info = get_stream(env, stream_id);
    if !info.is_active {
        return None;
    }

    let now = env.ledger().timestamp();
    let elapsed = now - info.start_time;
    let amount = (elapsed as i128)
        .saturating_mul(info.rate_per_second)
        .min(info.balance);
    info.balance = info.balance.saturating_sub(amount);
    info.start_time = now;
    set_stream(env, stream_id, &info);
    extend_stream_ttl(env, stream_id);

    Some(amount)
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
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128);

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
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128);
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
        let first_stream_id = client.create_stream(&payer, &recipient_a, &10_i128, &1_000_i128);
        let second_stream_id = client.create_stream(&payer, &recipient_b, &5_i128, &1_000_i128);
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
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128);
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
}
