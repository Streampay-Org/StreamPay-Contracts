//! # Comprehensive Error Code Stabilization & Golden Vector Test Suite (#157)
//!
//! Enforces the Acceptance Criteria for Issue #157:
//! 1. Existing error codes remain stable and backward-compatible.
//! 2. New error codes are unique and allocated in category-disjoint ranges.
//! 3. Unknown / out-of-range values decode and fail safely to `Error::UnknownError`.
//! 4. Golden vectors cover all public entrypoints and failure modes.
//! 5. Authorization boundaries and state preservation across errors.

use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Vec as SorobanVec};
use streampay_contracts::{
    Error, ErrorCategory, ErrorSeverity, Recoverability, StreamPayContract, StreamPayContractClient,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn setup_client(env: &Env) -> (StreamPayContractClient<'_>, Address, Address) {
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(env, &contract_id);
    let payer = Address::generate(env);
    let recipient = Address::generate(env);
    (client, payer, recipient)
}

fn advance_time(env: &Env, seconds: u64) {
    env.ledger().with_mut(|li| {
        li.timestamp += seconds;
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// AC1 & AC2: Code Stability, Uniqueness, and Categorization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_error_code_uniqueness_across_all_variants() {
    let all = Error::all_errors();
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert_ne!(
                all[i].code(),
                all[j].code(),
                "Duplicate numeric code detected between {:?} ({}) and {:?} ({})",
                all[i],
                all[i].code(),
                all[j],
                all[j].code()
            );
        }
    }
}

#[test]
fn test_error_codes_are_stable_constants() {
    // Exact numerical values guaranteed to clients across contract upgrades
    assert_eq!(Error::RateAndBalanceMustBePositive.code(), 101);
    assert_eq!(Error::EndTimeMustBeInFuture.code(), 102);
    assert_eq!(Error::BatchTooLarge.code(), 103);
    assert_eq!(Error::InvalidAmount.code(), 104);
    assert_eq!(Error::ZeroAmount.code(), 105);
    assert_eq!(Error::InvalidTimeRange.code(), 106);

    assert_eq!(Error::StreamAlreadyActive.code(), 201);
    assert_eq!(Error::StreamNotActive.code(), 202);
    assert_eq!(Error::CannotCancelInactiveStream.code(), 203);
    assert_eq!(Error::CannotPauseInactiveStream.code(), 204);
    assert_eq!(Error::StreamAlreadyPaused.code(), 205);
    assert_eq!(Error::CannotResumeInactiveStream.code(), 206);
    assert_eq!(Error::StreamIsNotPaused.code(), 207);
    assert_eq!(Error::StreamAlreadyTerminated.code(), 208);

    assert_eq!(Error::StreamNotFound.code(), 301);
    assert_eq!(Error::CannotArchiveActiveStream.code(), 302);
    assert_eq!(Error::CannotArchiveStreamWithUnsettledBalance.code(), 303);
    assert_eq!(Error::StorageKeyNotFound.code(), 304);
    assert_eq!(Error::StorageQuotaExceeded.code(), 305);

    assert_eq!(Error::Unauthorized.code(), 401);
    assert_eq!(Error::NotPayer.code(), 402);
    assert_eq!(Error::NotRecipient.code(), 403);

    assert_eq!(Error::ArithmeticOverflow.code(), 501);
    assert_eq!(Error::InsufficientBalance.code(), 502);
    assert_eq!(Error::ZeroSettlement.code(), 503);

    assert_eq!(Error::UnknownError.code(), 999);
}

#[test]
fn test_category_disjoint_ranges() {
    for err in Error::all_errors() {
        let code = err.code();
        match err.category() {
            ErrorCategory::Validation => {
                assert!(
                    (100..=199).contains(&code),
                    "Validation error {:?} with code {} outside 100..=199",
                    err,
                    code
                );
            }
            ErrorCategory::Lifecycle => {
                assert!(
                    (200..=299).contains(&code),
                    "Lifecycle error {:?} with code {} outside 200..=299",
                    err,
                    code
                );
            }
            ErrorCategory::Storage => {
                assert!(
                    (300..=399).contains(&code),
                    "Storage error {:?} with code {} outside 300..=399",
                    err,
                    code
                );
            }
            ErrorCategory::Authorization => {
                assert!(
                    (400..=499).contains(&code),
                    "Authorization error {:?} with code {} outside 400..=499",
                    err,
                    code
                );
            }
            ErrorCategory::Settlement => {
                assert!(
                    (500..=599).contains(&code),
                    "Settlement error {:?} with code {} outside 500..=599",
                    err,
                    code
                );
            }
            ErrorCategory::System => {
                assert!(
                    (900..=999).contains(&code),
                    "System error {:?} with code {} outside 900..=999",
                    err,
                    code
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AC3: Safe Fallback for Unknown and Legacy Values
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_unknown_values_decode_safely_to_fallback() {
    let test_codes = [
        0,
        50,
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
        800,
        1000,
        12345,
        u32::MAX,
    ];
    for code in test_codes {
        let decoded = Error::decode(code);
        assert_eq!(
            decoded,
            Error::UnknownError,
            "Unknown code {} must decode to Error::UnknownError",
            code
        );
    }
}

#[test]
fn test_legacy_sequential_index_backward_compatibility() {
    let legacy_map = [
        (1, Error::RateAndBalanceMustBePositive),
        (2, Error::StreamAlreadyActive),
        (3, Error::EndTimeMustBeInFuture),
        (4, Error::StreamNotActive),
        (5, Error::BatchTooLarge),
        (6, Error::CannotCancelInactiveStream),
        (7, Error::CannotPauseInactiveStream),
        (8, Error::StreamAlreadyPaused),
        (9, Error::CannotResumeInactiveStream),
        (10, Error::StreamIsNotPaused),
        (11, Error::CannotArchiveActiveStream),
        (12, Error::CannotArchiveStreamWithUnsettledBalance),
        (13, Error::StreamNotFound),
    ];

    for (legacy_code, expected_err) in legacy_map {
        let decoded = Error::decode(legacy_code);
        assert_eq!(
            decoded, expected_err,
            "Legacy code {} did not decode to expected {:?}",
            legacy_code, expected_err
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AC4: Golden Vectors Covering Public Entrypoints
// ═══════════════════════════════════════════════════════════════════════════

struct EntrypointGoldenVector {
    name: &'static str,
    expected_error: Error,
    expected_code: u32,
    expected_message: &'static str,
    expected_category: ErrorCategory,
    expected_recoverability: Recoverability,
    expected_severity: ErrorSeverity,
}

#[test]
fn test_golden_vectors_specification() {
    let golden_vectors = [
        EntrypointGoldenVector {
            name: "create_stream (non-positive rate/balance)",
            expected_error: Error::RateAndBalanceMustBePositive,
            expected_code: 101,
            expected_message: "rate and balance must be positive",
            expected_category: ErrorCategory::Validation,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "start_stream (end_time in past or now)",
            expected_error: Error::EndTimeMustBeInFuture,
            expected_code: 102,
            expected_message: "end_time must be in the future",
            expected_category: ErrorCategory::Validation,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "batch_settle (exceeding MAX_BATCH_SETTLE_SIZE)",
            expected_error: Error::BatchTooLarge,
            expected_code: 103,
            expected_message: "batch too large",
            expected_category: ErrorCategory::Validation,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "start_stream (already active)",
            expected_error: Error::StreamAlreadyActive,
            expected_code: 201,
            expected_message: "stream already active",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Terminal,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "stop_stream (not active)",
            expected_error: Error::StreamNotActive,
            expected_code: 202,
            expected_message: "stream not active",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Terminal,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "cancel_stream (inactive stream)",
            expected_error: Error::CannotCancelInactiveStream,
            expected_code: 203,
            expected_message: "cannot cancel inactive stream",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Terminal,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "pause_stream (inactive stream)",
            expected_error: Error::CannotPauseInactiveStream,
            expected_code: 204,
            expected_message: "cannot pause inactive stream",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "pause_stream (already paused)",
            expected_error: Error::StreamAlreadyPaused,
            expected_code: 205,
            expected_message: "stream already paused",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "resume_stream (inactive stream)",
            expected_error: Error::CannotResumeInactiveStream,
            expected_code: 206,
            expected_message: "cannot resume inactive stream",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "resume_stream (not paused)",
            expected_error: Error::StreamIsNotPaused,
            expected_code: 207,
            expected_message: "stream is not paused",
            expected_category: ErrorCategory::Lifecycle,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "get_stream_info (missing ID)",
            expected_error: Error::StreamNotFound,
            expected_code: 301,
            expected_message: "stream not found",
            expected_category: ErrorCategory::Storage,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "archive_stream (stream active)",
            expected_error: Error::CannotArchiveActiveStream,
            expected_code: 302,
            expected_message: "cannot archive active stream",
            expected_category: ErrorCategory::Storage,
            expected_recoverability: Recoverability::Terminal,
            expected_severity: ErrorSeverity::Medium,
        },
        EntrypointGoldenVector {
            name: "archive_stream (unsettled balance)",
            expected_error: Error::CannotArchiveStreamWithUnsettledBalance,
            expected_code: 303,
            expected_message: "cannot archive stream with unsettled balance",
            expected_category: ErrorCategory::Storage,
            expected_recoverability: Recoverability::Retryable,
            expected_severity: ErrorSeverity::Medium,
        },
    ];

    for gv in golden_vectors {
        assert_eq!(
            gv.expected_error.code(),
            gv.expected_code,
            "Golden vector {} code mismatch",
            gv.name
        );
        assert_eq!(
            gv.expected_error.message(),
            gv.expected_message,
            "Golden vector {} message mismatch",
            gv.name
        );
        assert_eq!(
            gv.expected_error.category(),
            gv.expected_category,
            "Golden vector {} category mismatch",
            gv.name
        );
        assert_eq!(
            gv.expected_error.recoverability(),
            gv.expected_recoverability,
            "Golden vector {} recoverability mismatch",
            gv.name
        );
        assert_eq!(
            gv.expected_error.severity(),
            gv.expected_severity,
            "Golden vector {} severity mismatch",
            gv.name
        );
        assert_eq!(
            Error::decode(gv.expected_code),
            gv.expected_error,
            "Golden vector {} decode round-trip failed",
            gv.name
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Entrypoint Execution & Failure Mode Regression Coverage
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_create_stream_validation_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    // Rate <= 0
    let res_rate_zero = catch_unwind(AssertUnwindSafe(|| {
        client.create_stream(&payer, &recipient, &0_i128, &1000_i128, &0_u64)
    }));
    assert!(res_rate_zero.is_err());

    let res_rate_neg = catch_unwind(AssertUnwindSafe(|| {
        client.create_stream(&payer, &recipient, &-10_i128, &1000_i128, &0_u64)
    }));
    assert!(res_rate_neg.is_err());

    // Balance <= 0
    let res_bal_zero = catch_unwind(AssertUnwindSafe(|| {
        client.create_stream(&payer, &recipient, &10_i128, &0_i128, &0_u64)
    }));
    assert!(res_bal_zero.is_err());

    let res_bal_neg = catch_unwind(AssertUnwindSafe(|| {
        client.create_stream(&payer, &recipient, &10_i128, &-500_i128, &0_u64)
    }));
    assert!(res_bal_neg.is_err());
}

#[test]
fn test_start_stream_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    // Valid create with end_time in the past
    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &5_u64);
    advance_time(&env, 10); // Now timestamp is 10 > end_time (5)

    let res_past_end = catch_unwind(AssertUnwindSafe(|| {
        client.start_stream(&stream_id);
    }));
    assert!(res_past_end.is_err());

    // Create stream with valid future end time
    let stream_id_2 = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &100_u64);
    client.start_stream(&stream_id_2);

    // Starting already active stream fails
    let res_already_active = catch_unwind(AssertUnwindSafe(|| {
        client.start_stream(&stream_id_2);
    }));
    assert!(res_already_active.is_err());
}

#[test]
fn test_stop_stream_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);

    // Stopping un-started stream fails
    let res_inactive = catch_unwind(AssertUnwindSafe(|| {
        client.stop_stream(&stream_id);
    }));
    assert!(res_inactive.is_err());

    client.start_stream(&stream_id);
    client.stop_stream(&stream_id);

    // Stopping already stopped stream fails
    let res_already_stopped = catch_unwind(AssertUnwindSafe(|| {
        client.stop_stream(&stream_id);
    }));
    assert!(res_already_stopped.is_err());
}

#[test]
fn test_cancel_stream_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);

    // Cancel un-started stream fails
    let res_inactive = catch_unwind(AssertUnwindSafe(|| {
        client.cancel_stream(&stream_id);
    }));
    assert!(res_inactive.is_err());

    client.start_stream(&stream_id);
    client.cancel_stream(&stream_id);

    // Cancel already cancelled stream fails
    let res_already_cancelled = catch_unwind(AssertUnwindSafe(|| {
        client.cancel_stream(&stream_id);
    }));
    assert!(res_already_cancelled.is_err());
}

#[test]
fn test_pause_and_resume_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);

    // Pause inactive fails
    let res_pause_inactive = catch_unwind(AssertUnwindSafe(|| {
        client.pause_stream(&stream_id);
    }));
    assert!(res_pause_inactive.is_err());

    // Resume inactive fails
    let res_resume_inactive = catch_unwind(AssertUnwindSafe(|| {
        client.resume_stream(&stream_id);
    }));
    assert!(res_resume_inactive.is_err());

    client.start_stream(&stream_id);

    // Resume un-paused active stream fails
    let res_resume_not_paused = catch_unwind(AssertUnwindSafe(|| {
        client.resume_stream(&stream_id);
    }));
    assert!(res_resume_not_paused.is_err());

    advance_time(&env, 2);
    client.pause_stream(&stream_id);

    // Double pause fails
    let res_double_pause = catch_unwind(AssertUnwindSafe(|| {
        client.pause_stream(&stream_id);
    }));
    assert!(res_double_pause.is_err());

    client.resume_stream(&stream_id);
}

#[test]
fn test_archive_stream_failure_modes() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &100_i128, &0_u64);

    // Cannot archive un-started stream with positive balance
    let res_unsettled = catch_unwind(AssertUnwindSafe(|| {
        client.archive_stream(&stream_id);
    }));
    assert!(res_unsettled.is_err());

    client.start_stream(&stream_id);

    // Cannot archive active stream
    let res_active = catch_unwind(AssertUnwindSafe(|| {
        client.archive_stream(&stream_id);
    }));
    assert!(res_active.is_err());

    // Settle full balance and stop
    advance_time(&env, 10);
    client.settle_stream(&stream_id);
    client.stop_stream(&stream_id);

    // Archive succeeds
    client.archive_stream(&stream_id);

    // Fetching archived stream fails with StreamNotFound
    let res_not_found = catch_unwind(AssertUnwindSafe(|| {
        client.get_stream_info(&stream_id);
    }));
    assert!(res_not_found.is_err());
}

#[test]
fn test_batch_settle_bounds_and_failure_modes() {
    let env = Env::default();
    let (client, _payer, _recipient) = setup_client(&env);

    let mut too_large = SorobanVec::new(&env);
    for i in 1..=26 {
        too_large.push_back(i);
    }

    let res_too_large = catch_unwind(AssertUnwindSafe(|| {
        client.batch_settle(&too_large);
    }));
    assert!(res_too_large.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// Authorization Boundaries and State Preservation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_authorization_boundary_failure_preserves_state() {
    let env = Env::default();
    let (client, payer, recipient) = setup_client(&env);

    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);
    client.start_stream(&stream_id);

    advance_time(&env, 5);
    let before = client.get_stream_info(&stream_id);

    // Disable all auth mocks to verify authentication failure boundaries
    env.set_auths(&[]);

    let res_cancel = catch_unwind(AssertUnwindSafe(|| {
        client.cancel_stream(&stream_id);
    }));
    assert!(res_cancel.is_err());

    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    let res_stop = catch_unwind(AssertUnwindSafe(|| {
        client.stop_stream(&stream_id);
    }));
    assert!(res_stop.is_err());

    let res_archive = catch_unwind(AssertUnwindSafe(|| {
        client.archive_stream(&stream_id);
    }));
    assert!(res_archive.is_err());

    // Re-enable auths to inspect state
    env.mock_all_auths();
    let after = client.get_stream_info(&stream_id);

    assert_eq!(after.balance, before.balance);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.is_active, before.is_active);
    assert_eq!(after.paused_at, before.paused_at);
}
