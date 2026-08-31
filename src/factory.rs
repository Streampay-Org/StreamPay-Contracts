//! Factory contract for deploying and tracking child stream contracts.
//!
//! Provides: initialize, set_admin, grant_role, revoke_role, has_role,
//! set_wasm_hash, get_wasm_hash, get_template_version, deploy_stream,
//! get_stream_address, get_stream_count, upgrade, version.

use soroban_sdk::{
    contract, contractimpl, Address, BytesN, Env, Symbol, Vec,
};

use crate::{
    Role, StreamDeployedEvent, WasmTemplateUpdatedEvent, AdminTransferredEvent,
    RoleGrantedEvent, RoleRevokedEvent, ContractUpgradedEvent, VERSION,
    STREAM_TTL_EXTEND, STREAM_TTL_THRESHOLD,
};

#[contract]
pub struct StreamPayFactory;

#[contractimpl]
impl StreamPayFactory {
    /// Initialize the factory with an admin, child contract WASM template, and template version.
    pub fn initialize(
        env: Env,
        admin: Address,
        child_wasm_hash: BytesN<32>,
        template_version: u32,
    ) {
        if has_factory_admin(&env) {
            panic!("factory already initialized");
        }
        if template_version == 0 {
            panic!("template version must be positive");
        }
        set_factory_admin_storage(&env, &admin);
        set_factory_role_storage(&env, Role::Admin, &admin, true);
        set_factory_role_storage(&env, Role::FactoryOperator, &admin, true);
        set_factory_role_storage(&env, Role::UpgradeAdmin, &admin, true);
        set_factory_role_storage(&env, Role::EmergencyAdmin, &admin, true);

        set_factory_child_wasm(&env, &child_wasm_hash);
        set_factory_child_version(&env, template_version);
        set_factory_next_stream_id(&env, 1);
        extend_instance_ttl(&env);

        emit_factory_initialized(&env, &admin, &child_wasm_hash, template_version);
    }

    /// Get factory admin.
    pub fn get_admin(env: Env) -> Option<Address> {
        get_factory_admin_storage(&env)
    }

    /// Transfer factory admin. Caller must be current admin.
    pub fn set_admin(env: Env, new_admin: Address) {
        let current_admin = require_factory_admin(&env);
        if current_admin == new_admin {
            panic!("new admin must be different");
        }
        set_factory_admin_storage(&env, &new_admin);
        set_factory_role_storage(&env, Role::Admin, &new_admin, true);
        set_factory_role_storage(&env, Role::FactoryOperator, &new_admin, true);
        set_factory_role_storage(&env, Role::UpgradeAdmin, &new_admin, true);
        set_factory_role_storage(&env, Role::EmergencyAdmin, &new_admin, true);
        set_factory_role_storage(&env, Role::Admin, &current_admin, false);
        extend_instance_ttl(&env);

        emit_admin_transferred(&env, &current_admin, &new_admin);
    }

    /// Grant a role to an account in the factory.
    pub fn grant_role(env: Env, role: Role, account: Address) {
        let granter = require_factory_admin(&env);
        set_factory_role_storage(&env, role, &account, true);
        extend_instance_ttl(&env);
        emit_role_granted(&env, role, &account, &granter);
    }

    /// Revoke a role from an account in the factory.
    pub fn revoke_role(env: Env, role: Role, account: Address) {
        let revoker = require_factory_admin(&env);
        set_factory_role_storage(&env, role, &account, false);
        extend_instance_ttl(&env);
        emit_role_revoked(&env, role, &account, &revoker);
    }

    /// Check if account has role in factory.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        if let Some(admin) = get_factory_admin_storage(&env) {
            if admin == account {
                return true;
            }
        }
        get_factory_role_storage(&env, role, &account)
    }

    /// Update child contract WASM hash template with explicit version bump.
    /// Caller must be factory admin.
    pub fn set_wasm_hash(env: Env, new_wasm_hash: BytesN<32>, new_version: u32) {
        let updater = require_factory_admin(&env);
        let old_version = get_factory_child_version(&env);
        if new_version <= old_version {
            panic!("new template version must be strictly greater than current version");
        }
        let old_wasm = get_factory_child_wasm(&env);
        set_factory_child_wasm(&env, &new_wasm_hash);
        set_factory_child_version(&env, new_version);
        extend_instance_ttl(&env);

        emit_wasm_template_updated(
            &env,
            &old_wasm,
            &new_wasm_hash,
            old_version,
            new_version,
            &updater,
        );
    }

    /// Update child contract WASM hash as authorized operator with FactoryOperator role.
    pub fn set_wasm_hash_as_operator(
        env: Env,
        operator: Address,
        new_wasm_hash: BytesN<32>,
        new_version: u32,
    ) {
        operator.require_auth();
        if !Self::has_role(env.clone(), Role::FactoryOperator, operator.clone()) {
            panic!("unauthorized");
        }
        let old_version = get_factory_child_version(&env);
        if new_version <= old_version {
            panic!("new template version must be strictly greater than current version");
        }
        let old_wasm = get_factory_child_wasm(&env);
        set_factory_child_wasm(&env, &new_wasm_hash);
        set_factory_child_version(&env, new_version);
        extend_instance_ttl(&env);

        emit_wasm_template_updated(
            &env,
            &old_wasm,
            &new_wasm_hash,
            old_version,
            new_version,
            &operator,
        );
    }

    /// Get current child template WASM hash.
    pub fn get_wasm_hash(env: Env) -> BytesN<32> {
        get_factory_child_wasm(&env)
    }

    /// Get current child template version.
    pub fn get_template_version(env: Env) -> u32 {
        get_factory_child_version(&env)
    }

    /// Deploy a child stream contract via factory deployer.
    /// Payer must authorize.
    pub fn deploy_stream(
        env: Env,
        payer: Address,
        recipient: Address,
        rate_per_second: i128,
        initial_balance: i128,
        end_time: u64,
        salt: BytesN<32>,
    ) -> (u32, Address) {
        payer.require_auth();
        if rate_per_second <= 0 || initial_balance <= 0 {
            panic!("rate and balance must be positive");
        }
        let child_wasm_hash = get_factory_child_wasm(&env);
        let constructor_args: Vec<soroban_sdk::Val> = Vec::new(&env);
        let child_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(child_wasm_hash, constructor_args);

        let stream_id = get_factory_next_stream_id(&env);
        set_factory_stream_address(&env, stream_id, &child_address);
        set_factory_next_stream_id(&env, stream_id + 1);
        extend_factory_stream_ttl(&env, stream_id);
        extend_instance_ttl(&env);

        emit_stream_deployed(
            &env,
            stream_id,
            &child_address,
            &payer,
            &recipient,
            rate_per_second,
            initial_balance,
            end_time,
        );

        (stream_id, child_address)
    }

    /// Get child contract address for a stream ID.
    pub fn get_stream_address(env: Env, stream_id: u32) -> Address {
        get_factory_stream_address(&env, stream_id)
    }

    /// Get total streams deployed by factory.
    pub fn get_stream_count(env: Env) -> u32 {
        get_factory_next_stream_id(&env).saturating_sub(1)
    }

    /// Upgrade factory contract code with explicit version bump.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>, new_version: u32) {
        let updater = require_factory_admin(&env);
        let current_version = get_factory_version(&env);
        if new_version <= current_version {
            panic!("new version must be strictly greater than current version");
        }
        set_factory_version(&env, new_version);
        env.deployer().update_current_contract_wasm(new_wasm_hash.clone());
        extend_instance_ttl(&env);
        emit_contract_upgraded(&env, current_version, new_version, &new_wasm_hash, &updater);
    }

    /// Factory contract version.
    pub fn version(env: Env) -> u32 {
        get_factory_version(&env)
    }
}

fn emit_admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
    let topics = (Symbol::new(env, "admin_transferred"), Symbol::new(env, "admin"));
    let data = AdminTransferredEvent {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    };
    env.events().publish(topics, data);
}

fn emit_role_granted(env: &Env, role: Role, account: &Address, granter: &Address) {
    let topics = (Symbol::new(env, "role_granted"), role as u32);
    let data = RoleGrantedEvent {
        role,
        account: account.clone(),
        granter: granter.clone(),
    };
    env.events().publish(topics, data);
}

fn emit_role_revoked(env: &Env, role: Role, account: &Address, revoker: &Address) {
    let topics = (Symbol::new(env, "role_revoked"), role as u32);
    let data = RoleRevokedEvent {
        role,
        account: account.clone(),
        revoker: revoker.clone(),
    };
    env.events().publish(topics, data);
}

fn emit_contract_upgraded(
    env: &Env,
    old_version: u32,
    new_version: u32,
    new_wasm_hash: &BytesN<32>,
    _caller: &Address,
) {
    let topics = (Symbol::new(env, "contract_upgraded"), new_version);
    let data = ContractUpgradedEvent {
        old_version,
        new_version,
        new_wasm_hash: new_wasm_hash.clone(),
    };
    env.events().publish(topics, data);
}

fn emit_wasm_template_updated(
    env: &Env,
    old_wasm_hash: &BytesN<32>,
    new_wasm_hash: &BytesN<32>,
    old_version: u32,
    new_version: u32,
    updater: &Address,
) {
    let topics = (Symbol::new(env, "wasm_template_updated"), new_version);
    let data = WasmTemplateUpdatedEvent {
        old_wasm_hash: old_wasm_hash.clone(),
        new_wasm_hash: new_wasm_hash.clone(),
        old_version,
        new_version,
        updater: updater.clone(),
    };
    env.events().publish(topics, data);
}

#[allow(clippy::too_many_arguments)]
fn emit_stream_deployed(
    env: &Env,
    stream_id: u32,
    contract_address: &Address,
    payer: &Address,
    recipient: &Address,
    rate_per_second: i128,
    initial_balance: i128,
    end_time: u64,
) {
    let topics = (Symbol::new(env, "stream_deployed"), stream_id);
    let data = StreamDeployedEvent {
        stream_id,
        contract_address: contract_address.clone(),
        payer: payer.clone(),
        recipient: recipient.clone(),
        rate_per_second,
        initial_balance,
        end_time,
    };
    env.events().publish(topics, data);
}

fn emit_factory_initialized(
    env: &Env,
    admin: &Address,
    child_wasm_hash: &BytesN<32>,
    template_version: u32,
) {
    let topics = (Symbol::new(env, "factory_initialized"), template_version);
    let data = WasmTemplateUpdatedEvent {
        old_wasm_hash: child_wasm_hash.clone(),
        new_wasm_hash: child_wasm_hash.clone(),
        old_version: 0,
        new_version: template_version,
        updater: admin.clone(),
    };
    env.events().publish(topics, data);
}

fn get_factory_admin_storage(env: &Env) -> Option<Address> {
    let key = Symbol::new(env, "f_admin");
    env.storage().instance().get(&key)
}

fn set_factory_admin_storage(env: &Env, admin: &Address) {
    let key = Symbol::new(env, "f_admin");
    env.storage().instance().set(&key, admin);
}

fn has_factory_admin(env: &Env) -> bool {
    get_factory_admin_storage(env).is_some()
}

fn require_factory_admin(env: &Env) -> Address {
    let admin =
        get_factory_admin_storage(env).unwrap_or_else(|| panic!("factory admin not initialized"));
    admin.require_auth();
    admin
}

fn get_factory_role_storage(env: &Env, role: Role, account: &Address) -> bool {
    let key = (Symbol::new(env, "f_role"), role, account.clone());
    env.storage().instance().get(&key).unwrap_or(false)
}

fn set_factory_role_storage(env: &Env, role: Role, account: &Address, active: bool) {
    let key = (Symbol::new(env, "f_role"), role, account.clone());
    if active {
        env.storage().instance().set(&key, &true);
    } else {
        env.storage().instance().remove(&key);
    }
}

fn get_factory_child_wasm(env: &Env) -> BytesN<32> {
    let key = Symbol::new(env, "f_wasm");
    env.storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| panic!("child wasm hash not configured"))
}

fn set_factory_child_wasm(env: &Env, wasm_hash: &BytesN<32>) {
    let key = Symbol::new(env, "f_wasm");
    env.storage().instance().set(&key, wasm_hash);
}

fn get_factory_child_version(env: &Env) -> u32 {
    let key = Symbol::new(env, "f_tmpl_ver");
    env.storage().instance().get(&key).unwrap_or(0)
}

fn set_factory_child_version(env: &Env, ver: u32) {
    let key = Symbol::new(env, "f_tmpl_ver");
    env.storage().instance().set(&key, &ver);
}

fn get_factory_next_stream_id(env: &Env) -> u32 {
    let key = Symbol::new(env, "f_next_id");
    env.storage().instance().get(&key).unwrap_or(1)
}

fn set_factory_next_stream_id(env: &Env, id: u32) {
    let key = Symbol::new(env, "f_next_id");
    env.storage().instance().set(&key, &id);
}

fn factory_stream_key(env: &Env, stream_id: u32) -> (Symbol, u32) {
    (Symbol::new(env, "f_stream"), stream_id)
}

fn get_factory_stream_address(env: &Env, stream_id: u32) -> Address {
    let key = factory_stream_key(env, stream_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("stream not found"))
}

fn set_factory_stream_address(env: &Env, stream_id: u32, address: &Address) {
    let key = factory_stream_key(env, stream_id);
    env.storage().persistent().set(&key, address);
}

fn extend_factory_stream_ttl(env: &Env, stream_id: u32) {
    let key = factory_stream_key(env, stream_id);
    env.storage()
        .persistent()
        .extend_ttl(&key, STREAM_TTL_THRESHOLD, STREAM_TTL_EXTEND);
}

fn get_factory_version(env: &Env) -> u32 {
    let key = Symbol::new(env, "f_ver");
    env.storage().instance().get(&key).unwrap_or(VERSION)
}

fn set_factory_version(env: &Env, ver: u32) {
    let key = Symbol::new(env, "f_ver");
    env.storage().instance().set(&key, &ver);
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(crate::INSTANCE_TTL_THRESHOLD, crate::INSTANCE_TTL_EXTEND);
}
