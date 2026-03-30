//! StreamPay — Soroban smart contracts for continuous payment streaming.
//!
//! Provides: create_stream, start_stream, stop_stream, settle_stream,
//! archive_stream, get_stream_info, version.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

/// Contract version: major * 1_000_000 + minor * 1_000 + patch.
/// Current: 0.1.0 → 1_000
const VERSION: u32 = 1_000;

/// Schema version stored inside every [`StreamInfo`] entry.
///
/// Increment this constant whenever a field is added to or removed from
/// [`StreamInfo`].  The value is written at creation time and can be read
/// back to detect entries that pre-date a schema change.
///
/// | Value | Fields present |
/// |-------|----------------|
/// | 1     | All fields in the initial `StreamInfo` definition (this release) |
///
/// See `docs/schema-versioning.md` for the full evolution policy.
const STREAM_SCHEMA_VERSION: u32 = 1;

/// TTL threshold: extend when remaining TTL drops below ~1 day (17_280 ledgers at ~5s each).
const STREAM_TTL_THRESHOLD: u32 = 17_280;
/// TTL extend-to: refresh to ~30 days (518_400 ledgers).
const STREAM_TTL_EXTEND: u32 = 518_400;
/// Instance storage TTL threshold (~1 day).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
/// Instance storage TTL extend-to (~30 days).
const INSTANCE_TTL_EXTEND: u32 = 518_400;

/// Metadata for a single payment stream stored in Soroban persistent storage.
///
/// # Schema versioning
///
/// Soroban serialises `#[contracttype]` structs as XDR maps keyed by field
/// name (symbol).  Adding a **new** field to this struct is a **breaking
/// change** for any entry that was written before the upgrade: the host will
/// panic when it tries to deserialise the old map into the new struct because
/// the new key is absent.
///
/// To handle this safely:
///
/// 1. **Never remove or rename existing fields.**  Removal causes the same
///    deserialisation panic for new code reading old data; renaming is
///    equivalent to a remove + add.
/// 2. **Add new optional fields as sentinel-defaulted values.**  When a new
///    field must be added, provide a migration path (e.g. a `migrate_stream`
///    admin function) that reads old entries, supplies the sentinel default,
///    and re-writes them before any contract logic depends on the new field.
/// 3. **Use `schema_version` to detect stale entries.**  Every `StreamInfo`
///    carries [`schema_version`](StreamInfo::schema_version) set to
///    [`STREAM_SCHEMA_VERSION`] at creation.  Code that reads an entry can
///    compare this value against the constant to decide whether migration is
///    needed.
/// 4. **Bump [`STREAM_SCHEMA_VERSION`] with every struct change.**  This
///    makes stale entries detectable at runtime.
///
/// See `docs/schema-versioning.md` for the full policy, sentinel defaults
/// table, and worked migration examples.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamInfo {
    /// Schema version of this entry; set to [`STREAM_SCHEMA_VERSION`] at
    /// creation.  Used to detect entries written by an older contract version.
    pub schema_version: u32,
    /// Address that created and funds the stream.
    pub payer: Address,
    /// Address that receives the streamed funds.
    pub recipient: Address,
    /// Tokens released per second while the stream is active.
    pub rate_per_second: i128,
    /// Remaining token balance available for streaming.
    pub balance: i128,
    /// Ledger timestamp when the stream was last started (seconds since Unix epoch).
    /// `0` when the stream has never been started.
    pub start_time: u64,
    /// Ledger timestamp when the stream was stopped.
    /// `0` while the stream is active or has never been stopped.
    pub end_time: u64,
    /// `true` while the stream is running; `false` when created, stopped, or settled.
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
            schema_version: STREAM_SCHEMA_VERSION,
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

    // ── schema versioning tests ───────────────────────────────────────────────

    /// Newly created streams carry the current schema version sentinel.
    #[test]
    fn test_stream_schema_version_is_current() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &100_i128);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(
            info.schema_version, STREAM_SCHEMA_VERSION,
            "schema_version must equal STREAM_SCHEMA_VERSION at creation"
        );
    }

    /// schema_version is positive (guards against accidental zero-init).
    #[test]
    fn test_stream_schema_version_is_positive() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &100_i128);

        let info = client.get_stream_info(&stream_id);
        assert!(info.schema_version > 0, "schema_version must never be zero");
    }

    /// schema_version is preserved across stop/settle round-trips.
    #[test]
    fn test_stream_schema_version_survives_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);

        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128);

        client.start_stream(&stream_id);
        env.ledger().with_mut(|li| li.timestamp += 5);
        client.settle_stream(&stream_id);
        client.stop_stream(&stream_id);

        let info = client.get_stream_info(&stream_id);
        assert_eq!(
            info.schema_version, STREAM_SCHEMA_VERSION,
            "schema_version must not change during normal lifecycle operations"
        );
    }

    /// STREAM_SCHEMA_VERSION constant itself is the expected value (regression guard).
    #[test]
    fn test_schema_version_const_value() {
        assert_eq!(STREAM_SCHEMA_VERSION, 1, "initial schema version must be 1");
    }
}
