#![cfg(any(test, feature = "testutils"))]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    /// Initialize the mock token by setting 'fail' to false by default.
    pub fn init(env: Env) {
        let key = Symbol::new(&env, "fail");
        env.storage().instance().set(&key, &false);
    }

    /// Simulate a transfer. Panics if 'fail' is set to true.
    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let key = Symbol::new(&env, "fail");
        let fail = env.storage().instance().get::<_, bool>(&key).unwrap_or(false);
        if fail {
            panic!("MockToken: transfer failed (intentional simulation)");
        }
    }

    /// Set whether the transfer should fail.
    pub fn set_fail(env: Env, fail: bool) {
        let key = Symbol::new(&env, "fail");
        env.storage().instance().set(&key, &fail);
    }

    /// Return a mocked balance (always 1,000,000 for simplicity).
    pub fn balance_of(_env: Env, _address: Address) -> i128 {
        1_000_000
    }
}
