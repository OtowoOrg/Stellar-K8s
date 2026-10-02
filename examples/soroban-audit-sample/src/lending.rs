use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};

const BALANCE_TTL_THRESHOLD: u32 = 518_400;
const BALANCE_TTL_EXTEND_TO: u32 = 1_036_800;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Vault,
    Token,
    Collateral(Address),
    Debt(Address),
}

#[contract]
pub struct Lending;

#[contractimpl]
impl Lending {
    pub fn __constructor(env: Env, vault: Address, token: Address) {
        env.storage().instance().set(&DataKey::Vault, &vault);
        env.storage().instance().set(&DataKey::Token, &token);
    }

    /// Called by the vault whenever a user's deposit changes.
    pub fn sync_collateral(env: Env, user: Address, amount: i128) {
        let vault: Address = env.storage().instance().get(&DataKey::Vault).unwrap();
        vault.require_auth();
        Self::write(&env, DataKey::Collateral(user), amount);
    }

    /// Borrow up to 50% of the collateral mirrored from the vault.
    pub fn borrow(env: Env, user: Address, amount: i128) {
        user.require_auth();
        assert!(amount > 0, "amount must be positive");

        let debt = Self::debt(env.clone(), user.clone())
            .checked_add(amount)
            .expect("overflow");
        let max_debt = Self::collateral(env.clone(), user.clone()) / 2;
        assert!(debt <= max_debt, "insufficient collateral");
        Self::write(&env, DataKey::Debt(user.clone()), debt);

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &user, &amount);
    }

    pub fn collateral(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Collateral(user))
            .unwrap_or(0)
    }

    pub fn debt(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Debt(user))
            .unwrap_or(0)
    }

    fn write(env: &Env, key: DataKey, amount: i128) {
        env.storage().persistent().set(&key, &amount);
        env.storage()
            .persistent()
            .extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_TTL_EXTEND_TO);
    }
}
