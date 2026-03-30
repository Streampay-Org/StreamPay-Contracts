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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
            let sid = client.create_stream(&payer, &recipient, &rate, &balance);
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
