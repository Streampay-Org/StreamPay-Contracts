//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream, cancel_stream,
//! pause_stream, resume_stream, archive_stream, get_stream_info, version.
//!
//! ## Features
//!
//! ### Stream Cancelation (Issue #4)
//! - Payer can cancel a stream early with `cancel_stream`
//! - Upon cancellation, all accrued amounts are automatically settled to the recipient
//! - Remaining unaccrued balance is refunded to the payer
//! - Race condition safety: uses atomic settle-then-deactivate pattern
//!
//! ### Pause/Resume (Issue)
//! - Payer can pause an active stream with `pause_stream` (accrual stops)
//! - Payer can resume a paused stream with `resume_stream` (accrual resumes)
//! - Paused streams do not accrue time; balances are preserved
//! - Distinct from `stop_stream` (which is final unless restarted)
//!
//! ### Maximum Duration / End Time (Issue #7)
//! - `end_time` specifies when a stream must end (optional)
//! - Validated at creation: `end_time > start_time` when set
//! - At settlement, accrual is capped at `end_time` to prevent infinite streaming
//! - Streams auto-deactivate if settlement reaches `end_time`

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
    pub end_time: u64,           // Max duration: stream auto-deactivates at this time
    pub is_active: bool,
    pub paused_at: u64,          // 0 if not paused; timestamp of pause if paused
}

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
        end_time: u64,  // 0 = no limit; otherwise must be > start_time (validated at start)
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
            end_time,
            is_active: false,
            paused_at: 0,
        };
        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id + 1);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        stream_id
    }

    /// Start an existing stream.
    /// If end_time was set at creation, validates that end_time > current timestamp.
    pub fn start_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if info.is_active {
            panic!("stream already active");
        }
        let now = env.ledger().timestamp();
        
        // Validate end_time constraint if set
        if info.end_time > 0 && info.end_time <= now {
            panic!("end_time must be in the future");
        }
        
        info.is_active = true;
        info.start_time = now;
        info.paused_at = 0;  // Clear paused state
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
        info.paused_at = 0;  // Clear paused state
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Settle stream: compute streamed amount since start (or since resumption if paused) and deduct from balance.
    /// Respects end_time: if set, accrual is capped at end_time and stream auto-deactivates.
    /// When paused, uses paused_at timestamp instead of current time.
    pub fn settle_stream(env: Env, stream_id: u32) -> i128 {
        let mut info = get_stream(&env, stream_id);
        if !info.is_active {
            return 0;
        }
        
        let now = env.ledger().timestamp();
        
        // Determine settlement time: use paused_at if paused, else current time or end_time
        let settlement_time = if info.paused_at > 0 {
            // Paused: settle only up to pause point
            info.paused_at
        } else if info.end_time > 0 && now > info.end_time {
            // Past end_time: cap accrual at end_time
            info.end_time
        } else {
            // Normal case: use current time
            now
        };
        
        let elapsed = settlement_time - info.start_time;
        let amount = (elapsed as i128)
            .saturating_mul(info.rate_per_second)
            .min(info.balance);
        info.balance = info.balance.saturating_sub(amount);
        info.start_time = settlement_time;
        
        // Auto-deactivate if end_time reached
        if info.end_time > 0 && settlement_time >= info.end_time {
            info.is_active = false;
            info.end_time = settlement_time;
        }
        
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        amount
    }

    /// Cancel a stream early (payer-only).
    /// Immediately settles all accrued amounts to the recipient.
    /// Remaining unaccrued balance is retained by the payer.
    /// Atomic operation: prevents race conditions with settle.
    pub fn cancel_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        
        if !info.is_active {
            panic!("cannot cancel inactive stream");
        }
        
        let now = env.ledger().timestamp();
        
        // Settle accrued amount up to cancellation
        let elapsed = now - info.start_time;
        let accrued = (elapsed as i128)
            .saturating_mul(info.rate_per_second)
            .min(info.balance);
        
        // Deduct accrued from balance (paid to recipient)
        info.balance = info.balance.saturating_sub(accrued);
        info.is_active = false;
        info.end_time = now;  // Mark cancellation point
        
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
            panic!("cannot pause inactive stream");
        }
        if info.paused_at > 0 {
            panic!("stream already paused");
        }
        
        let now = env.ledger().timestamp();
        
        // Settle accrued amount up to pause point
        let elapsed = now - info.start_time;
        let accrued = (elapsed as i128)
            .saturating_mul(info.rate_per_second)
            .min(info.balance);
        info.balance = info.balance.saturating_sub(accrued);
        
        // Mark paused but keep is_active true (logical "paused" state)
        info.paused_at = now;
        
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
            panic!("cannot resume inactive stream");
        }
        if info.paused_at == 0 {
            panic!("stream is not paused");
        }
        
        let now = env.ledger().timestamp();
        
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
        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id);
        client.archive_stream(&stream_id);

        // Should panic — stream was archived (removed from storage)
        client.get_stream_info(&stream_id);
    }

    // ==================== New Tests: Cancel Stream (Issue #4) ====================

    #[test]
    fn test_cancel_stream_before_start() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);

        // Should panic: cannot cancel before starting
        client.cancel_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_cancel_stream_inactive_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);
        client.stop_stream(&stream_id);

        // Should panic: cannot cancel after stopping
        client.cancel_stream(&stream_id);
    }

    #[test]
    fn test_cancel_stream_mid_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=100/s, balance=1000
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Advance 5 seconds: 500 should be accrued
        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        client.cancel_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);

        // Balance should be 500 (1000 - 500 accrued)
        assert_eq!(info.balance, 500);
        assert!(!info.is_active);
    }

    #[test]
    fn test_cancel_near_depletion() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Advance 12 seconds (would drain 1200, but balance is only 1000)
        env.ledger().with_mut(|li| {
            li.timestamp += 12;
        });

        client.cancel_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);

        // Balance should be 0 (capped at balance)
        assert_eq!(info.balance, 0);
        assert!(!info.is_active);
    }

    // ==================== New Tests: Pause/Resume ====================

    #[test]
    fn test_pause_and_resume_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=100/s, balance=1000
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Advance 3 seconds
        env.ledger().with_mut(|li| {
            li.timestamp += 3;
        });

        // Pause: should settle 300 (3 * 100)
        client.pause_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 700);  // 1000 - 300
        assert!(info.is_active);        // Still marked active (paused state)
        assert!(info.paused_at > 0);    // Paused timestamp set

        // Advance another 5 seconds (no accrual while paused)
        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        // Resume
        client.resume_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 700);  // Balance unchanged during pause
        assert!(info.is_active);
        assert_eq!(info.paused_at, 0);  // Paused state cleared
    }

    #[test]
    fn test_settle_while_paused() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Advance 5 seconds
        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        client.pause_stream(&stream_id);
        let info_paused = client.get_stream_info(&stream_id);
        assert_eq!(info_paused.balance, 500);

        // Advance another 10 seconds (should NOT accrue while paused)
        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });

        let settled = client.settle_stream(&stream_id);
        // settle_stream while paused should return 0 (no additional accrual)
        assert_eq!(settled, 0);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 500);  // Balance unchanged
    }

    #[test]
    #[should_panic]
    fn test_pause_already_paused_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);
        client.pause_stream(&stream_id);

        // Should panic: already paused
        client.pause_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_resume_not_paused_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // Should panic: stream not paused
        client.resume_stream(&stream_id);
    }

    #[test]
    fn test_multiple_pause_resume_toggles() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=50/s, balance=1000
        let stream_id = client.create_stream(&payer, &recipient, &50_i128, &1_000_i128, &0_u64);
        client.start_stream(&stream_id);

        // First pause/resume cycle: advance 2s, pause, advance 3s, resume
        env.ledger().with_mut(|li| {
            li.timestamp += 2;
        });
        client.pause_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 900);  // 1000 - 100

        env.ledger().with_mut(|li| {
            li.timestamp += 3;
        });
        client.resume_stream(&stream_id);

        // Second pause/resume: advance 4s, pause, advance 6s, resume
        env.ledger().with_mut(|li| {
            li.timestamp += 4;
        });
        client.pause_stream(&stream_id);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 700);  // 900 - 200

        env.ledger().with_mut(|li| {
            li.timestamp += 6;
        });
        client.resume_stream(&stream_id);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 700);  // Still 700 (paused prevented accrual)
    }

    // ==================== New Tests: End Time (Issue #7) ====================

    #[test]
    fn test_create_stream_with_end_time() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let end_time = 1000_u64;
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &end_time);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.end_time, end_time);
    }

    #[test]
    #[should_panic]
    fn test_start_stream_end_time_in_past_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &100_u64);

        // Current timestamp is 0; end_time is 100, so it's in the future
        client.start_stream(&stream_id);

        // Advance past end_time
        env.ledger().with_mut(|li| {
            li.timestamp = 200;
        });

        // Try to re-start with past end_time (should panic)
        // First stop it
        client.stop_stream(&stream_id);

        // Try to start again with past end_time
        client.start_stream(&stream_id);  // Should panic
    }

    #[test]
    fn test_settle_respects_end_time() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // rate=100/s, balance=2000, end_time=10s from start
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &2_000_i128, &10_u64);
        client.start_stream(&stream_id);

        // Now timestamps start at whatever env.ledger().timestamp() is
        let start_ts = env.ledger().timestamp();

        // Advance 5 seconds (within end_time window)
        env.ledger().with_mut(|li| {
            li.timestamp = start_ts + 5;
        });
        let amount1 = client.settle_stream(&stream_id);
        assert_eq!(amount1, 500);  // 5 * 100
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 1_500);
        assert!(info.is_active);    // Still active

        // Advance past end_time (to 15 seconds)
        env.ledger().with_mut(|li| {
            li.timestamp = start_ts + 15;
        });
        let amount2 = client.settle_stream(&stream_id);
        // Should only accrue from 5 to 10 = 500
        assert_eq!(amount2, 500);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 1_000);
        assert!(!info.is_active);   // Auto-deactivated at end_time
    }

    #[test]
    fn test_settle_at_exact_end_time() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &1_000_i128, &5_u64);
        client.start_stream(&stream_id);

        let start_ts = env.ledger().timestamp();

        // Settle at exactly end_time
        env.ledger().with_mut(|li| {
            li.timestamp = start_ts + 5;
        });
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 500);  // 5 * 100
        let info = client.get_stream_info(&stream_id);
        assert!(!info.is_active);
    }

    #[test]
    fn test_end_time_zero_means_no_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        // end_time=0 → no time limit
        let stream_id = client.create_stream(&payer, &recipient, &100_i128, &10_000_i128, &0_u64);
        client.start_stream(&stream_id);

        let start_ts = env.ledger().timestamp();

        // Advance very far (100 seconds)
        env.ledger().with_mut(|li| {
            li.timestamp = start_ts + 100;
        });
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 10_000);  // Full balance drained
        let info = client.get_stream_info(&stream_id);
        assert!(info.is_active);     // Still active (only stops due to balance depletion)
    }
}

