use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Symbol, Vec as SorobanVec};
use streampay_contracts::{StreamInfo, StreamPayContract, StreamPayContractClient};

fn setup<'a>(
    env: &'a Env,
    rate: i128,
    balance: i128,
    end_time: u64,
) -> (StreamPayContractClient<'a>, u32) {
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(env, &contract_id);
    let payer = Address::generate(env);
    let recipient = Address::generate(env);
    let stream_id = client.create_stream(&payer, &recipient, &rate, &balance, &end_time);
    client.start_stream(&stream_id);
    (client, stream_id)
}

fn at(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|ledger| ledger.timestamp = timestamp);
}

fn snapshot(client: &StreamPayContractClient<'_>, stream_id: u32) -> StreamInfo {
    client.get_stream_info(&stream_id)
}

/// Reproduce the persisted shape written by the pre-fix pause implementation:
/// the pause interval was already deducted from `balance`, but `start_time`
/// remained at the pre-pause cursor while `paused_at` recorded the boundary.
fn setup_legacy_paused<'a>(env: &'a Env) -> (StreamPayContractClient<'a>, u32) {
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(env, &contract_id);
    let payer = Address::generate(env);
    let recipient = Address::generate(env);
    let stream_id = client.create_stream(&payer, &recipient, &1, &10, &10);
    client.start_stream(&stream_id);

    at(env, 2);
    client.pause_stream(&stream_id);
    let mut legacy = snapshot(&client, stream_id);
    assert_eq!(legacy.balance, 8);
    assert_eq!(legacy.start_time, 2);
    assert_eq!(legacy.paused_at, 2);

    legacy.start_time = 0;
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&(Symbol::new(env, "stream"), stream_id), &legacy);
    });

    (client, stream_id)
}

#[test]
fn issue153_control_active_boundary_matrix() {
    for (timestamp, expected_amount, expected_active) in [
        (9_u64, 9_i128, true),
        (10, 10, false),
        (11, 10, false),
        (u64::MAX, 10, false),
    ] {
        let env = Env::default();
        let (client, stream_id) = setup(&env, 1, 100, 10);
        at(&env, timestamp);
        let amount = client.settle_stream(&stream_id);
        let info = snapshot(&client, stream_id);

        assert_eq!(amount, expected_amount);
        assert_eq!(info.balance, 100 - expected_amount);
        assert_eq!(info.start_time, timestamp.min(10));
        assert_eq!(info.end_time, 10);
        assert_eq!(info.is_active, expected_active);
        assert_eq!(info.paused_at, 0);
    }
}

#[test]
fn issue153_control_terminal_call_permutations() {
    // Natural end -> repeated settle -> cancel rejection.
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);
    at(&env, 11);
    assert_eq!(client.settle_stream(&stream_id), 10);
    let terminal = snapshot(&client, stream_id);
    at(&env, 100);
    assert_eq!(client.settle_stream(&stream_id), 0);
    let rejected = catch_unwind(AssertUnwindSafe(|| client.cancel_stream(&stream_id)));
    assert!(rejected.is_err());
    let after = snapshot(&client, stream_id);
    assert_eq!(after.balance, terminal.balance);
    assert_eq!(after.start_time, terminal.start_time);
    assert_eq!(after.end_time, terminal.end_time);
    assert_eq!(after.paused_at, terminal.paused_at);
    assert_eq!(after.is_active, terminal.is_active);

    // Cancel -> settle.
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);
    at(&env, 9);
    client.cancel_stream(&stream_id);
    let cancelled = snapshot(&client, stream_id);
    at(&env, 100);
    assert_eq!(client.settle_stream(&stream_id), 0);
    let after = snapshot(&client, stream_id);
    assert_eq!(after.balance, cancelled.balance);
    assert_eq!(after.start_time, cancelled.start_time);
    assert_eq!(after.end_time, cancelled.end_time);
    assert_eq!(after.paused_at, cancelled.paused_at);
    assert_eq!(after.is_active, cancelled.is_active);

    // Settle -> settle -> cancel before the configured end.
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);
    at(&env, 2);
    assert_eq!(client.settle_stream(&stream_id), 2);
    assert_eq!(client.settle_stream(&stream_id), 0);
    at(&env, 3);
    client.cancel_stream(&stream_id);
    let settled_then_cancelled = snapshot(&client, stream_id);
    assert_eq!(settled_then_cancelled.balance, 97);
    assert!(!settled_then_cancelled.is_active);
}

#[test]
fn issue153_control_authorization_and_permissionless_settle() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);
    at(&env, 1);
    let before = snapshot(&client, stream_id);

    // Construction/start were authorized through mock_all_auths. Disable it for
    // the negative controls and prove failed calls leave state unchanged.
    env.set_auths(&[]);
    let cancel = catch_unwind(AssertUnwindSafe(|| client.cancel_stream(&stream_id)));
    assert!(cancel.is_err());
    let after_cancel = snapshot(&client, stream_id);
    assert_eq!(after_cancel.balance, before.balance);
    assert_eq!(after_cancel.start_time, before.start_time);
    assert_eq!(after_cancel.end_time, before.end_time);
    assert_eq!(after_cancel.paused_at, before.paused_at);
    assert_eq!(after_cancel.is_active, before.is_active);

    let pause = catch_unwind(AssertUnwindSafe(|| client.pause_stream(&stream_id)));
    assert!(pause.is_err());
    let after_pause = snapshot(&client, stream_id);
    assert_eq!(after_pause.balance, before.balance);
    assert_eq!(after_pause.start_time, before.start_time);
    assert_eq!(after_pause.end_time, before.end_time);
    assert_eq!(after_pause.paused_at, before.paused_at);
    assert_eq!(after_pause.is_active, before.is_active);

    // settle_stream intentionally remains permissionless.
    assert_eq!(client.settle_stream(&stream_id), 1);
}

#[test]
fn issue153_red_pause_after_natural_end_caps_and_terminalizes() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 2, 1);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let info = snapshot(&client, stream_id);
    eprintln!(
        "late pause: balance={} start={} end={} active={} paused={}",
        info.balance, info.start_time, info.end_time, info.is_active, info.paused_at
    );

    assert_eq!(info.balance, 1, "only [0, end_time] is eligible");
    assert_eq!(
        info.start_time, 1,
        "accrual cursor must reach the capped boundary"
    );
    assert!(
        !info.is_active,
        "pause at/after natural end must be terminal"
    );
    assert_eq!(
        info.paused_at, 0,
        "terminal state must not retain a pause marker"
    );
}

#[test]
fn issue153_red_pause_then_settle_does_not_pay_twice() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 10, 10);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_pause = snapshot(&client, stream_id);
    let second = client.settle_stream(&stream_id);
    let after_settle = snapshot(&client, stream_id);
    eprintln!(
        "pause->settle: pause(balance={},start={},paused={}); second={}; final(balance={},start={},paused={})",
        after_pause.balance,
        after_pause.start_time,
        after_pause.paused_at,
        second,
        after_settle.balance,
        after_settle.start_time,
        after_settle.paused_at
    );

    assert_eq!(second, 0, "the [0,2] interval was already paid by pause");
    assert_eq!(after_settle.balance, after_pause.balance);
}

#[test]
fn issue153_red_pause_then_cancel_does_not_charge_twice_or_leave_pause() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 10, 10);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_pause = snapshot(&client, stream_id);
    client.cancel_stream(&stream_id);
    let after_cancel = snapshot(&client, stream_id);
    at(&env, 3);
    let after_terminal_settle = client.settle_stream(&stream_id);
    eprintln!(
        "pause->cancel->settle: pause(balance={},start={},end={},paused={}); cancel(balance={},start={},end={},active={},paused={}); terminal_settle={}",
        after_pause.balance,
        after_pause.start_time,
        after_pause.end_time,
        after_pause.paused_at,
        after_cancel.balance,
        after_cancel.start_time,
        after_cancel.end_time,
        after_cancel.is_active,
        after_cancel.paused_at,
        after_terminal_settle
    );

    assert_eq!(after_terminal_settle, 0);
    assert_eq!(after_cancel.balance, after_pause.balance);
    assert!(!after_cancel.is_active);
    assert_eq!(after_cancel.paused_at, 0);
}

#[test]
fn issue153_red_pause_settle_cancel_respects_earned_time() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 20, 10);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_pause = snapshot(&client, stream_id);
    let second = client.settle_stream(&stream_id);
    at(&env, 3);
    client.cancel_stream(&stream_id);
    let final_info = snapshot(&client, stream_id);
    let recipient_accounted = 20 - final_info.balance;
    eprintln!(
        "pause->settle->cancel: pause_balance={}; settle={}; final_balance={}; accounted={}; eligible=2",
        after_pause.balance, second, final_info.balance, recipient_accounted
    );

    assert_eq!(
        recipient_accounted, 2,
        "recipient_accounted must not exceed rate * eligible_active_seconds"
    );
    assert_eq!(final_info.paused_at, 0);
}

#[test]
fn issue153_red_unlimited_pause_freezes_very_late_settle() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 200, 0);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_pause = snapshot(&client, stream_id);
    at(&env, 100);
    let second = client.settle_stream(&stream_id);
    let after_settle = snapshot(&client, stream_id);
    eprintln!(
        "unlimited pause->settle: pause_balance={}; second={}; final_balance={}; start={}; paused={}",
        after_pause.balance,
        second,
        after_settle.balance,
        after_settle.start_time,
        after_settle.paused_at
    );

    assert_eq!(second, 0, "[2,100] is paused and [0,2] was already paid");
    assert_eq!(after_settle.balance, after_pause.balance);
    assert!(after_settle.is_active);
}

#[test]
fn issue153_red_paused_batch_settle_does_not_replay_interval() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 20, 10);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_pause = snapshot(&client, stream_id);
    let mut ids = SorobanVec::new(&env);
    ids.push_back(stream_id);
    let amounts = client.batch_settle(&ids);
    let after_batch = snapshot(&client, stream_id);
    eprintln!(
        "pause->batch: pause_balance={}; batch_amount={}; final_balance={}",
        after_pause.balance,
        amounts.get(0).unwrap(),
        after_batch.balance
    );

    assert_eq!(amounts.get(0).unwrap(), 0);
    assert_eq!(after_batch.balance, after_pause.balance);
}

#[test]
fn issue153_red_resume_after_end_cannot_resurrect_accrual() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let after_first_pause = snapshot(&client, stream_id);
    at(&env, 11);
    client.resume_stream(&stream_id);
    let after_resume = snapshot(&client, stream_id);
    at(&env, 12);
    let late_pause = catch_unwind(AssertUnwindSafe(|| client.pause_stream(&stream_id)));
    let after_second_pause = snapshot(&client, stream_id);
    eprintln!(
        "pause->late resume->rejected pause: first_balance={}; resume(start={},active={},paused={}); final(balance={},start={},end={},active={},paused={})",
        after_first_pause.balance,
        after_resume.start_time,
        after_resume.is_active,
        after_resume.paused_at,
        after_second_pause.balance,
        after_second_pause.start_time,
        after_second_pause.end_time,
        after_second_pause.is_active,
        after_second_pause.paused_at
    );

    assert!(
        late_pause.is_err(),
        "terminal stream cannot be paused again"
    );
    assert_eq!(after_second_pause.balance, after_first_pause.balance);
    assert!(!after_second_pause.is_active);
    assert_eq!(after_second_pause.paused_at, 0);
}

#[test]
fn issue153_red_pause_at_natural_end_terminalizes() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 20, 10);
    at(&env, 10);
    client.pause_stream(&stream_id);
    let info = snapshot(&client, stream_id);
    eprintln!(
        "exact-end pause: balance={} start={} end={} active={} paused={}",
        info.balance, info.start_time, info.end_time, info.is_active, info.paused_at
    );

    assert_eq!(info.balance, 10);
    assert_eq!(info.start_time, 10);
    assert!(!info.is_active);
    assert_eq!(info.paused_at, 0);
}

#[test]
fn issue153_red_late_pause_with_small_balance_preserves_unearned_value() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 10, 15, 1);
    at(&env, 2);
    client.pause_stream(&stream_id);
    let info = snapshot(&client, stream_id);
    let recipient_accounted = 15 - info.balance;
    eprintln!(
        "small-balance late pause: accounted={}; balance={}; start={}; end={}; active={}; paused={}",
        recipient_accounted,
        info.balance,
        info.start_time,
        info.end_time,
        info.is_active,
        info.paused_at
    );

    assert_eq!(
        recipient_accounted, 10,
        "recipient_accounted must be capped by rate * eligible_active_seconds"
    );
    assert_eq!(info.balance, 5);
    assert!(!info.is_active);
    assert_eq!(info.paused_at, 0);
}

#[test]
fn issue153_red_legacy_paused_settle_does_not_repay_preupgrade_interval() {
    let env = Env::default();
    let (client, stream_id) = setup_legacy_paused(&env);

    at(&env, 3);
    let amount = client.settle_stream(&stream_id);
    let info = snapshot(&client, stream_id);

    assert_eq!(amount, 0, "the legacy pause already paid [0,2]");
    assert_eq!(info.balance, 8, "settle must preserve unearned custody");
    assert_eq!(info.start_time, 2, "settle must normalize the old cursor");
    assert_eq!(info.paused_at, 2);
    assert!(info.is_active);
}

#[test]
fn issue153_red_legacy_paused_batch_does_not_repay_preupgrade_interval() {
    let env = Env::default();
    let (client, stream_id) = setup_legacy_paused(&env);

    at(&env, 3);
    let mut ids = SorobanVec::new(&env);
    ids.push_back(stream_id);
    let amounts = client.batch_settle(&ids);
    let info = snapshot(&client, stream_id);

    assert_eq!(amounts.get(0).unwrap(), 0);
    assert_eq!(info.balance, 8, "batch must preserve unearned custody");
    assert_eq!(info.start_time, 2, "batch must normalize the old cursor");
    assert_eq!(info.paused_at, 2);
    assert!(info.is_active);
}

#[test]
fn issue153_red_legacy_paused_cancel_does_not_repay_preupgrade_interval() {
    let env = Env::default();
    let (client, stream_id) = setup_legacy_paused(&env);

    at(&env, 3);
    client.cancel_stream(&stream_id);
    let info = snapshot(&client, stream_id);

    assert_eq!(info.balance, 8, "cancel must preserve unearned custody");
    assert_eq!(info.start_time, 2, "cancel must normalize the old cursor");
    assert_eq!(info.end_time, 3);
    assert_eq!(info.paused_at, 0);
    assert!(!info.is_active);
}

#[test]
fn issue153_red_stop_after_natural_end_settles_exact_entitlement() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);

    at(&env, 11);
    client.stop_stream(&stream_id);
    let stopped = snapshot(&client, stream_id);

    assert_eq!(stopped.balance, 90, "recipient earned exactly [0,10]");
    assert_eq!(stopped.start_time, 10);
    assert_eq!(stopped.end_time, 10, "natural end remains authoritative");
    assert_eq!(stopped.paused_at, 0);
    assert!(!stopped.is_active);

    at(&env, 100);
    assert_eq!(client.settle_stream(&stream_id), 0);
    assert_eq!(snapshot(&client, stream_id).balance, 90);
}

#[test]
fn issue153_red_partial_settle_then_stop_settles_only_new_interval() {
    let env = Env::default();
    let (client, stream_id) = setup(&env, 1, 100, 10);

    at(&env, 2);
    assert_eq!(client.settle_stream(&stream_id), 2);
    at(&env, 3);
    client.stop_stream(&stream_id);
    let stopped = snapshot(&client, stream_id);

    assert_eq!(
        stopped.balance, 97,
        "stop must settle the new [2,3] interval"
    );
    assert_eq!(stopped.start_time, 3);
    assert_eq!(stopped.end_time, 3);
    assert_eq!(stopped.paused_at, 0);
    assert!(!stopped.is_active);

    at(&env, 100);
    assert_eq!(client.settle_stream(&stream_id), 0);
    assert_eq!(snapshot(&client, stream_id).balance, 97);
}
