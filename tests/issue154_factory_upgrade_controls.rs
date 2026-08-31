use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, Symbol};
use streampay_contracts::factory::{StreamPayFactory, StreamPayFactoryClient};
use streampay_contracts::{
    AdminInitializedEvent, AdminTransferredEvent, Role, RoleGrantedEvent,
    RoleRevokedEvent, StreamPayContract, StreamPayContractClient,
    WasmTemplateUpdatedEvent, VERSION,
};

fn advance_ledger_time(env: &Env, seconds: u64) {
    env.ledger().with_mut(|li| {
        li.timestamp += seconds;
    });
}

fn dummy_wasm_hash(env: &Env, fill: u8) -> BytesN<32> {
    BytesN::from_array(env, &[fill; 32])
}

// -----------------------------------------------------------------------------
// StreamPayContract: Role-Based Access Control (RBAC) & Authorization Tests
// -----------------------------------------------------------------------------

#[test]
fn test_uninitialized_contract_state_is_readable_and_has_no_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    // Existing deployed state without an admin must be readable
    assert_eq!(client.get_admin(), None);
    assert_eq!(client.version(), VERSION);

    // Streams can still be created and managed normally
    let payer = Address::generate(&env);
    let recipient = Address::generate(&env);
    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);
    assert_eq!(stream_id, 1);

    let info = client.get_stream_info(&stream_id);
    assert_eq!(info.payer, payer);
    assert_eq!(info.balance, 1000);
}

#[test]
fn test_admin_initialization_succeeds_once_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Verify AdminInitializedEvent emitted in initialize invocation
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (emitting_contract, topics, data) = events.get(0).unwrap();
    assert_eq!(emitting_contract, contract_id);
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "admin_initialized"));
    let event_data: AdminInitializedEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_data.admin, admin);

    // Verify roles and admin accessor
    assert_eq!(client.get_admin(), Some(admin.clone()));
    assert!(client.has_role(&Role::Admin, &admin));
    assert!(client.has_role(&Role::UpgradeAdmin, &admin));
    assert!(client.has_role(&Role::FactoryOperator, &admin));
    assert!(client.has_role(&Role::EmergencyAdmin, &admin));
}

#[test]
fn test_admin_double_initialization_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let second_admin = Address::generate(&env);
    let res = catch_unwind(AssertUnwindSafe(|| {
        client.initialize(&second_admin);
    }));
    assert!(res.is_err(), "second initialization must panic");
}

#[test]
fn test_unauthorized_admin_transfer_fails() {
    let env = Env::default();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Initialize with admin auth
    env.mock_all_auths();
    client.initialize(&admin);

    // Disable mock auths: unauthenticated caller invokes set_admin
    env.set_auths(&[]);
    let res = catch_unwind(AssertUnwindSafe(|| {
        client.set_admin(&new_admin);
    }));
    assert!(res.is_err(), "unauthorized set_admin must fail");

    // Admin should remain unchanged
    assert_eq!(client.get_admin(), Some(admin));
}

#[test]
fn test_set_admin_same_address_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let res = catch_unwind(AssertUnwindSafe(|| {
        client.set_admin(&admin);
    }));
    assert!(res.is_err(), "transfer to same address must fail");
}

#[test]
fn test_admin_transfer_updates_roles_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    client.initialize(&admin1);
    client.set_admin(&admin2);

    // Verify AdminTransferredEvent emitted by set_admin
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_, topics, data) = events.get(0).unwrap();
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "admin_transferred"));
    let event_data: AdminTransferredEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_data.old_admin, admin1);
    assert_eq!(event_data.new_admin, admin2);

    assert_eq!(client.get_admin(), Some(admin2.clone()));
    assert!(client.has_role(&Role::Admin, &admin2));
    assert!(client.has_role(&Role::UpgradeAdmin, &admin2));
    assert!(!client.has_role(&Role::Admin, &admin1));
}

#[test]
fn test_role_grant_and_revoke_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);

    client.initialize(&admin);

    assert!(!client.has_role(&Role::UpgradeAdmin, &operator));

    // Grant role
    client.grant_role(&Role::UpgradeAdmin, &operator);

    // Verify RoleGrantedEvent
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_, topics, data) = events.get(0).unwrap();
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "role_granted"));
    let event_data: RoleGrantedEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_data.role, Role::UpgradeAdmin);
    assert_eq!(event_data.account, operator);
    assert_eq!(event_data.granter, admin);

    assert!(client.has_role(&Role::UpgradeAdmin, &operator));

    // Revoke role
    client.revoke_role(&Role::UpgradeAdmin, &operator);

    // Verify RoleRevokedEvent
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_, topics, data) = events.get(0).unwrap();
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "role_revoked"));
    let event_data: RoleRevokedEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_data.role, Role::UpgradeAdmin);
    assert_eq!(event_data.account, operator);
    assert_eq!(event_data.revoker, admin);

    assert!(!client.has_role(&Role::UpgradeAdmin, &operator));
}

#[test]
fn test_unauthorized_role_grant_and_revoke_fails() {
    let env = Env::default();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);

    env.mock_all_auths();
    client.initialize(&admin);

    // Without auth
    env.set_auths(&[]);
    let grant_res = catch_unwind(AssertUnwindSafe(|| {
        client.grant_role(&Role::UpgradeAdmin, &operator);
    }));
    assert!(grant_res.is_err(), "unauthorized grant_role must fail");

    let revoke_res = catch_unwind(AssertUnwindSafe(|| {
        client.revoke_role(&Role::UpgradeAdmin, &operator);
    }));
    assert!(revoke_res.is_err(), "unauthorized revoke_role must fail");
}

// -----------------------------------------------------------------------------
// StreamPayContract: Upgrade Control & Version Compatibility Tests
// -----------------------------------------------------------------------------

#[test]
fn test_upgrade_version_same_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);
    let current_ver = client.version();
    assert_eq!(current_ver, VERSION); // 2_000

    let wasm_hash = dummy_wasm_hash(&env, 1);

    let res_same = catch_unwind(AssertUnwindSafe(|| {
        client.upgrade(&wasm_hash, &current_ver);
    }));
    assert!(res_same.is_err(), "upgrade to same version must fail");
}

#[test]
fn test_upgrade_version_downgrade_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);
    let current_ver = client.version();

    let wasm_hash = dummy_wasm_hash(&env, 1);

    let res_downgrade = catch_unwind(AssertUnwindSafe(|| {
        client.upgrade(&wasm_hash, &(current_ver - 1));
    }));
    assert!(res_downgrade.is_err(), "downgrade must fail");
}

#[test]
fn test_unauthorized_upgrade_fails() {
    let env = Env::default();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let wasm_hash = dummy_wasm_hash(&env, 1);

    env.set_auths(&[]);
    let res = catch_unwind(AssertUnwindSafe(|| {
        client.upgrade(&wasm_hash, &3_000);
    }));
    assert!(res.is_err(), "unauthorized upgrade must fail");
}

#[test]
fn test_upgrade_as_operator_unauthorized_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let unauthorized_user = Address::generate(&env);

    client.initialize(&admin);
    let wasm_hash = dummy_wasm_hash(&env, 2);

    let res_unauth = catch_unwind(AssertUnwindSafe(|| {
        client.upgrade_as_operator(&unauthorized_user, &wasm_hash, &3_000);
    }));
    assert!(res_unauth.is_err(), "unauthorized operator must fail");
}

#[test]
fn test_upgrade_as_operator_invalid_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);

    client.initialize(&admin);
    client.grant_role(&Role::UpgradeAdmin, &operator);

    let wasm_hash = dummy_wasm_hash(&env, 2);

    let res_invalid_ver = catch_unwind(AssertUnwindSafe(|| {
        client.upgrade_as_operator(&operator, &wasm_hash, &2_000);
    }));
    assert!(res_invalid_ver.is_err(), "version <= current must fail");
}

// -----------------------------------------------------------------------------
// Migration & State Preservation Across Admin & Upgrade Operations
// -----------------------------------------------------------------------------

#[test]
fn test_stream_lifecycle_persists_across_admin_and_role_operations() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(StreamPayContract, ());
    let client = StreamPayContractClient::new(&env, &contract_id);

    let payer = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Step 1: Create and start stream before any admin is initialized
    let stream_id = client.create_stream(&payer, &recipient, &10_i128, &1000_i128, &0_u64);
    client.start_stream(&stream_id);

    // Step 2: Initialize admin mid-lifecycle
    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Step 3: Settle stream after 5 seconds
    advance_ledger_time(&env, 5);
    let amount1 = client.settle_stream(&stream_id);
    assert_eq!(amount1, 50);

    let info_after_settle = client.get_stream_info(&stream_id);
    assert_eq!(info_after_settle.balance, 950);
    assert!(info_after_settle.is_active);

    // Step 4: Transfer admin
    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);

    // Step 5: Settle stream after another 5 seconds
    advance_ledger_time(&env, 5);
    let amount2 = client.settle_stream(&stream_id);
    assert_eq!(amount2, 50);

    let info_final = client.get_stream_info(&stream_id);
    assert_eq!(info_final.balance, 900);

    // Step 6: Payer cancels stream
    advance_ledger_time(&env, 2);
    client.cancel_stream(&stream_id);

    let info_cancelled = client.get_stream_info(&stream_id);
    assert_eq!(info_cancelled.balance, 880);
    assert!(!info_cancelled.is_active);
}

// -----------------------------------------------------------------------------
// StreamPayFactory: Factory Controls, Template Versioning & Deployer Tests
// -----------------------------------------------------------------------------

#[test]
fn test_factory_initialization_and_controls() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let initial_ver = 1_000_u32;

    assert_eq!(factory.get_admin(), None);

    factory.initialize(&admin, &initial_wasm, &initial_ver);

    assert_eq!(factory.get_admin(), Some(admin.clone()));
    assert_eq!(factory.get_wasm_hash(), initial_wasm);
    assert_eq!(factory.get_template_version(), initial_ver);
    assert_eq!(factory.get_stream_count(), 0);
    assert!(factory.has_role(&Role::Admin, &admin));
    assert!(factory.has_role(&Role::FactoryOperator, &admin));
}

#[test]
fn test_factory_double_initialization_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let initial_ver = 1_000_u32;

    factory.initialize(&admin, &initial_wasm, &initial_ver);

    let res = catch_unwind(AssertUnwindSafe(|| {
        factory.initialize(&admin, &initial_wasm, &initial_ver);
    }));
    assert!(res.is_err(), "factory double initialization must fail");
}

#[test]
fn test_factory_wasm_template_update_same_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let new_wasm = dummy_wasm_hash(&env, 11);

    factory.initialize(&admin, &initial_wasm, &1_000);

    let res_same = catch_unwind(AssertUnwindSafe(|| {
        factory.set_wasm_hash(&new_wasm, &1_000);
    }));
    assert!(res_same.is_err(), "same template version must fail");
}

#[test]
fn test_factory_wasm_template_update_lower_version_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let new_wasm = dummy_wasm_hash(&env, 11);

    factory.initialize(&admin, &initial_wasm, &1_000);

    let res_lower = catch_unwind(AssertUnwindSafe(|| {
        factory.set_wasm_hash(&new_wasm, &999);
    }));
    assert!(res_lower.is_err(), "lower template version must fail");
}

#[test]
fn test_factory_wasm_template_update_succeeds_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let new_wasm = dummy_wasm_hash(&env, 11);

    factory.initialize(&admin, &initial_wasm, &1_000);
    factory.set_wasm_hash(&new_wasm, &2_000);

    // Verify WasmTemplateUpdatedEvent emitted by set_wasm_hash
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_, topics, data) = events.get(0).unwrap();
    let topic0: Symbol = soroban_sdk::FromVal::from_val(&env, &topics.get(0).unwrap());
    assert_eq!(topic0, Symbol::new(&env, "wasm_template_updated"));
    let event_data: WasmTemplateUpdatedEvent = soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(event_data.old_wasm_hash, initial_wasm);
    assert_eq!(event_data.new_wasm_hash, new_wasm);
    assert_eq!(event_data.old_version, 1_000);
    assert_eq!(event_data.new_version, 2_000);
    assert_eq!(event_data.updater, admin);

    assert_eq!(factory.get_wasm_hash(), new_wasm);
    assert_eq!(factory.get_template_version(), 2_000);
}

#[test]
fn test_factory_operator_delegated_template_update() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let unauthorized = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let new_wasm = dummy_wasm_hash(&env, 12);

    factory.initialize(&admin, &initial_wasm, &1_000);
    factory.grant_role(&Role::FactoryOperator, &operator);

    // Unauthorized user cannot call set_wasm_hash_as_operator
    let res_unauth = catch_unwind(AssertUnwindSafe(|| {
        factory.set_wasm_hash_as_operator(&unauthorized, &new_wasm, &2_000);
    }));
    assert!(res_unauth.is_err(), "unauthorized operator must fail");

    // Authorized operator succeeds
    factory.set_wasm_hash_as_operator(&operator, &new_wasm, &2_000);
    assert_eq!(factory.get_wasm_hash(), new_wasm);
    assert_eq!(factory.get_template_version(), 2_000);
}

#[test]
fn test_factory_admin_transfer_and_role_management() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);

    factory.initialize(&admin1, &initial_wasm, &1_000);
    factory.set_admin(&admin2);

    assert_eq!(factory.get_admin(), Some(admin2.clone()));
    assert!(factory.has_role(&Role::Admin, &admin2));
    assert!(!factory.has_role(&Role::Admin, &admin1));
}

#[test]
fn test_factory_upgrade_version_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let initial_wasm = dummy_wasm_hash(&env, 10);
    let new_wasm = dummy_wasm_hash(&env, 20);

    factory.initialize(&admin, &initial_wasm, &1_000);
    let current_ver = factory.version();
    assert_eq!(current_ver, VERSION);

    // Upgrade same or lower version must fail
    let res_same = catch_unwind(AssertUnwindSafe(|| {
        factory.upgrade(&new_wasm, &current_ver);
    }));
    assert!(res_same.is_err(), "factory upgrade same version must fail");
}

#[test]
fn test_factory_deploy_stream_invalid_params_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register(StreamPayFactory, ());
    let factory = StreamPayFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let child_wasm_hash = dummy_wasm_hash(&env, 30);
    factory.initialize(&admin, &child_wasm_hash, &1_000);

    let payer = Address::generate(&env);
    let recipient = Address::generate(&env);
    let salt = dummy_wasm_hash(&env, 99);

    // Negative rate
    let res_rate = catch_unwind(AssertUnwindSafe(|| {
        factory.deploy_stream(&payer, &recipient, &-1, &1000, &0, &salt);
    }));
    assert!(res_rate.is_err(), "negative rate must fail");

    // Zero balance
    let res_bal = catch_unwind(AssertUnwindSafe(|| {
        factory.deploy_stream(&payer, &recipient, &10, &0, &0, &salt);
    }));
    assert!(res_bal.is_err(), "zero balance must fail");
}
