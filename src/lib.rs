//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! # Public entry points
//!
//! | Function           | Auth      | Description                                         |
//! |--------------------|-----------|-----------------------------------------------------|
//! | `create_stream`    | payer     | Escrow tokens and register a new stream             |
//! | `start_stream`     | payer     | Begin accrual                                       |
//! | `stop_stream`      | payer     | Pause accrual                                       |
//! | `settle_stream`    | none      | Move accrued tokens to the claimable pool           |
//! | `withdraw_stream`  | recipient | Transfer all claimable tokens on-ledger             |
//! | `archive_stream`   | payer     | Remove a fully-settled, fully-withdrawn stream      |
//! | `get_stream_info`  | none      | Read-only stream metadata                           |
//! | `version`          | none      | Contract version constant                           |
//!
//! # Accounting invariant
//!
//! For every stream at every point in time:
//!
//! ```text
//! balance + claimable_balance ≤ initial_deposit
//! ```
//!
//! * `balance`           — tokens held in escrow, not yet earned by the recipient.
//! * `claimable_balance` — tokens earned by the recipient, not yet transferred.
//!
//! `settle_stream` (permissionless) advances accounting:
//! `balance -= accrued`, `claimable_balance += accrued`.
//!
//! `withdraw_stream` (recipient-auth) performs the actual on-ledger transfer
//! of `claimable_balance` to the recipient and resets it to zero.
//! It implicitly settles any outstanding accrual first.
//!
//! Calling `withdraw_stream` when nothing is claimable is safe and returns 0
//! (idempotent — safe to call multiple times).

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};

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

/// All state associated with a payment stream.
///
/// # Balance accounting
///
/// `balance` and `claimable_balance` together track all escrowed funds:
///
/// * `balance`           — not yet earned; decreases each settlement period.
/// * `claimable_balance` — earned but not yet transferred to the recipient;
///                         increases on settle, resets to 0 on withdraw.
///
/// Both are non-negative. Their sum never exceeds the `initial_balance`
/// passed to `create_stream`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub payer: Address,
    pub recipient: Address,
    /// SEP-41-compatible token contract used for this stream.
    pub token: Address,
    /// Streaming rate in token units per second.
    pub rate_per_second: i128,
    /// Tokens still held in escrow; not yet accrued to the recipient.
    pub balance: i128,
    /// Tokens accrued by the recipient but not yet withdrawn on-ledger.
    pub claimable_balance: i128,
    /// Timestamp of the last settlement (or stream start if never settled).
    pub start_time: u64,
    /// Timestamp when the stream was stopped (0 if never stopped).
    pub end_time: u64,
    pub is_active: bool,
}

#[contract]
pub struct StreamPayContract;

#[contractimpl]
impl StreamPayContract {
    /// Create a new payment stream and immediately escrow `initial_balance` tokens.
    ///
    /// `initial_balance` tokens are transferred from `payer` into this contract
    /// on creation, so `payer` must hold at least `initial_balance` of `token`
    /// and must authorise both this call and the token transfer.
    ///
    /// Returns the new `stream_id`.
    pub fn create_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        token: Address,
        rate_per_second: i128,
        initial_balance: i128,
    ) -> u32 {
        payer.require_auth();
        if rate_per_second <= 0 || initial_balance <= 0 {
            panic!("rate and balance must be positive");
        }

        // Escrow initial_balance tokens from payer into this contract.
        token::Client::new(&env, &token)
            .transfer(&payer, &env.current_contract_address(), &initial_balance);

        let stream_id = get_next_stream_id(&env);
        let info = StreamInfo {
            payer: payer.clone(),
            recipient,
            token,
            rate_per_second,
            balance: initial_balance,
            claimable_balance: 0,
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

    /// Activate a stream and begin accrual (payer only).
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

    /// Pause an active stream (payer only).
    ///
    /// Records `end_time` so that `withdraw_stream` can later settle the
    /// final accrual period even after the stream becomes inactive.
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

    /// Advance accounting: move elapsed accrual from `balance` into
    /// `claimable_balance` (permissionless).
    ///
    /// Returns the amount accrued this call. Returns 0 for inactive streams.
    /// Each call only covers elapsed time since the last settlement,
    /// preventing double-counting.
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
        info.claimable_balance = info.claimable_balance.saturating_add(amount);
        info.start_time = now;
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);
        amount
    }

    /// Withdraw all claimable tokens to the recipient (recipient auth required).
    ///
    /// Implicitly settles any outstanding accrual before transferring, so the
    /// recipient does not need to call `settle_stream` first.
    ///
    /// For inactive streams, settles the final period between the last
    /// settlement timestamp (`start_time`) and the stop timestamp (`end_time`),
    /// ensuring no earned tokens are stranded when the payer stops the stream
    /// without a prior `settle_stream` call.
    ///
    /// Returns the amount transferred on-ledger. Returns 0 if nothing is
    /// claimable (idempotent — safe to call multiple times).
    ///
    /// # Interaction with internal accounting
    ///
    /// After a successful withdrawal:
    /// * `claimable_balance` is reset to 0.
    /// * `balance` reflects only the un-accrued remainder.
    /// * `start_time` is advanced to the settlement boundary, preventing
    ///   the same elapsed period from being double-counted on a future call.
    pub fn withdraw_stream(env: Env, stream_id: u32) -> i128 {
        let mut info = get_stream(&env, stream_id);
        info.recipient.require_auth();

        // Determine the settlement boundary:
        //   active  → settle up to now
        //   stopped → settle the gap [start_time, end_time]  (end_time set by stop_stream)
        //   never started (end_time == 0, is_active == false) → settle_until == 0, no-op
        let settle_until: u64 = if info.is_active {
            env.ledger().timestamp()
        } else {
            info.end_time
        };

        if settle_until > info.start_time {
            let elapsed = settle_until - info.start_time;
            let accrued = (elapsed as i128)
                .saturating_mul(info.rate_per_second)
                .min(info.balance);
            info.balance = info.balance.saturating_sub(accrued);
            info.claimable_balance = info.claimable_balance.saturating_add(accrued);
            // Advance start_time so the same period is never settled twice.
            info.start_time = settle_until;
        }

        let claimable = info.claimable_balance;
        info.claimable_balance = 0;

        // Write state before the external token call (checks-effects-interactions).
        set_stream(&env, stream_id, &info);
        extend_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);

        if claimable > 0 {
            // Transfer tokens from this contract to the recipient.
            // The contract is the sender, so no additional auth is required.
            token::Client::new(&env, &info.token)
                .transfer(&env.current_contract_address(), &info.recipient, &claimable);
        }

        claimable
    }

    /// Get stream info (read-only).
    pub fn get_stream_info(env: Env, stream_id: u32) -> StreamInfo {
        get_stream(&env, stream_id)
    }

    /// Returns the contract version as a u32 (see VERSION encoding).
    pub fn version(_env: Env) -> u32 {
        VERSION
    }

    /// Archive (remove) a fully-settled, fully-withdrawn, inactive stream (payer only).
    ///
    /// Requires `is_active == false`, `balance == 0`, and `claimable_balance == 0`.
    /// The `claimable_balance == 0` guard ensures the recipient has received all
    /// owed tokens before the stream record is permanently deleted.
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
    use soroban_sdk::token;

    use super::*;

    /// Deploy a Stellar asset contract, mint `amount` to `account`, and return
    /// the token contract address.
    fn setup_token(env: &Env, admin: &Address, account: &Address, amount: i128) -> Address {
        let token_address = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        token::StellarAssetClient::new(env, &token_address).mint(account, &amount);
        token_address
    }

    // -------------------------------------------------------------------------
    // Core lifecycle tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_create_stream_valid() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 10_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &10_000_i128);
        assert_eq!(stream_id, 1);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.payer, payer);
        assert_eq!(info.recipient, recipient);
        assert_eq!(info.token, token_address);
        assert_eq!(info.rate_per_second, 100);
        assert_eq!(info.balance, 10_000);
        assert_eq!(info.claimable_balance, 0);
        assert!(!info.is_active);
    }

    #[test]
    fn test_start_and_stop_stream() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 5_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &50_i128, &5_000_i128);
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

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &1_000_i128);
        client.start_stream(&stream_id);
        let amount = client.settle_stream(&stream_id);
        assert!(amount >= 0);
    }

    #[test]
    fn test_settle_moves_to_claimable() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &1_000_i128);
        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        let settled = client.settle_stream(&stream_id);
        assert_eq!(settled, 50);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 950);
        assert_eq!(info.claimable_balance, 50); // accrued, awaiting withdrawal
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

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 10_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &10_000_i128);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 10_000);
    }

    #[test]
    fn test_create_stream_extends_ttl() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 10_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &10_000_i128);

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

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        // rate=100/s, balance=1000 → fully drained after 10s
        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
        let settled = client.settle_stream(&stream_id);
        assert_eq!(settled, 1_000);

        client.stop_stream(&stream_id);

        // Withdraw clears claimable_balance so archive can proceed.
        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 1_000);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 0);
        assert_eq!(info.claimable_balance, 0);
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

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 10_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &10_000_i128);

        // Stream is inactive but balance > 0 — panics to protect recipient.
        client.archive_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_archive_active_stream_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 10_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &10_000_i128);
        client.start_stream(&stream_id);

        // Panics — stream is still active.
        client.archive_stream(&stream_id);
    }

    #[test]
    #[should_panic]
    fn test_archived_stream_not_found() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id);
        client.withdraw_stream(&stream_id); // clear claimable before archive
        client.archive_stream(&stream_id);

        // Panics — stream was archived.
        client.get_stream_info(&stream_id);
    }

    // -------------------------------------------------------------------------
    // Withdraw tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_withdraw_full_balance_drain() {
        // rate=100/s, balance=1000, advance 10s → fully drained.
        // withdraw_stream should return 1000 and transfer on-ledger.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &100_i128, &1_000_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10;
        });

        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 1_000);

        // Verify on-ledger token balances.
        let tok = token::Client::new(&env, &token_address);
        assert_eq!(tok.balance(&recipient), 1_000);
        assert_eq!(tok.balance(&contract_id), 0);

        // claimable_balance resets to 0 after withdrawal.
        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.claimable_balance, 0);
        assert_eq!(info.balance, 0);
    }

    #[test]
    fn test_withdraw_partial_accrual() {
        // rate=10/s, balance=1_000, advance 5s → 50 accrued.
        // Remaining 950 stays escrowed in balance.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &1_000_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 50);

        let tok = token::Client::new(&env, &token_address);
        assert_eq!(tok.balance(&recipient), 50);
        assert_eq!(tok.balance(&contract_id), 950);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.balance, 950);
        assert_eq!(info.claimable_balance, 0);
        assert!(info.is_active); // stream still running
    }

    #[test]
    fn test_withdraw_inactive_stream() {
        // rate=10/s, balance=200, advance 5s, stop WITHOUT settle_stream first.
        // withdraw_stream must still capture the 50 tokens accrued during
        // the active period via its implicit settle-on-stop logic.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 200);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &200_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });
        // Payer stops stream without an explicit settle first.
        client.stop_stream(&stream_id);

        // Recipient withdraws; implicit settle covers [start_time, end_time].
        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 50);

        let tok = token::Client::new(&env, &token_address);
        assert_eq!(tok.balance(&recipient), 50);
        assert_eq!(tok.balance(&contract_id), 150); // 150 remains escrowed

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.claimable_balance, 0);
        assert_eq!(info.balance, 150);
    }

    #[test]
    fn test_withdraw_double_claim_idempotent() {
        // First withdraw claims all accrued tokens. A second call with no
        // new accrual (stream is stopped) must return 0 and leave balances
        // unchanged — double-claim must never transfer more than owed.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 500);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &500_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 10; // accrues 100
        });
        client.stop_stream(&stream_id);

        let first = client.withdraw_stream(&stream_id);
        assert_eq!(first, 100);

        // Second call: stream stopped, claimable=0, start_time==end_time → 0.
        let second = client.withdraw_stream(&stream_id);
        assert_eq!(second, 0);

        let tok = token::Client::new(&env, &token_address);
        assert_eq!(tok.balance(&recipient), 100); // unchanged after second call
    }

    #[test]
    fn test_withdraw_after_explicit_settle() {
        // Explicit settle_stream followed by withdraw_stream:
        // settle moves 50 → claimable_balance; withdraw transfers 50 on-ledger.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &1_000_i128);
        client.start_stream(&stream_id);

        env.ledger().with_mut(|li| {
            li.timestamp += 5;
        });

        // Keeper settles accounting; start_time advances to now.
        let settled = client.settle_stream(&stream_id);
        assert_eq!(settled, 50);

        // Recipient withdraws; no additional accrual since settle just ran.
        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 50);

        let tok = token::Client::new(&env, &token_address);
        assert_eq!(tok.balance(&recipient), 50);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.claimable_balance, 0);
        assert_eq!(info.balance, 950);
    }

    #[test]
    fn test_withdraw_never_started_stream_returns_zero() {
        // Calling withdraw on a stream that was created but never started
        // must return 0 and not panic.
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = setup_token(&env, &admin, &payer, 1_000);

        let stream_id =
            client.create_stream(&payer, &recipient, &token_address, &10_i128, &1_000_i128);

        let withdrawn = client.withdraw_stream(&stream_id);
        assert_eq!(withdrawn, 0);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(info.claimable_balance, 0);
        assert_eq!(info.balance, 1_000); // escrowed funds untouched
    }
}
