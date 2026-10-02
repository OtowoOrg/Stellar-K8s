#![no_std]

mod bridge_admin;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, MuxedAddress, String,
};

#[contract]
pub struct WrappedToken;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Relayer,
    Paused,
    Decimals,
    Name,
    Symbol,
    TotalSupply,
    Balance(Address),
    Allowance(Address, Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowanceData {
    pub amount: i128,
    pub live_until_ledger: u32,
}

fn balance(env: &Env, account: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Balance(account.clone()))
        .unwrap_or(0)
}

fn set_balance(env: &Env, account: &Address, amount: i128) {
    assert!(amount >= 0, "negative balance");
    env.storage()
        .persistent()
        .set(&DataKey::Balance(account.clone()), &amount);
}

fn allowance(env: &Env, owner: &Address, spender: &Address) -> AllowanceData {
    env.storage()
        .persistent()
        .get(&DataKey::Allowance(owner.clone(), spender.clone()))
        .unwrap_or(AllowanceData {
            amount: 0,
            live_until_ledger: 0,
        })
}

fn require_amount(amount: i128) {
    assert!(amount >= 0, "amount must not be negative");
}

#[contractimpl]
impl WrappedToken {
    /// Initializes the wrapped asset and assigns the bridge relayer.
    pub fn initialize(
        env: Env,
        admin: Address,
        relayer: Address,
        decimals: u32,
        name: String,
        symbol: String,
    ) {
        assert!(
            !env.storage().instance().has(&DataKey::Admin),
            "already initialized"
        );
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Relayer, &relayer);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::Decimals, &decimals);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
    }

    pub fn admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }

    pub fn relayer(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Relayer)
            .expect("not initialized")
    }

    pub fn paused(env: Env) -> bool {
        bridge_admin::is_paused(&env)
    }

    pub fn set_relayer(env: Env, relayer: Address) {
        bridge_admin::set_relayer(&env, relayer);
    }

    pub fn set_paused(env: Env, paused: bool) {
        bridge_admin::set_paused(&env, paused);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        require_amount(amount);
        bridge_admin::ensure_not_paused(&env);
        bridge_admin::require_relayer(&env);
        let next = balance(&env, &to)
            .checked_add(amount)
            .expect("balance overflow");
        let supply = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0i128);
        env.storage().instance().set(
            &DataKey::TotalSupply,
            &supply.checked_add(amount).expect("supply overflow"),
        );
        set_balance(&env, &to, next);
        env.events().publish((symbol_short!("mint"), to), amount);
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }
}

#[contractimpl]
impl WrappedToken {
    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        let data = allowance(&env, &from, &spender);
        if data.live_until_ledger != 0 && data.live_until_ledger < env.ledger().sequence() {
            0
        } else {
            data.amount
        }
    }

    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        live_until_ledger: u32,
    ) {
        require_amount(amount);
        from.require_auth();
        assert!(
            live_until_ledger == 0 || live_until_ledger >= env.ledger().sequence(),
            "invalid expiration"
        );
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender.clone()),
            &AllowanceData {
                amount,
                live_until_ledger,
            },
        );
        env.events().publish(
            (symbol_short!("approve"), from, spender),
            (amount, live_until_ledger),
        );
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        balance(&env, &id)
    }

    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        require_amount(amount);
        bridge_admin::ensure_not_paused(&env);
        from.require_auth();
        let to_address = to.address();
        let from_balance = balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");
        set_balance(&env, &from, from_balance - amount);
        set_balance(
            &env,
            &to_address,
            balance(&env, &to_address)
                .checked_add(amount)
                .expect("balance overflow"),
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to_address), amount);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        require_amount(amount);
        bridge_admin::ensure_not_paused(&env);
        spender.require_auth();
        let data = allowance(&env, &from, &spender);
        assert!(
            data.live_until_ledger == 0 || data.live_until_ledger >= env.ledger().sequence(),
            "allowance expired"
        );
        assert!(data.amount >= amount, "insufficient allowance");
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender),
            &AllowanceData {
                amount: data.amount - amount,
                ..data
            },
        );
        let from_balance = balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");
        set_balance(&env, &from, from_balance - amount);
        set_balance(
            &env,
            &to,
            balance(&env, &to)
                .checked_add(amount)
                .expect("balance overflow"),
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        require_amount(amount);
        bridge_admin::require_relayer(&env);
        let current = balance(&env, &from);
        assert!(current >= amount, "insufficient balance");
        set_balance(&env, &from, current - amount);
        let supply = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply - amount));
        env.events().publish((symbol_short!("burn"), from), amount);
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        require_amount(amount);
        bridge_admin::require_relayer(&env);
        let data = allowance(&env, &from, &spender);
        assert!(
            data.live_until_ledger == 0 || data.live_until_ledger >= env.ledger().sequence(),
            "allowance expired"
        );
        assert!(data.amount >= amount, "insufficient allowance");
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender),
            &AllowanceData {
                amount: data.amount - amount,
                ..data
            },
        );
        let current = balance(&env, &from);
        assert!(current >= amount, "insufficient balance");
        set_balance(&env, &from, current - amount);
        let supply = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply - amount));
        env.events().publish((symbol_short!("burn"), from), amount);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::Decimals)
            .expect("not initialized")
    }

    pub fn name(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKey::Name)
            .expect("not initialized")
    }

    pub fn symbol(env: Env) -> String {
        env.storage()
            .instance()
            .get(&DataKey::Symbol)
            .expect("not initialized")
    }
}

#[cfg(test)]
mod test;
