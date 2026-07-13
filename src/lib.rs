//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! batch_settle, withdraw_stream, archive_stream, get_stream_info, version.

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, String, Symbol, Vec};

mod stream;

use stream::{
    extend_stream_ttl, get_stream, set_stream, stream_key, StreamInfo, StreamMode,
    STREAM_SCHEMA_VERSION,
};

/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.2.0 → 2_000
const VERSION: u32 = 2_000;

/// Instance storage TTL threshold (~1 day).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
/// Instance storage TTL extend-to (~30 days).
const INSTANCE_TTL_EXTEND: u32 = 518_400;
/// Hard cap for batch settlement to keep Soroban resource usage predictable.
const MAX_BATCH_SETTLE_SIZE: u32 = 25;
/// Maximum memo length in bytes.
const MEMO_MAX_LEN: usize = 32;
/// Minimum allowed streaming rate.
const MIN_RATE_PER_SECOND: i128 = 1;
/// Minimum initial escrow deposit.
const MIN_INITIAL_BALANCE: i128 = 1;

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
    /// Create a new payment stream and escrow `initial_balance` tokens.
    pub fn create_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        token_addr: Address,
        rate_per_second: i128,
        initial_balance: i128,
        recipient_can_stop: bool,
    ) -> u32 {
        Self::create_stream_with_options(
            env,
            payer,
            recipient,
            token_addr,
            rate_per_second,
            initial_balance,
            String::from_str(&env, ""),
            0,
            recipient_can_stop,
        )
    }

    /// Create a stream with memo and optional `end_time` (0 = unlimited).
    pub fn create_stream_with_options(
        env: Env,
        payer: Address,
        recipient: Address,
        token_addr: Address,
        rate_per_second: i128,
        initial_balance: i128,
        memo: String,
        end_time: u64,
        recipient_can_stop: bool,
    ) -> u32 {
        payer.require_auth();
        if memo.len() > MEMO_MAX_LEN as u32 {
            panic!("memo exceeds 32 chars");
        }
        if rate_per_second < MIN_RATE_PER_SECOND || initial_balance < MIN_INITIAL_BALANCE {
            panic!("rate and balance must be positive");
        }

        token::Client::new(&env, &token_addr).transfer(
            &payer,
            &env.current_contract_address(),
            &initial_balance,
        );

        let stream_id = get_next_stream_id(&env);
        if stream_id == 0 {
            panic!("stream id overflow");
        }

        let info = StreamInfo {
            schema_version: STREAM_SCHEMA_VERSION,
            payer: payer.clone(),
            recipient: recipient.clone(),
            token: token_addr,
            rate_per_second,
            balance: initial_balance,
            claimable_balance: 0,
            start_time: 0,
            end_time,
            is_active: false,
            paused_at: 0,
            memo,
            recipient_can_stop,
            mode: StreamMode::Linear,
        };

        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id.wrapping_add(1));
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        emit_stream_created(
            &env,
            stream_id,
            &payer,
            &recipient,
            rate_per_second,
            initial_balance,
        );
        stream_id
    }

    /// Create a linear vesting stream (total unlocks evenly over `duration_seconds`).
    pub fn create_vesting_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        total_amount: i128,
        duration_seconds: u64,
    ) -> u32 {
        payer.require_auth();
        if total_amount < MIN_INITIAL_BALANCE {
            panic!("rate and balance must be positive");
        }
        if duration_seconds == 0 {
            panic!("vesting duration must be positive");
        }

        let stream_id = get_next_stream_id(&env);
        if stream_id == 0 {
            panic!("stream id overflow");
        }

        let info = StreamInfo {
            schema_version: STREAM_SCHEMA_VERSION,
            payer: payer.clone(),
            recipient,
            token: Address::generate(&env),
            rate_per_second: 0,
            balance: total_amount,
            claimable_balance: 0,
            start_time: 0,
            end_time: 0,
            is_active: false,
            paused_at: 0,
            memo: String::from_str(&env, ""),
            recipient_can_stop: false,
            mode: StreamMode::LinearVesting {
                duration_seconds,
                vested_amount: 0,
                schedule_anchor: 0,
            },
        };

        set_stream(&env, stream_id, &info);
        set_next_stream_id(&env, stream_id.wrapping_add(1));
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
        if info.end_time > 0 && info.end_time <= now {
            panic!("end_time must be in the future");
        }

        if let StreamMode::LinearVesting {
            schedule_anchor, ..
        } = &mut info.mode
        {
            if *schedule_anchor == 0 {
                *schedule_anchor = now;
            }
        }

        info.is_active = true;
        info.start_time = now;
        info.paused_at = 0;
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Stop an active stream. `stopper` must be the payer, or the recipient when
    /// `recipient_can_stop` was set at creation.
    pub fn stop_stream(env: Env, stream_id: u32, stopper: Address) {
        let mut info = get_stream(&env, stream_id);
        if !info.is_active {
            panic!("stream not active");
        }

        if stopper == info.payer {
            stopper.require_auth();
        } else if stopper == info.recipient && info.recipient_can_stop {
            stopper.require_auth();
        } else {
            panic!("unauthorized stopper");
        }

        info.is_active = false;
        info.end_time = env.ledger().timestamp();
        info.paused_at = 0;
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Settle stream: move accrued tokens to `claimable_balance`.
    pub fn settle_stream(env: Env, stream_id: u32) -> i128 {
        match settle_stream_amount(&env, stream_id) {
            None => 0,
            Some(amount) => {
                if amount > 0 {
                    extend_instance_ttl(&env);
                }
                amount
            }
        }
    }

    /// Settle multiple streams in a single invocation (all-or-nothing).
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

    pub fn max_batch_settle_size(_env: Env) -> u32 {
        MAX_BATCH_SETTLE_SIZE
    }

    /// Read-only view of accrued amount without mutating state.
    pub fn accrued_amount(env: Env, stream_id: u32) -> i128 {
        let info = get_stream(&env, stream_id);
        if !info.is_active {
            return 0;
        }
        let settle_until = settlement_boundary(&info, env.ledger().timestamp());
        compute_accrual(&info, settle_until)
    }

    pub fn cancel_stream(env: Env, stream_id: u32) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if !info.is_active {
            panic!("cannot cancel inactive stream");
        }

        let now = env.ledger().timestamp();
        let accrued = compute_accrual(&info, now);
        apply_settlement(&mut info, accrued, now);
        info.is_active = false;
        info.end_time = now;
        info.paused_at = 0;

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

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
        let accrued = compute_accrual(&info, now);
        apply_settlement(&mut info, accrued, now);
        info.paused_at = now;

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

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
        info.start_time = now;
        info.paused_at = 0;

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }

    /// Withdraw all claimable tokens to the recipient (recipient auth required).
    pub fn withdraw_stream(env: Env, stream_id: u32) -> i128 {
        let mut info = get_stream(&env, stream_id);
        info.recipient.require_auth();

        let now = env.ledger().timestamp();
        let settle_until = if info.is_active {
            settlement_boundary(&info, now)
        } else {
            info.end_time
        };

        if settle_until > info.start_time {
            let accrued = compute_accrual(&info, settle_until);
            apply_settlement(&mut info, accrued, settle_until);
        }

        let claimable = info.claimable_balance;
        let token_addr = info.token.clone();
        let recipient = info.recipient.clone();
        info.claimable_balance = 0;

        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);

        if claimable > 0 {
            token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &recipient,
                &claimable,
            );
        }

        claimable
    }

    pub fn is_stream_active(env: Env, stream_id: u32) -> bool {
        get_stream(&env, stream_id).is_active
    }

    pub fn get_stream_info(env: Env, stream_id: u32) -> StreamInfo {
        get_stream(&env, stream_id)
    }

    pub fn version(_env: Env) -> u32 {
        VERSION
    }

    pub fn archive_stream(env: Env, stream_id: u32) {
        let info = get_stream(&env, stream_id);
        info.payer.require_auth();
        if info.is_active {
            panic!("cannot archive active stream");
        }
        if info.balance != 0 {
            panic!("cannot archive stream with unsettled balance");
        }
        if info.claimable_balance != 0 {
            panic!("cannot archive stream with unclaimed balance");
        }
        let key = stream_key(&env, stream_id);
        env.storage().persistent().remove(&key);
        extend_instance_ttl(&env);
    }

    /// Update the rate of an existing stream (payer-only).
    ///
    /// Rate increases are capped at 10% above the **current** rate per call;
    /// repeated calls compound (e.g. 100 → 110 → 121).
    pub fn update_rate(env: Env, stream_id: u32, new_rate: i128) {
        let mut info = get_stream(&env, stream_id);
        info.payer.require_auth();

        if new_rate <= 0 {
            panic!("rate must be positive");
        }

        let old_rate = info.rate_per_second;
        let max_allowed_rate = old_rate + (old_rate / 10);
        if new_rate > max_allowed_rate {
            panic!("rate increase exceeds 10% limit");
        }

        if info.is_active {
            let now = env.ledger().timestamp();
            let settle_until = settlement_boundary(&info, now);
            let accrued = compute_accrual(&info, settle_until);
            apply_settlement(&mut info, accrued, settle_until);
        }

        info.rate_per_second = new_rate;
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
    }
}

/// Central settlement helper used by `settle_stream` and `batch_settle`.
///
/// Returns `None` when the stream is inactive (callers map that to `0`).
/// Returns `Some(accrued)` for active streams, including `Some(0)` when
/// `elapsed == 0`.
///
/// See `docs/accrual-spec.md` §3–§4.
fn settle_stream_amount(env: &Env, stream_id: u32) -> Option<i128> {
    let mut info = get_stream(env, stream_id);
    if !info.is_active {
        return None;
    }

    let now = env.ledger().timestamp();
    let settle_until = settlement_boundary(&info, now);
    if settle_until <= info.start_time {
        return Some(0);
    }

    let accrued = compute_accrual(&info, settle_until);
    if accrued == 0 {
        return Some(0);
    }

    apply_settlement(&mut info, accrued, settle_until);

    if info.end_time > 0 && settle_until >= info.end_time {
        info.is_active = false;
    }

    set_stream(env, stream_id, &info);
    extend_stream_ttl(env, stream_id);
    Some(accrued)
}

fn settlement_boundary(info: &StreamInfo, now: u64) -> u64 {
    if info.paused_at > 0 {
        info.paused_at
    } else if info.end_time > 0 && info.end_time <= now {
        info.end_time
    } else {
        now
    }
}

fn compute_accrual(info: &StreamInfo, settle_until: u64) -> i128 {
    if settle_until <= info.start_time {
        return 0;
    }

    match &info.mode {
        StreamMode::Linear => {
            let elapsed = settle_until - info.start_time;
            (elapsed as i128)
                .saturating_mul(info.rate_per_second)
                .min(info.balance)
        }
        StreamMode::LinearVesting {
            duration_seconds,
            vested_amount,
            schedule_anchor,
        } => {
            if *schedule_anchor == 0 {
                return 0;
            }
            let elapsed = settle_until.saturating_sub(*schedule_anchor);
            let total_amount = info.balance.saturating_add(*vested_amount);
            let target = compute_linear_vested(total_amount, *duration_seconds, elapsed);
            target.saturating_sub(*vested_amount).min(info.balance)
        }
    }
}

fn apply_settlement(info: &mut StreamInfo, accrued: i128, settle_until: u64) {
    if accrued <= 0 {
        return;
    }
    info.balance = info.balance.saturating_sub(accrued);
    info.claimable_balance = info.claimable_balance.saturating_add(accrued);
    info.start_time = settle_until;

    if let StreamMode::LinearVesting { vested_amount, .. } = &mut info.mode {
        *vested_amount = vested_amount.saturating_add(accrued);
    }
}

fn compute_linear_vested(total_amount: i128, duration_seconds: u64, elapsed_seconds: u64) -> i128 {
    let capped_elapsed = if elapsed_seconds > duration_seconds {
        duration_seconds
    } else {
        elapsed_seconds
    };
    total_amount.saturating_mul(capped_elapsed as i128) / duration_seconds as i128
}

fn get_next_stream_id(env: &Env) -> u32 {
    let key = Symbol::new(env, "next_id");
    env.storage().instance().get(&key).unwrap_or(1)
}

fn set_next_stream_id(env: &Env, id: u32) {
    let key = Symbol::new(env, "next_id");
    env.storage().instance().set(&key, &id);
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

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

#[cfg(test)]
mod test {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::{token, Address, Env, IntoVal, String};

    use super::*;
    use stream::STREAM_SCHEMA_VERSION;

    fn advance_ledger_time(env: &Env, seconds: u64) {
        env.ledger().with_mut(|li| {
            li.timestamp += seconds;
        });
    }

    fn setup_token_env() -> (Env, Address, token::Client<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin);
        let token_client = token::Client::new(&env, &token_addr);
        (env, contract_id, token_client, token_addr)
    }

    fn mint_and_create(
        env: &Env,
        client: &StreamPayContractClient,
        token_client: &token::Client,
        token_addr: &Address,
        payer: &Address,
        recipient: &Address,
        rate: i128,
        balance: i128,
    ) -> u32 {
        token_client.mint(payer, &balance);
        client.create_stream(payer, recipient, token_addr, &rate, &balance, &false)
    }

    fn create_simple_stream(
        env: &Env,
        client: &StreamPayContractClient,
        payer: &Address,
        recipient: &Address,
        rate: i128,
        balance: i128,
        recipient_can_stop: bool,
    ) -> u32 {
        let admin = Address::generate(env);
        let token_addr = env.register_stellar_asset_contract(admin);
        let token_client = token::Client::new(env, &token_addr);
        token_client.mint(payer, &balance);
        client.create_stream(
            payer,
            recipient,
            &token_addr,
            &rate,
            &balance,
            &recipient_can_stop,
        )
    }

    #[test]
    fn test_create_stream_valid() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 100, 10_000,
        );
        assert_eq!(stream_id, 1);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.payer, payer);
        assert_eq!(info.recipient, recipient);
        assert_eq!(info.token, token_addr);
        assert_eq!(info.rate_per_second, 100);
        assert_eq!(info.balance, 10_000);
        assert_eq!(info.claimable_balance, 0);
        assert!(!info.is_active);
        assert_eq!(info.schema_version, STREAM_SCHEMA_VERSION);
    }

    #[test]
    fn test_create_stream_emits_event() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 100, 10_000,
        );

        let events = env.events().all();
        assert_eq!(events.len(), 1);
        let (emitting_contract, topics, data) = events.get(0).unwrap();
        assert_eq!(emitting_contract, contract_id);
        let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
        let topic1: u32 = soroban_sdk::FromVal::from_val(&env, &topics.get(1).unwrap());
        assert_eq!(topic0, Symbol::new(&env, "stream_created"));
        assert_eq!(topic1, stream_id);
        let event_data: StreamCreatedEvent = soroban_sdk::FromVal::from_val(&env, &data);
        assert_eq!(event_data.payer, payer);
        assert_eq!(event_data.recipient, recipient);
        assert_eq!(event_data.rate_per_second, 100);
        assert_eq!(event_data.initial_balance, 10_000);
    }

    #[test]
    fn test_streaminfo_round_trip_persists_all_fields() {
        let env = Env::default();
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_addr = Address::generate(&env);
        let info = StreamInfo {
            schema_version: STREAM_SCHEMA_VERSION,
            payer: payer.clone(),
            recipient: recipient.clone(),
            token: token_addr.clone(),
            rate_per_second: 42,
            balance: 9_999,
            claimable_balance: 7,
            start_time: 11,
            end_time: 22,
            is_active: true,
            paused_at: 0,
            memo: String::from_str(&env, "memo"),
            recipient_can_stop: true,
            mode: StreamMode::Linear,
        };
        set_stream(&env, 99, &info);
        let loaded = get_stream(&env, 99);
        assert_eq!(loaded, info);
    }

    #[test]
    fn test_create_stream_allows_last_u32_id() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        set_next_stream_id(&env, u32::MAX);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        token_client.mint(&payer, &10_000);
        let stream_id =
            client.create_stream(&payer, &recipient, &token_addr, &100, &10_000, &false);
        assert_eq!(stream_id, u32::MAX);
        assert_eq!(get_next_stream_id(&env), 0);
    }

    #[test]
    #[should_panic(expected = "stream id overflow")]
    fn test_create_stream_panics_when_id_would_overflow() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        set_next_stream_id(&env, 0);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        token_client.mint(&payer, &10_000);
        client.create_stream(&payer, &recipient, &token_addr, &100, &10_000, &false);
    }

    #[test]
    fn test_start_and_stop_stream() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 50, 5_000,
        );
        client.start_stream(&stream_id);
        assert!(client.get_stream_info(&stream_id).is_active);
        client.stop_stream(&stream_id, &payer);
        assert!(!client.get_stream_info(&stream_id).is_active);
    }

    #[test]
    #[should_panic(expected = "stream not active")]
    fn test_stop_stream_inactive_panics() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 50, 5_000,
        );
        client.stop_stream(&stream_id, &payer);
    }

    #[test]
    #[should_panic(expected = "stream not found")]
    fn test_stop_stream_missing_id_panics() {
        let (env, contract_id, _, _) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        client.stop_stream(&999_u32, &payer);
    }

    #[test]
    fn test_stop_stream_requires_payer_auth() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 50, 5_000,
        );
        client.start_stream(&stream_id);
        env.set_auths(&[]);
        let result = catch_unwind(AssertUnwindSafe(|| client.stop_stream(&stream_id, &payer)));
        assert!(result.is_err());
        assert!(client.get_stream_info(&stream_id).is_active);
    }

    #[test]
    fn test_settle_returns_amount() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 1_000,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        assert_eq!(client.settle_stream(&stream_id), 100);
    }

    #[test]
    fn test_settle_inactive_returns_zero() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 1_000,
        );
        assert_eq!(client.settle_stream(&stream_id), 0);
    }

    #[test]
    fn test_settle_zero_elapsed_returns_zero() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 1_000,
        );
        client.start_stream(&stream_id);
        assert_eq!(client.settle_stream(&stream_id), 0);
        assert_eq!(client.get_stream_info(&stream_id).balance, 1_000);
    }

    #[test]
    fn test_settle_moves_to_claimable() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 1_000,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.settle_stream(&stream_id);
        assert_eq!(client.get_stream_info(&stream_id).claimable_balance, 100);
    }

    #[test]
    fn test_accrued_amount_capped_by_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 20);
        assert_eq!(client.accrued_amount(&stream_id), 1_000);
    }

    #[test]
    fn test_accrued_amount_inactive_returns_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 1_000, false,
        );
        assert_eq!(client.accrued_amount(&stream_id), 0);
    }

    #[test]
    fn test_accrued_amount_view_matches_settle_same_second() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        client.start_stream(&stream_id);
        assert_eq!(client.accrued_amount(&stream_id), 0);
        assert_eq!(client.settle_stream(&stream_id), 0);
    }

    #[test]
    fn test_accrual_balance_cap_fires() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 20);
        assert_eq!(client.settle_stream(&stream_id), 1_000);
        assert_eq!(client.get_stream_info(&stream_id).balance, 0);
    }

    #[test]
    fn test_accrual_saturating_multiply_at_i128_max() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env,
            &client,
            &payer,
            &recipient,
            i128::MAX,
            500,
            false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        let amount = client.settle_stream(&stream_id);
        assert!(amount >= 0);
        assert!(amount <= 500);
        assert_eq!(amount, 500);
        assert_eq!(client.get_stream_info(&stream_id).balance, 0);
    }

    #[test]
    fn test_accrual_astronomical_elapsed_no_panic() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, u64::MAX / 2);
        let amount = client.settle_stream(&stream_id);
        assert_eq!(amount, 1_000);
    }

    #[test]
    fn test_accrual_sequential_drain_then_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 50);
        assert_eq!(client.settle_stream(&stream_id), 500);
        advance_ledger_time(&env, 50);
        assert_eq!(client.settle_stream(&stream_id), 500);
        assert_eq!(client.get_stream_info(&stream_id).balance, 0);
        assert_eq!(client.settle_stream(&stream_id), 0);
    }

    #[test]
    fn test_batch_settle_empty_vec() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let amounts = client.batch_settle(&Vec::new(&env));
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
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        let mut ids = Vec::new(&env);
        ids.push_back(stream_id);
        let amounts = client.batch_settle(&ids);
        assert_eq!(amounts.get(0).unwrap(), 0);
    }

    #[test]
    fn test_batch_settle_single_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        let mut ids = Vec::new(&env);
        ids.push_back(stream_id);
        assert_eq!(client.batch_settle(&ids).get(0).unwrap(), 100);
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
        let admin = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin);
        let token_client = token::Client::new(&env, &token_addr);
        token_client.mint(&payer, &2_000);
        let first = client.create_stream(&payer, &recipient_a, &token_addr, &10, &1_000, &false);
        let second = client.create_stream(&payer, &recipient_b, &token_addr, &5, &1_000, &false);
        client.start_stream(&first);
        client.start_stream(&second);
        advance_ledger_time(&env, 10);
        let mut ids = Vec::new(&env);
        ids.push_back(first);
        ids.push_back(second);
        let amounts = client.batch_settle(&ids);
        assert_eq!(amounts.get(0).unwrap(), 100);
        assert_eq!(amounts.get(1).unwrap(), 50);
    }

    #[test]
    fn test_batch_settle_missing_id_reverts_all() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        let original = client.get_stream_info(&stream_id);
        let mut ids = Vec::new(&env);
        ids.push_back(stream_id);
        ids.push_back(999_u32);
        let result = catch_unwind(AssertUnwindSafe(|| client.batch_settle(&ids)));
        assert!(result.is_err());
        let after = client.get_stream_info(&stream_id);
        assert_eq!(after.balance, original.balance);
    }

    #[test]
    #[should_panic(expected = "batch too large")]
    fn test_batch_settle_too_large_panics() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let mut ids = Vec::new(&env);
        for id in 1..=(MAX_BATCH_SETTLE_SIZE + 1) {
            ids.push_back(id);
        }
        client.batch_settle(&ids);
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
        advance_ledger_time(&env, 10);
        assert_eq!(client.settle_stream(&stream_id), 100);
        advance_ledger_time(&env, 40);
        assert_eq!(client.settle_stream(&stream_id), 400);
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
        advance_ledger_time(&env, 1);
        assert_eq!(client.settle_stream(&stream_id), 333);
        advance_ledger_time(&env, 1);
        assert_eq!(client.settle_stream(&stream_id), 333);
        advance_ledger_time(&env, 1);
        assert_eq!(client.settle_stream(&stream_id), 334);
        assert_eq!(client.get_stream_info(&stream_id).balance, 0);
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
        advance_ledger_time(&env, 10);
        client.stop_stream(&stream_id, &payer);
        advance_ledger_time(&env, 20);
        client.start_stream(&stream_id);
        assert_eq!(client.settle_stream(&stream_id), 300);
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
        advance_ledger_time(&env, 15);
        assert_eq!(client.settle_stream(&stream_id), 1_000);
        assert_eq!(client.get_stream_info(&stream_id).balance, 0);
    }

    #[test]
    fn test_vesting_duration_one_second() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_vesting_stream(&payer, &recipient, &100_i128, &1_u64);
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 1);
        assert_eq!(client.settle_stream(&stream_id), 100);
    }

    #[test]
    fn test_withdraw_double_claim_idempotent() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 500,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.stop_stream(&stream_id, &payer);
        let first = client.withdraw_stream(&stream_id);
        assert_eq!(first, 100);
        assert_eq!(client.get_stream_info(&stream_id).claimable_balance, 0);
        assert_eq!(client.withdraw_stream(&stream_id), 0);
        assert_eq!(token_client.balance(&recipient), 100);
    }

    #[test]
    fn test_withdraw_stop_without_settle_claims_earnings() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 500,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.stop_stream(&stream_id, &payer);
        assert_eq!(client.withdraw_stream(&stream_id), 100);
        assert_eq!(client.withdraw_stream(&stream_id), 0);
    }

    #[test]
    fn test_withdraw_requires_recipient_auth() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 500,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 5);
        client.stop_stream(&stream_id, &payer);
        env.set_auths(&[]);
        let result = catch_unwind(AssertUnwindSafe(|| client.withdraw_stream(&stream_id)));
        assert!(result.is_err());
    }

    #[test]
    fn test_third_party_cannot_withdraw() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stranger = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 10, 500,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 5);
        client.stop_stream(&stream_id, &payer);
        env.set_auths(&[]);
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &stranger,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_stream",
                args: (stream_id,).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let result = catch_unwind(AssertUnwindSafe(|| client.withdraw_stream(&stream_id)));
        assert!(result.is_err());
    }

    #[test]
    fn test_update_rate_compounds_per_call() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        client.update_rate(&stream_id, &110);
        client.update_rate(&stream_id, &121);
        assert_eq!(client.get_stream_info(&stream_id).rate_per_second, 121);
    }

    #[test]
    fn test_update_rate_inactive_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        client.update_rate(&stream_id, &80);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 80);
        assert_eq!(info.balance, 10_000);
    }

    #[test]
    fn test_update_rate_active_stream_settles_first() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.update_rate(&stream_id, &50);
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.rate_per_second, 50);
        assert_eq!(info.balance, 9_000);
    }

    #[test]
    #[should_panic(expected = "rate increase exceeds 10% limit")]
    fn test_update_rate_large_increase_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        client.update_rate(&stream_id, &120);
    }

    #[test]
    fn test_recipient_can_stop_when_flag_set() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 50, 5_000, true,
        );
        client.start_stream(&stream_id);
        client.stop_stream(&stream_id, &recipient);
        assert!(!client.get_stream_info(&stream_id).is_active);
    }

    #[test]
    #[should_panic(expected = "unauthorized stopper")]
    fn test_recipient_cannot_stop_when_flag_false() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 50, 5_000, false,
        );
        client.start_stream(&stream_id);
        client.stop_stream(&stream_id, &recipient);
    }

    #[test]
    fn test_archive_settled_stream() {
        let (env, contract_id, token_client, token_addr) = setup_token_env();
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = mint_and_create(
            &env, &client, &token_client, &token_addr, &payer, &recipient, 100, 1_000,
        );
        client.start_stream(&stream_id);
        advance_ledger_time(&env, 10);
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id, &payer);
        client.withdraw_stream(&stream_id);
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
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        client.archive_stream(&stream_id);
    }

    #[test]
    fn test_settle_stopped_stream_returns_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 10, 1_000, false,
        );
        client.start_stream(&stream_id);
        client.stop_stream(&stream_id, &payer);
        assert_eq!(client.settle_stream(&stream_id), 0);
    }

    #[test]
    fn test_stream_schema_version_is_positive() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        assert!(client.get_stream_info(&stream_id).schema_version > 0);
    }

    #[test]
    fn test_stream_schema_version_is_current() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = create_simple_stream(
            &env, &client, &payer, &recipient, 100, 10_000, false,
        );
        assert_eq!(
            client.get_stream_info(&stream_id).schema_version,
            STREAM_SCHEMA_VERSION
        );
    }

    #[test]
    fn test_version_returns_expected() {
        let env = Env::default();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        assert_eq!(client.version(), 2_000);
    }
}
