use soroban_sdk::{contract, contractclient, contractimpl, contracttype, token, Address, Env};

use crate::lending::LendingClient;

/// ~30 days at 5s ledgers.
const BALANCE_TTL_THRESHOLD: u32 = 518_400;
const BALANCE_TTL_EXTEND_TO: u32 = 1_036_800;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Token,
    Lending,
    Balance(Address),
}

/// Implemented by contracts that want to be notified when they receive a withdrawal.
#[contractclient(name = "WithdrawReceiverClient")]
pub trait WithdrawReceiver {
    fn on_withdraw(env: Env, from: Address, amount: i128);
}

#[contract]
pub struct Vault;

#[contractimpl]
impl Vault {
    pub fn __constructor(env: Env, token: Address, lending: Address) {
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Lending, &lending);
    }

    pub fn deposit(env: Env, from: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "amount must be positive");

        let new_balance = Self::balance(env.clone(), from.clone())
            .checked_add(amount)
            .expect("overflow");
        Self::write_balance(&env, &from, new_balance);
        Self::lending(&env).sync_collateral(&from, &new_balance);

        token::Client::new(&env, &Self::token(&env)).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );
    }

    /// Withdraw `amount` of `from`'s deposit to `to`.
    pub fn withdraw(env: Env, from: Address, to: Address, amount: i128) {
        // INJECTED BUG (authorization bypass, checklist AUTH-02):
        // authorizes the *recipient* instead of the owner of the funds, so anyone
        // can withdraw any depositor's balance to themselves.
        to.require_auth();
        assert!(amount > 0, "amount must be positive");

        let balance = Self::balance(env.clone(), from.clone());
        assert!(balance >= amount, "insufficient balance");
        let new_balance = balance - amount;
        Self::write_balance(&env, &from, new_balance);
        Self::lending(&env).sync_collateral(&from, &new_balance);

        token::Client::new(&env, &Self::token(&env)).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );
    }

    /// Withdraw `amount` to a `receiver` contract and notify it via `on_withdraw`.
    pub fn withdraw_and_notify(env: Env, from: Address, receiver: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "amount must be positive");

        let balance = Self::balance(env.clone(), from.clone());
        assert!(balance >= amount, "insufficient balance");

        // INJECTED BUG (cross-contract reentrancy, checklist XCC-02):
        // interactions happen before effects. While `receiver` runs, the lending
        // contract still holds the pre-withdrawal collateral for `from`.
        token::Client::new(&env, &Self::token(&env)).transfer(
            &env.current_contract_address(),
            &receiver,
            &amount,
        );
        WithdrawReceiverClient::new(&env, &receiver).on_withdraw(&from, &amount);

        let new_balance = balance - amount;
        Self::write_balance(&env, &from, new_balance);
        Self::lending(&env).sync_collateral(&from, &new_balance);
    }

    pub fn balance(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(user))
            .unwrap_or(0)
    }

    fn write_balance(env: &Env, user: &Address, amount: i128) {
        let key = DataKey::Balance(user.clone());
        env.storage().persistent().set(&key, &amount);
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_TTL_EXTEND_TO);
    }

    fn token(env: &Env) -> Address {
        env.storage().instance().get(&DataKey::Token).unwrap()
    }

    fn lending(env: &Env) -> LendingClient<'_> {
        let address: Address = env.storage().instance().get(&DataKey::Lending).unwrap();
        LendingClient::new(env, &address)
    }
}
