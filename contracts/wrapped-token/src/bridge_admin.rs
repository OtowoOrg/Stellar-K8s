use soroban_sdk::{Address, Env};

use crate::{DataKey, WrappedToken};

pub fn require_admin(env: &Env) -> Address {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("not initialized");
    admin.require_auth();
    admin
}

pub fn require_relayer(env: &Env) -> Address {
    let relayer: Address = env
        .storage()
        .instance()
        .get(&DataKey::Relayer)
        .expect("not initialized");
    relayer.require_auth();
    relayer
}

pub fn set_relayer(env: &Env, relayer: Address) {
    require_admin(env);
    env.storage().instance().set(&DataKey::Relayer, &relayer);
}

pub fn set_paused(env: &Env, paused: bool) {
    require_admin(env);
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn ensure_not_paused(env: &Env) {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    assert!(!paused, "token is paused");
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn _contract_marker(_: &WrappedToken) {}
