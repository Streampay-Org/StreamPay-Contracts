//! Stream storage primitives.
//!
//! This module owns the on-chain shape of a single `StreamInfo` record and
//! the small set of helpers used by `lib.rs` to read, write and bump the TTL
//! on that record. Keeping the storage layer here makes the entry-point
//! contract in `lib.rs` easier to read and audit.
//!
//! Storage layout:
//!
//! - Persistent: `("stream", stream_id) -> StreamInfo`
//!
//! See `docs/storage-layout.md` for the full layout including the instance
//! counter and event topics.

use soroban_sdk::{contracttype, Address, Env, String, Symbol};

/// TTL threshold: extend when remaining TTL drops below ~1 day (17_280 ledgers at ~5s each).
pub const STREAM_TTL_THRESHOLD: u32 = 17_280;
/// TTL extend-to: refresh to ~30 days (518_400 ledgers).
pub const STREAM_TTL_EXTEND: u32 = 518_400;

/// Bump whenever `StreamInfo` fields change. See `docs/schema-versioning.md`.
pub const STREAM_SCHEMA_VERSION: u32 = 2;

/// Accrual mode for a stream.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StreamMode {
    /// Continuous rate-per-second streaming.
    Linear,
    /// Total amount unlocks linearly over `duration_seconds` from the schedule anchor.
    LinearVesting {
        duration_seconds: u64,
        /// Cumulative amount already released from the vesting schedule.
        vested_amount: i128,
        /// Unix seconds when the vesting clock started (first `start_stream`).
        schedule_anchor: u64,
    },
}

/// On-chain representation of a payment stream.
///
/// Stored under `("stream", stream_id)` in persistent storage. All time fields
/// are Unix seconds derived from `Env::ledger().timestamp()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    /// Schema sentinel for safe upgrades. Immutable after creation.
    pub schema_version: u32,
    /// Account that funded the stream and retains ownership of the escrowed balance.
    pub payer: Address,
    /// Account entitled to receive accrued tokens once they are settled.
    pub recipient: Address,
    /// SEP-41-compatible token contract used for this stream.
    pub token: Address,
    /// Tokens streamed per whole second. Must be strictly positive for linear streams.
    pub rate_per_second: i128,
    /// Tokens still held in escrow for this stream (not yet earned).
    pub balance: i128,
    /// Tokens accrued by the recipient but not yet withdrawn on-ledger.
    pub claimable_balance: i128,
    /// Unix seconds when the stream was started or last settled.
    pub start_time: u64,
    /// Unix seconds when accrual stops; `0` means "no time limit" until stopped.
    pub end_time: u64,
    /// Whether the stream is currently accruing.
    pub is_active: bool,
    /// Pause timestamp; `0` when not paused.
    pub paused_at: u64,
    /// Immutable off-chain correlation string (max 32 bytes).
    pub memo: String,
    /// When `true`, the recipient may call `stop_stream` as `stopper`.
    pub recipient_can_stop: bool,
    /// Accrual mode (linear rate or linear vesting).
    pub mode: StreamMode,
}

/// Build the persistent storage key used for the given stream id.
pub fn stream_key(env: &Env, stream_id: u32) -> (Symbol, u32) {
    (Symbol::new(env, "stream"), stream_id)
}

/// Load a `StreamInfo` from persistent storage.
///
/// Panics with `"stream not found"` if no entry exists for the id.
pub fn get_stream(env: &Env, stream_id: u32) -> StreamInfo {
    let key = stream_key(env, stream_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("stream not found"))
}

/// Persist `info` under the canonical key for `stream_id`.
pub fn set_stream(env: &Env, stream_id: u32, info: &StreamInfo) {
    let key = stream_key(env, stream_id);
    env.storage().persistent().set(&key, info);
}

/// Bump the TTL of a stream's persistent entry to roughly 30 days.
pub fn extend_stream_ttl(env: &Env, stream_id: u32) {
    let key = stream_key(env, stream_id);
    env.storage()
        .persistent()
        .extend_ttl(&key, STREAM_TTL_THRESHOLD, STREAM_TTL_EXTEND);
}
