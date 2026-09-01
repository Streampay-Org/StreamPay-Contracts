use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, IntoVal, Vec as SorobanVec};
use streampay_contracts::{StreamInfo, StreamPayContract, StreamPayContractClient};

/// Helper struct for test context
struct TestContext<'a> {
    env: Env,
    client: StreamPayContractClient<'a>,
    payer: Address,
    recipient: Address,
    other: Address,
}

impl<'a> TestContext<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(StreamPayContract, ());
        let client = StreamPayContractClient::new(&env, &contract_id);
        let payer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let other = Address::generate(&env);
        Self {
            env,
            client,
            payer,
            recipient,
            other,
        }
    }

    fn advance_time(&self, seconds: u64) {
        self.env.ledger().with_mut(|li| {
            li.timestamp += seconds;
        });
    }

    fn set_time(&self, timestamp: u64) {
        self.env.ledger().with_mut(|li| {
            li.timestamp = timestamp;
        });
    }

    fn create_stream(&self, rate: i128, balance: i128, end_time: u64) -> u32 {
        self.client
            .create_stream(&self.payer, &self.recipient, &rate, &balance, &end_time)
    }

    fn snapshot(&self, stream_id: u32) -> StreamInfo {
        self.client.get_stream_info(&stream_id)
    }
}

// ── Acceptance Criterion 1: Value-moving calls pause as intended ─────────────

#[test]
fn test_settle_stream_while_paused_moves_zero_value() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    // Run for 5 seconds -> earns 50
    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    let info_at_pause = ctx.snapshot(stream_id);
    assert_eq!(info_at_pause.balance, 950);
    assert_eq!(info_at_pause.paused_at, 5);
    assert!(info_at_pause.is_active);

    // Advance 10 seconds while paused
    ctx.advance_time(10);
    let amount1 = ctx.client.settle_stream(&stream_id);
    assert_eq!(amount1, 0, "settle while paused must return 0");

    let info_after_settle1 = ctx.snapshot(stream_id);
    assert_eq!(
        info_after_settle1.balance, 950,
        "balance must not change while paused"
    );
    assert_eq!(info_after_settle1.paused_at, 5);

    // Advance another 100 seconds while paused
    ctx.advance_time(100);
    let amount2 = ctx.client.settle_stream(&stream_id);
    assert_eq!(amount2, 0, "settle after long pause must still return 0");

    let info_after_settle2 = ctx.snapshot(stream_id);
    assert_eq!(info_after_settle2.balance, 950);
    assert_eq!(info_after_settle2.paused_at, 5);
}

#[test]
fn test_batch_settle_while_paused_moves_zero_value() {
    let ctx = TestContext::new();
    let sid1 = ctx.create_stream(10, 1_000, 0);
    let sid2 = ctx.create_stream(20, 2_000, 0);
    ctx.client.start_stream(&sid1);
    ctx.client.start_stream(&sid2);

    ctx.advance_time(5);
    // Pause sid1, keep sid2 active
    ctx.client.pause_stream(&sid1);

    ctx.advance_time(10); // at t=15
    let mut batch = SorobanVec::new(&ctx.env);
    batch.push_back(sid1);
    batch.push_back(sid2);

    let amounts = ctx.client.batch_settle(&batch);
    assert_eq!(amounts.len(), 2);
    assert_eq!(
        amounts.get(0).unwrap(),
        0,
        "paused stream in batch must yield 0"
    );
    assert_eq!(
        amounts.get(1).unwrap(),
        300,
        "active stream in batch must settle normally (15s * 20)"
    );

    let info1 = ctx.snapshot(sid1);
    let info2 = ctx.snapshot(sid2);
    assert_eq!(info1.balance, 950);
    assert_eq!(info2.balance, 1_700);
}

// ── Acceptance Criterion 2: Recovery paths are explicit and authorized ────────

#[test]
fn test_recovery_path_cancel_while_paused() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    // While paused, emergency cancel by payer
    ctx.advance_time(20);
    ctx.client.cancel_stream(&stream_id);

    let info = ctx.snapshot(stream_id);
    assert!(!info.is_active, "stream must be inactive after cancel");
    assert_eq!(
        info.paused_at, 0,
        "paused_at must be reset on terminal cancel"
    );
    assert_eq!(info.balance, 950, "payer retains unaccrued balance of 950");

    // Further settlement attempts on terminal stream return 0
    ctx.advance_time(10);
    let amount = ctx.client.settle_stream(&stream_id);
    assert_eq!(amount, 0);
}

#[test]
fn test_recovery_path_stop_while_paused() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    // While paused, stop by payer
    ctx.advance_time(15);
    ctx.client.stop_stream(&stream_id);

    let info = ctx.snapshot(stream_id);
    assert!(!info.is_active);
    assert_eq!(info.paused_at, 0);
    assert_eq!(info.balance, 950);
}

#[test]
fn test_unauthorized_recovery_paths_rejected() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    let snapshot_before = ctx.snapshot(stream_id);

    // Revoke mock authorizations
    ctx.env.set_auths(&[]);

    // 1. Unauthorized cancel fails
    let res_cancel = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.cancel_stream(&stream_id);
    }));
    assert!(res_cancel.is_err());

    // 2. Unauthorized stop fails
    let res_stop = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.stop_stream(&stream_id);
    }));
    assert!(res_stop.is_err());

    // 3. Unauthorized resume fails
    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());

    // 4. Unauthorized pause fails
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    // 5. Unauthorized archive fails
    let res_archive = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.archive_stream(&stream_id);
    }));
    assert!(res_archive.is_err());

    // State remains completely intact
    let snapshot_after = ctx.snapshot(stream_id);
    assert_eq!(snapshot_after.balance, snapshot_before.balance);
    assert_eq!(snapshot_after.is_active, snapshot_before.is_active);
    assert_eq!(snapshot_after.paused_at, snapshot_before.paused_at);
    assert_eq!(snapshot_after.start_time, snapshot_before.start_time);
}

#[test]
fn test_non_payer_role_transition_isolation() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    // Mock only recipient or other third party auth, not payer
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.recipient,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.client.address,
            fn_name: "pause_stream",
            args: (&stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let res_recipient_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_recipient_pause.is_err());

    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.other,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.client.address,
            fn_name: "resume_stream",
            args: (&stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let res_other_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_other_resume.is_err());
}

// ── Acceptance Criterion 3: Resume cannot reset balances or timestamps ─────────

#[test]
fn test_resume_preserves_balance_and_updates_start_timestamp() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    // t=0 -> t=4: accrue 40
    ctx.advance_time(4);
    ctx.client.pause_stream(&stream_id);

    let info_paused = ctx.snapshot(stream_id);
    assert_eq!(info_paused.balance, 960);
    assert_eq!(info_paused.paused_at, 4);

    // Paused from t=4 to t=24 (20 seconds pause duration)
    ctx.advance_time(20);
    ctx.client.resume_stream(&stream_id);

    let info_resumed = ctx.snapshot(stream_id);
    assert_eq!(
        info_resumed.balance, 960,
        "resume must NOT reset balance to initial or modify balance"
    );
    assert_eq!(
        info_resumed.start_time, 24,
        "resume must set start_time to current ledger timestamp"
    );
    assert_eq!(info_resumed.paused_at, 0, "paused_at must be cleared");
    assert!(info_resumed.is_active);

    // Accrue for 6 seconds after resume: t=24 -> t=30
    ctx.advance_time(6);
    let amount = ctx.client.settle_stream(&stream_id);
    assert_eq!(
        amount, 60,
        "accrual after resume must only account for active interval [24, 30]"
    );

    let info_final = ctx.snapshot(stream_id);
    assert_eq!(info_final.balance, 900);
    assert_eq!(info_final.start_time, 30);
}

#[test]
fn test_multiple_pause_resume_cycles_strictly_conserve_balance() {
    let ctx = TestContext::new();
    let initial_balance = 10_000_i128;
    let rate = 5_i128;
    let stream_id = ctx.create_stream(rate, initial_balance, 0);
    ctx.client.start_stream(&stream_id);

    let mut expected_balance = initial_balance;
    let mut total_settled = 0_i128;

    // Execute 5 cycles of [active run -> pause -> idle pause -> settle check -> resume]
    for cycle in 1..=5 {
        let active_secs = cycle * 10;
        let paused_secs = cycle * 20;

        // Active period
        ctx.advance_time(active_secs);
        ctx.client.pause_stream(&stream_id);
        let earned = (active_secs as i128) * rate;
        expected_balance -= earned;
        total_settled += earned;

        let info_pause = ctx.snapshot(stream_id);
        assert_eq!(
            info_pause.balance, expected_balance,
            "Cycle {cycle}: pause balance check"
        );
        assert!(info_pause.paused_at > 0);

        // Paused idle period
        ctx.advance_time(paused_secs);
        let settle_amount = ctx.client.settle_stream(&stream_id);
        assert_eq!(
            settle_amount, 0,
            "Cycle {cycle}: settle while paused must be 0"
        );

        // Resume
        ctx.client.resume_stream(&stream_id);
        let info_resume = ctx.snapshot(stream_id);
        assert_eq!(
            info_resume.balance, expected_balance,
            "Cycle {cycle}: resume must not alter balance"
        );
        assert_eq!(info_resume.paused_at, 0);
    }

    // Final settlement on active stream
    ctx.advance_time(10);
    let final_settle = ctx.client.settle_stream(&stream_id);
    total_settled += final_settle;
    expected_balance -= final_settle;

    let final_info = ctx.snapshot(stream_id);
    assert_eq!(final_info.balance, expected_balance);
    assert_eq!(total_settled + final_info.balance, initial_balance);
}

#[test]
fn test_resume_on_stream_past_natural_end_is_terminal() {
    let ctx = TestContext::new();
    // Bounded stream ending at t=10
    let stream_id = ctx.create_stream(10, 1_000, 10);
    ctx.client.start_stream(&stream_id);

    // Pause at t=4 -> accrued 40, remaining balance 960
    ctx.advance_time(4);
    ctx.client.pause_stream(&stream_id);

    // Advance past end_time: t=15
    ctx.set_time(15);
    ctx.client.resume_stream(&stream_id);

    let info = ctx.snapshot(stream_id);
    assert!(
        !info.is_active,
        "resuming past end_time must terminalize stream"
    );
    assert_eq!(info.paused_at, 0);
    assert_eq!(info.balance, 960, "no post-end accrual may occur");

    // Subsequent pause or resume must fail
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());
}

// ── Acceptance Criterion 4: Entrypoint-matrix tests cover mutations ───────────

#[test]
fn test_entrypoint_matrix_unstarted_stream() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);

    // 1. Cannot pause unstarted stream
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    // 2. Cannot resume unstarted stream
    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());

    // 3. Cannot cancel unstarted stream
    let res_cancel = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.cancel_stream(&stream_id);
    }));
    assert!(res_cancel.is_err());

    // 4. Cannot stop unstarted stream
    let res_stop = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.stop_stream(&stream_id);
    }));
    assert!(res_stop.is_err());

    // 5. Settle unstarted stream returns 0
    assert_eq!(ctx.client.settle_stream(&stream_id), 0);

    // 6. Cannot archive stream with balance > 0
    let res_archive = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.archive_stream(&stream_id);
    }));
    assert!(res_archive.is_err());
}

#[test]
fn test_entrypoint_matrix_active_stream() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    // 1. Cannot re-start active stream
    let res_start = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.start_stream(&stream_id);
    }));
    assert!(res_start.is_err());

    // 2. Cannot resume active (non-paused) stream
    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());

    // 3. Cannot archive active stream
    let res_archive = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.archive_stream(&stream_id);
    }));
    assert!(res_archive.is_err());

    // 4. Settle works
    ctx.advance_time(5);
    assert_eq!(ctx.client.settle_stream(&stream_id), 50);

    // 5. Pause works
    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);
    let info = ctx.snapshot(stream_id);
    assert!(info.is_active);
    assert!(info.paused_at > 0);
}

#[test]
fn test_entrypoint_matrix_paused_stream() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);
    ctx.advance_time(5);
    ctx.client.pause_stream(&stream_id);

    // 1. Cannot pause already paused stream
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    // 2. Cannot start already active/paused stream
    let res_start = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.start_stream(&stream_id);
    }));
    assert!(res_start.is_err());

    // 3. Cannot archive active/paused stream
    let res_archive = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.archive_stream(&stream_id);
    }));
    assert!(res_archive.is_err());

    // 4. Settle yields 0
    ctx.advance_time(10);
    assert_eq!(ctx.client.settle_stream(&stream_id), 0);

    // 5. Resume works
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.snapshot(stream_id).paused_at, 0);
}

#[test]
fn test_entrypoint_matrix_stopped_terminal_stream() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);
    ctx.advance_time(5);
    ctx.client.stop_stream(&stream_id);

    // 1. Cannot start stopped stream
    let res_start = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.start_stream(&stream_id);
    }));
    assert!(res_start.is_err());

    // 2. Cannot pause stopped stream
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    // 3. Cannot resume stopped stream
    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());

    // 4. Cannot stop already stopped stream
    let res_stop = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.stop_stream(&stream_id);
    }));
    assert!(res_stop.is_err());

    // 5. Cannot cancel already stopped stream
    let res_cancel = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.cancel_stream(&stream_id);
    }));
    assert!(res_cancel.is_err());

    // 6. Settle returns 0
    ctx.advance_time(20);
    assert_eq!(ctx.client.settle_stream(&stream_id), 0);
}

#[test]
fn test_entrypoint_matrix_cancelled_terminal_stream() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);
    ctx.advance_time(5);
    ctx.client.cancel_stream(&stream_id);

    // 1. Cannot start cancelled stream
    let res_start = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.start_stream(&stream_id);
    }));
    assert!(res_start.is_err());

    // 2. Cannot pause cancelled stream
    let res_pause = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.pause_stream(&stream_id);
    }));
    assert!(res_pause.is_err());

    // 3. Cannot resume cancelled stream
    let res_resume = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.resume_stream(&stream_id);
    }));
    assert!(res_resume.is_err());

    // 4. Cannot cancel cancelled stream
    let res_cancel = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.cancel_stream(&stream_id);
    }));
    assert!(res_cancel.is_err());

    // 5. Cannot stop cancelled stream
    let res_stop = catch_unwind(AssertUnwindSafe(|| {
        ctx.client.stop_stream(&stream_id);
    }));
    assert!(res_stop.is_err());

    // 6. Settle returns 0
    ctx.advance_time(20);
    assert_eq!(ctx.client.settle_stream(&stream_id), 0);
}

#[test]
fn test_rapid_toggle_at_same_timestamp() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10, 1_000, 0);
    ctx.client.start_stream(&stream_id);

    ctx.advance_time(10); // t=10

    // Rapid pause and resume at same timestamp
    ctx.client.pause_stream(&stream_id);
    assert_eq!(ctx.snapshot(stream_id).balance, 900);
    assert_eq!(ctx.snapshot(stream_id).paused_at, 10);

    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.snapshot(stream_id).balance, 900);
    assert_eq!(ctx.snapshot(stream_id).paused_at, 0);
    assert_eq!(ctx.snapshot(stream_id).start_time, 10);

    ctx.client.pause_stream(&stream_id);
    assert_eq!(ctx.snapshot(stream_id).balance, 900);
    assert_eq!(ctx.snapshot(stream_id).paused_at, 10);

    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.snapshot(stream_id).balance, 900);
    assert_eq!(ctx.snapshot(stream_id).paused_at, 0);

    // Now advance time and settle
    ctx.advance_time(5); // t=15
    let amount = ctx.client.settle_stream(&stream_id);
    assert_eq!(amount, 50);
    assert_eq!(ctx.snapshot(stream_id).balance, 850);
}
