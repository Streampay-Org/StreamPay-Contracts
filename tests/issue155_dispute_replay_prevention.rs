use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, IntoVal, Symbol};
use streampay_contracts::{
    DisputeResolution, DisputeResolvedEvent, StreamInfo, StreamPayContract, StreamPayContractClient,
};

fn setup<'a>(
    env: &'a Env,
    rate: i128,
    balance: i128,
    end_time: u64,
) -> (StreamPayContractClient<'a>, Address, Address, u32) {
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(env, &contract_id);
    let payer = Address::generate(env);
    let recipient = Address::generate(env);
    let stream_id = client.create_stream(&payer, &recipient, &rate, &balance, &end_time);
    client.start_stream(&stream_id);
    (client, payer, recipient, stream_id)
}

fn at(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|ledger| ledger.timestamp = timestamp);
}

fn snapshot(client: &StreamPayContractClient<'_>, stream_id: u32) -> StreamInfo {
    client.get_stream_info(&stream_id)
}

#[test]
fn test_issue155_resolution_consumed_once_and_replay_rejected() {
    let env = Env::default();
    let (client, payer, recipient, stream_id) = setup(&env, 10, 1_000, 0);
    let arbitrator = Address::generate(&env);

    let res_bytes = [42u8; 32];
    let resolution_id = BytesN::from_array(&env, &res_bytes);

    assert!(!client.is_resolution_consumed(&resolution_id));

    // Resolve dispute: 700 to recipient, 300 to payer
    client.resolve_dispute(
        &stream_id,
        &resolution_id,
        &arbitrator,
        &700_i128,
        &300_i128,
    );

    // Verify DisputeResolvedEvent emitted by resolve_dispute
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_, topics, data) = events.get(0).unwrap();
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    let topic1: u32 = soroban_sdk::FromVal::from_val(&env, &topics.get(1).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "dispute_resolved"));
    assert_eq!(topic1, stream_id);

    let event_payload: DisputeResolvedEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_payload.stream_id, stream_id);
    assert_eq!(event_payload.resolution_id, resolution_id);
    assert_eq!(event_payload.arbitrator, arbitrator);
    assert_eq!(event_payload.recipient_amount, 700);
    assert_eq!(event_payload.payer_amount, 300);

    // Invariant 1: resolution is consumed
    assert!(client.is_resolution_consumed(&resolution_id));

    // Verify persisted resolution record
    let res_info = client.get_resolution_info(&resolution_id);
    assert_eq!(
        res_info,
        DisputeResolution {
            stream_id,
            arbitrator: arbitrator.clone(),
            recipient_amount: 700,
            payer_amount: 300,
            resolved_at: env.ledger().timestamp(),
        }
    );

    // Stream state is terminalized
    let info = snapshot(&client, stream_id);
    assert_eq!(info.balance, 0);
    assert!(!info.is_active);
    assert_eq!(info.paused_at, 0);
    assert_eq!(info.payer, payer);
    assert_eq!(info.recipient, recipient);
    assert_eq!(event_payload.stream_id, stream_id);
    assert_eq!(event_payload.resolution_id, resolution_id);
    assert_eq!(event_payload.arbitrator, arbitrator);
    assert_eq!(event_payload.recipient_amount, 700);
    assert_eq!(event_payload.payer_amount, 300);

    // Replay attack with same resolution_id must be rejected
    let replay_same_stream = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(
            &stream_id,
            &resolution_id,
            &arbitrator,
            &700_i128,
            &300_i128,
        );
    }));
    assert!(
        replay_same_stream.is_err(),
        "Replaying same resolution_id on same stream must fail"
    );

    // Replaying same resolution_id on a DIFFERENT stream must also be rejected
    let payer2 = Address::generate(&env);
    let recipient2 = Address::generate(&env);
    let stream_id2 = client.create_stream(&payer2, &recipient2, &10_i128, &1_000_i128, &0_u64);
    client.start_stream(&stream_id2);

    let replay_different_stream = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(
            &stream_id2,
            &resolution_id,
            &arbitrator,
            &700_i128,
            &300_i128,
        );
    }));
    assert!(
        replay_different_stream.is_err(),
        "Replaying same resolution_id across different streams must fail"
    );
}

#[test]
fn test_issue155_conflict_rejection_and_value_conservation() {
    let env = Env::default();
    let (client, _, _, stream_id) = setup(&env, 10, 1_000, 0);
    let arbitrator = Address::generate(&env);

    // 1. Under-allocation (sum < balance) -> conflict
    let res_under = BytesN::from_array(&env, &[101u8; 32]);
    let under_err = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(&stream_id, &res_under, &arbitrator, &400_i128, &500_i128);
    }));
    assert!(under_err.is_err(), "Under-allocation must be rejected");
    assert!(!client.is_resolution_consumed(&res_under));

    // 2. Over-allocation (sum > balance) -> conflict
    let res_over = BytesN::from_array(&env, &[102u8; 32]);
    let over_err = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(&stream_id, &res_over, &arbitrator, &600_i128, &500_i128);
    }));
    assert!(over_err.is_err(), "Over-allocation must be rejected");
    assert!(!client.is_resolution_consumed(&res_over));

    // 3. Negative amounts -> conflict
    let res_neg = BytesN::from_array(&env, &[103u8; 32]);
    let neg_err = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(&stream_id, &res_neg, &arbitrator, &-50_i128, &1_050_i128);
    }));
    assert!(
        neg_err.is_err(),
        "Negative recipient amount must be rejected"
    );
    assert!(!client.is_resolution_consumed(&res_neg));

    // 4. Stream state remains uncorrupted
    let info = snapshot(&client, stream_id);
    assert_eq!(info.balance, 1_000);
    assert!(info.is_active);
}

#[test]
fn test_issue155_concurrent_submissions_cannot_both_settle() {
    let env = Env::default();
    let (client, _, _, stream_id) = setup(&env, 10, 2_000, 0);
    let arbitrator = Address::generate(&env);

    let res_a = BytesN::from_array(&env, &[111u8; 32]);
    let res_b = BytesN::from_array(&env, &[222u8; 32]);

    // First resolution wins
    client.resolve_dispute(&stream_id, &res_a, &arbitrator, &1_500_i128, &500_i128);
    assert!(client.is_resolution_consumed(&res_a));

    // Concurrent/racing second resolution targeting the same stream cannot settle
    let second_res_err = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(&stream_id, &res_b, &arbitrator, &1_000_i128, &1_000_i128);
    }));
    assert!(
        second_res_err.is_err(),
        "Second resolution after terminal dispute settlement must fail"
    );
    assert!(!client.is_resolution_consumed(&res_b));

    // Settling after dispute must return 0 and move no value
    let settle_amount = client.settle_stream(&stream_id);
    assert_eq!(settle_amount, 0);

    let final_info = snapshot(&client, stream_id);
    assert_eq!(final_info.balance, 0);
    assert!(!final_info.is_active);
}

#[test]
fn test_issue155_unauthorized_arbitrator_rejected() {
    let env = Env::default();
    // Do NOT call env.mock_all_auths() globally; we want true auth checking
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let payer = Address::generate(&env);
    let recipient = Address::generate(&env);
    let arbitrator = Address::generate(&env);

    // Authorize payer for create_stream
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &payer,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_stream",
            args: (payer.clone(), recipient.clone(), 10_i128, 1_000_i128, 0_u64).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1_000_i128, &0_u64);

    let resolution_id = BytesN::from_array(&env, &[88u8; 32]);

    // Calling resolve_dispute without arbitrator's auth must fail require_auth
    let unauthed_result = catch_unwind(AssertUnwindSafe(|| {
        client.resolve_dispute(
            &stream_id,
            &resolution_id,
            &arbitrator,
            &500_i128,
            &500_i128,
        );
    }));
    assert!(
        unauthed_result.is_err(),
        "Unauthorized arbitrator invocation must be rejected"
    );
    assert!(!client.is_resolution_consumed(&resolution_id));
}

#[test]
fn test_issue155_dispute_on_partially_settled_stream() {
    let env = Env::default();
    let (client, _, _, stream_id) = setup(&env, 10, 1_000, 0);
    let arbitrator = Address::generate(&env);

    // Advance 20s and settle 200 units
    at(&env, 20);
    let settled = client.settle_stream(&stream_id);
    assert_eq!(settled, 200);

    let mid_info = snapshot(&client, stream_id);
    assert_eq!(mid_info.balance, 800);

    // Resolve dispute on the remaining 800 units
    let res_id = BytesN::from_array(&env, &[99u8; 32]);
    client.resolve_dispute(&stream_id, &res_id, &arbitrator, &600_i128, &200_i128);

    let final_info = snapshot(&client, stream_id);
    assert_eq!(final_info.balance, 0);
    assert!(!final_info.is_active);

    // Invariant: settled (200) + recipient_amount (600) + payer_amount (200) == initial (1000)
    assert_eq!(settled + 600 + 200, 1_000);
}

#[test]
fn test_issue155_dispute_on_paused_stream_clears_pause_state() {
    let env = Env::default();
    let (client, _, _, stream_id) = setup(&env, 10, 1_000, 100);
    let arbitrator = Address::generate(&env);

    at(&env, 30);
    client.pause_stream(&stream_id);
    let paused_info = snapshot(&client, stream_id);
    assert_eq!(paused_info.balance, 700);
    assert_eq!(paused_info.paused_at, 30);

    let res_id = BytesN::from_array(&env, &[77u8; 32]);
    client.resolve_dispute(&stream_id, &res_id, &arbitrator, &500_i128, &200_i128);

    let resolved_info = snapshot(&client, stream_id);
    assert_eq!(resolved_info.balance, 0);
    assert!(!resolved_info.is_active);
    assert_eq!(resolved_info.paused_at, 0);

    // Resuming a dispute-resolved stream must fail
    let resume_err = catch_unwind(AssertUnwindSafe(|| {
        client.resume_stream(&stream_id);
    }));
    assert!(
        resume_err.is_err(),
        "Cannot resume a dispute-resolved stream"
    );
}
