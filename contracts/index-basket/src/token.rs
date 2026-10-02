//! SEP-41 implementation of the basket share token.
//!
//! `burn`/`burn_from` follow SEP-41 semantics: shares are destroyed without
//! releasing collateral, which accrues pro rata to the remaining holders. Use
//! [`crate::IndexBasket::redeem`] to unlock the underlying assets.

use soroban_sdk::{
    contractimpl, panic_with_error, token::TokenInterface, Address, Env, MuxedAddress, String,
};

use crate::types::{
    AllowanceKey, AllowanceValue, Approve, BasketError, Burn, DataKey, Mint, Transfer,
    BALANCE_BUMP, BALANCE_THRESHOLD,
};
use crate::{bump_instance, IndexBasket, IndexBasketArgs, IndexBasketClient};

pub(crate) fn total_supply(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalSupply)
        .unwrap_or(0)
}

fn set_total_supply(env: &Env, supply: i128) {
    env.storage().instance().set(&DataKey::TotalSupply, &supply);
}

pub(crate) fn balance_of(env: &Env, id: &Address) -> i128 {
    let key = DataKey::Balance(id.clone());
    match env.storage().persistent().get::<_, i128>(&key) {
        Some(balance) => {
            env.storage()
                .persistent()
                .extend_ttl(&key, BALANCE_THRESHOLD, BALANCE_BUMP);
            balance
        }
        None => 0,
    }
}

fn write_balance(env: &Env, id: &Address, amount: i128) {
    let key = DataKey::Balance(id.clone());
    env.storage().persistent().set(&key, &amount);
    env.storage()
        .persistent()
        .extend_ttl(&key, BALANCE_THRESHOLD, BALANCE_BUMP);
}

fn spend_balance(env: &Env, id: &Address, amount: i128) {
    let balance = balance_of(env, id);
    if balance < amount {
        panic_with_error!(env, BasketError::InsufficientBalance);
    }
    write_balance(env, id, balance - amount);
}

fn receive_balance(env: &Env, id: &Address, amount: i128) {
    let balance = balance_of(env, id);
    let updated = balance
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, BasketError::Overflow));
    write_balance(env, id, updated);
}

fn read_allowance(env: &Env, from: &Address, spender: &Address) -> i128 {
    let key = DataKey::Allowance(AllowanceKey {
        from: from.clone(),
        spender: spender.clone(),
    });
    match env.storage().temporary().get::<_, AllowanceValue>(&key) {
        Some(a) if a.expiration_ledger >= env.ledger().sequence() => a.amount,
        _ => 0,
    }
}

fn write_allowance(
    env: &Env,
    from: &Address,
    spender: &Address,
    amount: i128,
    expiration_ledger: u32,
) {
    if amount > 0 && expiration_ledger < env.ledger().sequence() {
        panic_with_error!(env, BasketError::InvalidExpiration);
    }
    let key = DataKey::Allowance(AllowanceKey {
        from: from.clone(),
        spender: spender.clone(),
    });
    env.storage().temporary().set(
        &key,
        &AllowanceValue {
            amount,
            expiration_ledger,
        },
    );
    if amount > 0 {
        let live_for = expiration_ledger - env.ledger().sequence();
        env.storage()
            .temporary()
            .extend_ttl(&key, live_for, live_for);
    }
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let key = DataKey::Allowance(AllowanceKey {
        from: from.clone(),
        spender: spender.clone(),
    });
    let Some(current) = env.storage().temporary().get::<_, AllowanceValue>(&key) else {
        panic_with_error!(env, BasketError::InsufficientAllowance);
    };
    if current.expiration_ledger < env.ledger().sequence() || current.amount < amount {
        panic_with_error!(env, BasketError::InsufficientAllowance);
    }
    if amount > 0 {
        write_allowance(
            env,
            from,
            spender,
            current.amount - amount,
            current.expiration_ledger,
        );
    }
}

fn check_nonnegative(env: &Env, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, BasketError::InvalidAmount);
    }
}

/// Mints `amount` shares to `to` and increases the total supply.
pub(crate) fn mint_shares(env: &Env, to: &Address, amount: i128) {
    let supply = total_supply(env)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, BasketError::Overflow));
    receive_balance(env, to, amount);
    set_total_supply(env, supply);
    Mint {
        to: to.clone(),
        amount,
    }
    .publish(env);
}

/// Burns `amount` shares held by `from` and decreases the total supply.
pub(crate) fn burn_shares(env: &Env, from: &Address, amount: i128) {
    spend_balance(env, from, amount);
    set_total_supply(env, total_supply(env) - amount);
    Burn {
        from: from.clone(),
        amount,
    }
    .publish(env);
}

fn move_shares(env: &Env, from: &Address, to: &Address, amount: i128) {
    spend_balance(env, from, amount);
    receive_balance(env, to, amount);
    Transfer {
        from: from.clone(),
        to: to.clone(),
        amount,
    }
    .publish(env);
}

#[contractimpl]
impl TokenInterface for IndexBasket {
    fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        bump_instance(&env);
        read_allowance(&env, &from, &spender)
    }

    fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        from.require_auth();
        check_nonnegative(&env, amount);
        bump_instance(&env);
        write_allowance(&env, &from, &spender, amount, expiration_ledger);
        Approve {
            from,
            spender,
            amount,
            expiration_ledger,
        }
        .publish(&env);
    }

    fn balance(env: Env, id: Address) -> i128 {
        bump_instance(&env);
        balance_of(&env, &id)
    }

    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        check_nonnegative(&env, amount);
        bump_instance(&env);
        move_shares(&env, &from, &to.address(), amount);
    }

    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        check_nonnegative(&env, amount);
        bump_instance(&env);
        spend_allowance(&env, &from, &spender, amount);
        move_shares(&env, &from, &to, amount);
    }

    fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        check_nonnegative(&env, amount);
        bump_instance(&env);
        burn_shares(&env, &from, amount);
    }

    fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        check_nonnegative(&env, amount);
        bump_instance(&env);
        spend_allowance(&env, &from, &spender, amount);
        burn_shares(&env, &from, amount);
    }

    fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Decimals).unwrap()
    }

    fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }
}
