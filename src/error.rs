//! # Public Contract Error Codes for StreamPay
//!
//! Provides a stabilized, versioned error classification taxonomy for StreamPay Soroban smart contracts.
//!
//! ## Error Code Ranges
//! - `100..=199`: Validation and Input Errors
//! - `200..=299`: Lifecycle and Stream State Errors
//! - `300..=399`: Storage and Archiving Errors
//! - `400..=499`: Authorization and Access Control Errors
//! - `500..=599`: Settlement and Arithmetic Errors
//! - `900..=999`: System and Fallback Errors

use soroban_sdk::contracterror;

/// Domain category for an [`Error`] variant.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ErrorCategory {
    /// Input validation failures (e.g. non-positive rate, invalid timestamp, oversized batch).
    Validation,
    /// Stream lifecycle and state transition violations (e.g. stream not active, already paused).
    Lifecycle,
    /// Storage, retrieval, and archiving errors (e.g. stream not found, unsettled balance archive).
    Storage,
    /// Authorization and permission failures (e.g. unauthorized caller, not payer).
    Authorization,
    /// Settlement, accrual, and arithmetic errors (e.g. overflow, zero settlement).
    Settlement,
    /// System-level and unknown fallback errors.
    System,
}

/// Recovery expectation for clients encountering an [`Error`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Recoverability {
    /// Transient or parameter-driven failure that can be retried with corrected inputs.
    Retryable,
    /// Requires manual intervention or administrative action.
    RequiresAdmin,
    /// Terminal state violation; the stream cannot accept this operation.
    Terminal,
}

/// Severity classification for logging and monitoring.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    /// Minor input validation or state-check failure.
    Low,
    /// Operational conflict (e.g. already paused, inactive stream).
    Medium,
    /// Security, authorization, or unexpected state boundary violation.
    High,
    /// Critical arithmetic or system fault.
    Critical,
}

/// Public contract error codes for StreamPay.
///
/// Serialized values are preserved across contract upgrades to provide clients
/// with stable, deterministic error classification.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    // ── 100..=199: Validation ────────────────────────────────────────────────
    /// Rate and balance must be strictly positive (> 0).
    RateAndBalanceMustBePositive = 101,
    /// Configured end_time must be in the future relative to current ledger timestamp.
    EndTimeMustBeInFuture = 102,
    /// Batch size exceeds `MAX_BATCH_SETTLE_SIZE` (25).
    BatchTooLarge = 103,
    /// Amount supplied is invalid (e.g. negative or exceeding allowed limits).
    InvalidAmount = 104,
    /// Settlement or transfer amount cannot be zero.
    ZeroAmount = 105,
    /// Invalid time range or schedule configuration.
    InvalidTimeRange = 106,

    // ── 200..=299: Lifecycle ─────────────────────────────────────────────────
    /// Stream is already active and cannot be started again.
    StreamAlreadyActive = 201,
    /// Stream is not active and cannot perform active operations (stop, etc.).
    StreamNotActive = 202,
    /// Cannot cancel an inactive or already stopped stream.
    CannotCancelInactiveStream = 203,
    /// Cannot pause an inactive stream.
    CannotPauseInactiveStream = 204,
    /// Stream is already in a paused state.
    StreamAlreadyPaused = 205,
    /// Cannot resume an inactive stream.
    CannotResumeInactiveStream = 206,
    /// Stream is not paused and therefore cannot be resumed.
    StreamIsNotPaused = 207,
    /// Stream has already reached terminal end state.
    StreamAlreadyTerminated = 208,

    // ── 300..=399: Storage & Archiving ───────────────────────────────────────
    /// Stream was not found in persistent contract storage.
    StreamNotFound = 301,
    /// Cannot archive a stream that is still active.
    CannotArchiveActiveStream = 302,
    /// Cannot archive a stream with unsettled or non-zero remaining balance.
    CannotArchiveStreamWithUnsettledBalance = 303,
    /// Storage key was not found.
    StorageKeyNotFound = 304,
    /// Persistent storage quota exceeded.
    StorageQuotaExceeded = 305,

    // ── 400..=499: Authorization ─────────────────────────────────────────────
    /// Caller is not authorized for the requested operation.
    Unauthorized = 401,
    /// Caller is not the payer of the stream.
    NotPayer = 402,
    /// Caller is not the recipient of the stream.
    NotRecipient = 403,

    // ── 500..=599: Settlement & Arithmetic ───────────────────────────────────
    /// Arithmetic overflow occurred during accrual or balance computation.
    ArithmeticOverflow = 501,
    /// Stream balance is insufficient for settlement.
    InsufficientBalance = 502,
    /// Settlement computation yielded zero transferable value.
    ZeroSettlement = 503,

    // ── 900..=999: System & Fallback ─────────────────────────────────────────
    /// Fallback error for unrecognized error codes or unexpected conditions.
    UnknownError = 999,
}

impl Error {
    /// Numeric error code.
    #[inline]
    pub fn code(&self) -> u32 {
        *self as u32
    }

    /// Client classification code for off-chain SDKs.
    #[inline]
    pub fn client_code(&self) -> u32 {
        self.code()
    }

    /// Safely decodes a `u32` code into an [`Error`] variant.
    ///
    /// Preserves stability for both new categorized codes (100-series) and legacy
    /// sequential indexes (1..=13), and safely falls back to [`Error::UnknownError`]
    /// for any unrecognized value.
    pub fn decode(code: u32) -> Self {
        match code {
            // Categorized codes
            101 | 1 => Error::RateAndBalanceMustBePositive,
            102 | 3 => Error::EndTimeMustBeInFuture,
            103 | 5 => Error::BatchTooLarge,
            104 => Error::InvalidAmount,
            105 => Error::ZeroAmount,
            106 => Error::InvalidTimeRange,

            201 | 2 => Error::StreamAlreadyActive,
            202 | 4 => Error::StreamNotActive,
            203 | 6 => Error::CannotCancelInactiveStream,
            204 | 7 => Error::CannotPauseInactiveStream,
            205 | 8 => Error::StreamAlreadyPaused,
            206 | 9 => Error::CannotResumeInactiveStream,
            207 | 10 => Error::StreamIsNotPaused,
            208 => Error::StreamAlreadyTerminated,

            301 | 13 => Error::StreamNotFound,
            302 | 11 => Error::CannotArchiveActiveStream,
            303 | 12 => Error::CannotArchiveStreamWithUnsettledBalance,
            304 => Error::StorageKeyNotFound,
            305 => Error::StorageQuotaExceeded,

            401 => Error::Unauthorized,
            402 => Error::NotPayer,
            403 => Error::NotRecipient,

            501 => Error::ArithmeticOverflow,
            502 => Error::InsufficientBalance,
            503 => Error::ZeroSettlement,

            999 => Error::UnknownError,

            // Safe fallback for all other unknown values
            _ => Error::UnknownError,
        }
    }

    /// Canonical error description matching existing contract panic messages.
    pub fn message(&self) -> &'static str {
        match self {
            Error::RateAndBalanceMustBePositive => "rate and balance must be positive",
            Error::EndTimeMustBeInFuture => "end_time must be in the future",
            Error::BatchTooLarge => "batch too large",
            Error::InvalidAmount => "invalid amount",
            Error::ZeroAmount => "amount cannot be zero",
            Error::InvalidTimeRange => "invalid time range",

            Error::StreamAlreadyActive => "stream already active",
            Error::StreamNotActive => "stream not active",
            Error::CannotCancelInactiveStream => "cannot cancel inactive stream",
            Error::CannotPauseInactiveStream => "cannot pause inactive stream",
            Error::StreamAlreadyPaused => "stream already paused",
            Error::CannotResumeInactiveStream => "cannot resume inactive stream",
            Error::StreamIsNotPaused => "stream is not paused",
            Error::StreamAlreadyTerminated => "stream already terminated",

            Error::StreamNotFound => "stream not found",
            Error::CannotArchiveActiveStream => "cannot archive active stream",
            Error::CannotArchiveStreamWithUnsettledBalance => {
                "cannot archive stream with unsettled balance"
            }
            Error::StorageKeyNotFound => "storage key not found",
            Error::StorageQuotaExceeded => "storage quota exceeded",

            Error::Unauthorized => "unauthorized caller",
            Error::NotPayer => "caller is not payer",
            Error::NotRecipient => "caller is not recipient",

            Error::ArithmeticOverflow => "arithmetic overflow",
            Error::InsufficientBalance => "insufficient balance",
            Error::ZeroSettlement => "zero settlement",

            Error::UnknownError => "unknown error",
        }
    }

    /// Returns the domain category for this error.
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::RateAndBalanceMustBePositive
            | Error::EndTimeMustBeInFuture
            | Error::BatchTooLarge
            | Error::InvalidAmount
            | Error::ZeroAmount
            | Error::InvalidTimeRange => ErrorCategory::Validation,

            Error::StreamAlreadyActive
            | Error::StreamNotActive
            | Error::CannotCancelInactiveStream
            | Error::CannotPauseInactiveStream
            | Error::StreamAlreadyPaused
            | Error::CannotResumeInactiveStream
            | Error::StreamIsNotPaused
            | Error::StreamAlreadyTerminated => ErrorCategory::Lifecycle,

            Error::StreamNotFound
            | Error::CannotArchiveActiveStream
            | Error::CannotArchiveStreamWithUnsettledBalance
            | Error::StorageKeyNotFound
            | Error::StorageQuotaExceeded => ErrorCategory::Storage,

            Error::Unauthorized | Error::NotPayer | Error::NotRecipient => {
                ErrorCategory::Authorization
            }

            Error::ArithmeticOverflow | Error::InsufficientBalance | Error::ZeroSettlement => {
                ErrorCategory::Settlement
            }

            Error::UnknownError => ErrorCategory::System,
        }
    }

    /// Returns the expected recoverability strategy.
    pub fn recoverability(&self) -> Recoverability {
        match self {
            Error::RateAndBalanceMustBePositive
            | Error::EndTimeMustBeInFuture
            | Error::BatchTooLarge
            | Error::InvalidAmount
            | Error::ZeroAmount
            | Error::InvalidTimeRange
            | Error::CannotPauseInactiveStream
            | Error::StreamAlreadyPaused
            | Error::CannotResumeInactiveStream
            | Error::StreamIsNotPaused
            | Error::CannotArchiveStreamWithUnsettledBalance
            | Error::StreamNotFound => Recoverability::Retryable,

            Error::StorageQuotaExceeded | Error::StorageKeyNotFound => {
                Recoverability::RequiresAdmin
            }

            Error::StreamAlreadyActive
            | Error::StreamNotActive
            | Error::CannotCancelInactiveStream
            | Error::StreamAlreadyTerminated
            | Error::CannotArchiveActiveStream
            | Error::Unauthorized
            | Error::NotPayer
            | Error::NotRecipient
            | Error::ArithmeticOverflow
            | Error::InsufficientBalance
            | Error::ZeroSettlement
            | Error::UnknownError => Recoverability::Terminal,
        }
    }

    /// Returns the operational severity level.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Error::ZeroAmount | Error::ZeroSettlement => ErrorSeverity::Low,

            Error::RateAndBalanceMustBePositive
            | Error::EndTimeMustBeInFuture
            | Error::BatchTooLarge
            | Error::InvalidAmount
            | Error::InvalidTimeRange
            | Error::InsufficientBalance
            | Error::StreamAlreadyActive
            | Error::StreamNotActive
            | Error::CannotCancelInactiveStream
            | Error::CannotPauseInactiveStream
            | Error::StreamAlreadyPaused
            | Error::CannotResumeInactiveStream
            | Error::StreamIsNotPaused
            | Error::StreamAlreadyTerminated
            | Error::StreamNotFound
            | Error::CannotArchiveActiveStream
            | Error::CannotArchiveStreamWithUnsettledBalance => ErrorSeverity::Medium,

            Error::Unauthorized | Error::NotPayer | Error::NotRecipient => ErrorSeverity::High,

            Error::ArithmeticOverflow
            | Error::StorageKeyNotFound
            | Error::StorageQuotaExceeded
            | Error::UnknownError => ErrorSeverity::Critical,
        }
    }

    /// List of all concrete error variants.
    pub fn all_errors() -> &'static [Error] {
        &[
            Error::RateAndBalanceMustBePositive,
            Error::EndTimeMustBeInFuture,
            Error::BatchTooLarge,
            Error::InvalidAmount,
            Error::ZeroAmount,
            Error::InvalidTimeRange,
            Error::StreamAlreadyActive,
            Error::StreamNotActive,
            Error::CannotCancelInactiveStream,
            Error::CannotPauseInactiveStream,
            Error::StreamAlreadyPaused,
            Error::CannotResumeInactiveStream,
            Error::StreamIsNotPaused,
            Error::StreamAlreadyTerminated,
            Error::StreamNotFound,
            Error::CannotArchiveActiveStream,
            Error::CannotArchiveStreamWithUnsettledBalance,
            Error::StorageKeyNotFound,
            Error::StorageQuotaExceeded,
            Error::Unauthorized,
            Error::NotPayer,
            Error::NotRecipient,
            Error::ArithmeticOverflow,
            Error::InsufficientBalance,
            Error::ZeroSettlement,
            Error::UnknownError,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_are_unique() {
        let errors = Error::all_errors();
        for i in 0..errors.len() {
            for j in (i + 1)..errors.len() {
                assert_ne!(
                    errors[i].code(),
                    errors[j].code(),
                    "Duplicate error code detected between {:?} and {:?}",
                    errors[i],
                    errors[j]
                );
            }
        }
    }

    #[test]
    fn test_every_variant_decodes_correctly() {
        for err in Error::all_errors() {
            let code = err.code();
            let decoded = Error::decode(code);
            assert_eq!(
                decoded, *err,
                "Error {:?} failed to round-trip decode from code {}",
                err, code
            );
        }
    }

    #[test]
    fn test_unknown_error_codes_fail_safely() {
        let unknown_codes = [
            0,
            99,
            100,
            199,
            200,
            299,
            300,
            399,
            400,
            499,
            500,
            599,
            600,
            1000,
            99999,
            u32::MAX,
        ];
        for code in unknown_codes {
            let decoded = Error::decode(code);
            assert_eq!(
                decoded,
                Error::UnknownError,
                "Unknown code {} did not decode to Error::UnknownError",
                code
            );
        }
    }

    #[test]
    fn test_legacy_sequential_codes_decode_correctly() {
        assert_eq!(Error::decode(1), Error::RateAndBalanceMustBePositive);
        assert_eq!(Error::decode(2), Error::StreamAlreadyActive);
        assert_eq!(Error::decode(3), Error::EndTimeMustBeInFuture);
        assert_eq!(Error::decode(4), Error::StreamNotActive);
        assert_eq!(Error::decode(5), Error::BatchTooLarge);
        assert_eq!(Error::decode(6), Error::CannotCancelInactiveStream);
        assert_eq!(Error::decode(7), Error::CannotPauseInactiveStream);
        assert_eq!(Error::decode(8), Error::StreamAlreadyPaused);
        assert_eq!(Error::decode(9), Error::CannotResumeInactiveStream);
        assert_eq!(Error::decode(10), Error::StreamIsNotPaused);
        assert_eq!(Error::decode(11), Error::CannotArchiveActiveStream);
        assert_eq!(
            Error::decode(12),
            Error::CannotArchiveStreamWithUnsettledBalance
        );
        assert_eq!(Error::decode(13), Error::StreamNotFound);
    }

    #[test]
    fn test_error_categories_and_ranges() {
        for err in Error::all_errors() {
            let code = err.code();
            let cat = err.category();
            match cat {
                ErrorCategory::Validation => {
                    assert!(
                        (100..=199).contains(&code),
                        "Validation error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
                ErrorCategory::Lifecycle => {
                    assert!(
                        (200..=299).contains(&code),
                        "Lifecycle error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
                ErrorCategory::Storage => {
                    assert!(
                        (300..=399).contains(&code),
                        "Storage error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
                ErrorCategory::Authorization => {
                    assert!(
                        (400..=499).contains(&code),
                        "Authorization error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
                ErrorCategory::Settlement => {
                    assert!(
                        (500..=599).contains(&code),
                        "Settlement error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
                ErrorCategory::System => {
                    assert!(
                        (900..=999).contains(&code),
                        "System error {:?} has out-of-range code {}",
                        err,
                        code
                    );
                }
            }
        }
    }

    #[test]
    fn test_messages_are_non_empty() {
        for err in Error::all_errors() {
            let msg = err.message();
            assert!(!msg.is_empty(), "Error {:?} has empty message", err);
        }
    }
}
