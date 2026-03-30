//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! archive_stream, get_stream_info, version.

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

/// Stream unlock mode.
///
/// Invariants:
/// - `BasicRate` unlocks from elapsed wall time using `rate_per_second`.
/// - `LinearVesting` unlocks as a cumulative linear function of
///   `total_amount * elapsed / duration_seconds` from the first start time.
/// - For `LinearVesting`, `vested_amount` is monotonic and never exceeds
///   `total_amount`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamMode {
    BasicRate,
    LinearVesting {
        total_amount: i128,
        duration_seconds: u64,
        vested_amount: i128,
        vesting_start_time: u64,
    },
}

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
    pub mode: StreamMode,
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
            mode: StreamMode::BasicRate,
        };
        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id + 1);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        stream_id
    }

    /// Create a stream with linear vesting unlock mode.
    ///
    /// Unlock amount is derived from elapsed schedule time and not from a
    /// caller-supplied `rate_per_second`.
    pub fn create_vesting_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        initial_balance: i128,
        duration_seconds: u64,
    ) -> u32 {
        payer.require_auth();
        if initial_balance <= 0 || duration_seconds == 0 {
            panic!("balance and duration must be positive");
        }

        let stream_id = get_next_stream_id(&env);
        let info = StreamInfo {
            payer: payer.clone(),
            recipient,
            rate_per_second: 0,
            balance: initial_balance,
            start_time: 0,
            end_time: 0,
            is_active: false,
            mode: StreamMode::LinearVesting {
                total_amount: initial_balance,
                duration_seconds,
                vested_amount: 0,
                vesting_start_time: 0,
            },
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
        let now = env.ledger().timestamp();
        info.is_active = true;
        info.start_time = now;
        if let StreamMode::LinearVesting {
            total_amount,
            duration_seconds,
            vested_amount,
            vesting_start_time,
        } = info.mode.clone()
        {
            let anchored_start = if vesting_start_time == 0 {
                now
            } else {
                vesting_start_time
            };
            info.mode = StreamMode::LinearVesting {
                total_amount,
                duration_seconds,
                vested_amount,
                vesting_start_time: anchored_start,
            };
        }
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
        let amount = match info.mode.clone() {
            StreamMode::BasicRate => {
                let elapsed = now.saturating_sub(info.start_time);
                let amount = (elapsed as i128)
                    .saturating_mul(info.rate_per_second)
                    .min(info.balance);
                info.start_time = now;
                amount
            }
            StreamMode::LinearVesting {
                total_amount,
                duration_seconds,
                vested_amount,
                vesting_start_time,
            } => {
                if duration_seconds == 0 {
                    panic!("invalid vesting duration");
                }
                let elapsed = now.saturating_sub(vesting_start_time);
                let total_vested = compute_linear_vested(total_amount, duration_seconds, elapsed);
                let newly_vested = total_vested.saturating_sub(vested_amount);
                let amount = newly_vested.min(info.balance);

                info.mode = StreamMode::LinearVesting {
                    total_amount,
                    duration_seconds,
                    vested_amount: vested_amount.saturating_add(amount),
                    vesting_start_time,
                };
                amount
            }
        };
        info.balance = info.balance.saturating_sub(amount);
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

fn compute_linear_vested(total_amount: i128, duration_seconds: u64, elapsed_seconds: u64) -> i128 {
    let capped_elapsed = if elapsed_seconds > duration_seconds {
        duration_seconds
    } else {
        elapsed_seconds
    };
    total_amount.saturating_mul(capped_elapsed as i128) / duration_seconds as i128
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
        assert_eq!(info.mode, StreamMode::BasicRate);
    }

    #[test]
    fn test_create_vesting_stream_valid() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &10_000_i128, &100_u64);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.payer, payer);
        assert_eq!(info.recipient, recipient);
        assert_eq!(info.rate_per_second, 0);
        assert_eq!(info.balance, 10_000);
        assert!(!info.is_active);
        assert_eq!(
            info.mode,
            StreamMode::LinearVesting {
                total_amount: 10_000,
                duration_seconds: 100,
                vested_amount: 0,
                vesting_start_time: 0,
            }
        );
    }

    #[test]
    #[should_panic]
    fn test_create_vesting_stream_zero_duration_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        client.create_vesting_stream(&payer, &recipient, &10_000_i128, &0_u64);
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
    fn test_vesting_unlocks_linearly_across_multiple_settlements() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &1_000_i128, &100_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
        let first = client.settle_stream(&stream_id);
        assert_eq!(first, 100);

        env.ledger().with_mut(|li| {
            li.timestamp += 40;
        });
        let second = client.settle_stream(&stream_id);
        assert_eq!(second, 400);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 500);
        if let StreamMode::LinearVesting { vested_amount, .. } = info.mode {
            assert_eq!(vested_amount, 500);
        } else {
            panic!("expected linear vesting mode");
        }
    }

    #[test]
    fn test_vesting_releases_rounding_remainder_at_end() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &1_000_i128, &3_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 1;
        });
        assert_eq!(client.settle_stream(&stream_id), 333);

        env.ledger().with_mut(|li| {
            li.timestamp += 1;
        });
        assert_eq!(client.settle_stream(&stream_id), 333);

        env.ledger().with_mut(|li| {
            li.timestamp += 1;
        });
        assert_eq!(client.settle_stream(&stream_id), 334);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
    }

    #[test]
    fn test_vesting_schedule_anchor_persists_across_restart() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &1_000_i128, &100_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
        client.stop_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 20;
        });
        client.start_stream(&stream_id);

        // Vesting is anchored to the first start time, so 30% is now claimable.
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 300);
    }

    #[test]
    fn test_vesting_unlocks_full_balance_after_duration() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &1_000_i128, &10_u64);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 15;
        });
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 1_000);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
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
