extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, BytesN as _},
    Address, BytesN, Env, MuxedAddress, String, Vec,
};
use std::vec::Vec as StdVec;

use crate::{BasketConfig, BasketFactory, BasketFactoryClient, ComponentSpec};

/// The compiled basket contract (`make build` produces it).
mod basket {
    soroban_sdk::contractimport!(file = "../target/wasm32v1-none/release/index_basket.wasm");
}

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

#[contracttype]
enum MockKey {
    Decimals,
    Balance(Address),
    Price(Address),
}

#[contract]
struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn __constructor(env: Env, decimals: u32) {
        env.storage().instance().set(&MockKey::Decimals, &decimals);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&MockKey::Decimals).unwrap()
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&MockKey::Balance(to), &(balance + amount));
    }

    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        let to = to.address();
        let from_balance = Self::balance(env.clone(), from.clone());
        assert!(
            amount >= 0 && from_balance >= amount,
            "insufficient balance"
        );
        env.storage()
            .persistent()
            .set(&MockKey::Balance(from), &(from_balance - amount));
        let to_balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&MockKey::Balance(to), &(to_balance + amount));
    }
}

#[contract]
struct MockOracle;

#[contractimpl]
impl MockOracle {
    pub fn set_price(env: Env, asset: Address, price: i128) {
        let data = basket::PriceData {
            price,
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&MockKey::Price(asset), &data);
    }

    pub fn lastprice(env: Env, asset: basket::OracleAsset) -> Option<basket::PriceData> {
        match asset {
            basket::OracleAsset::Stellar(address) => {
                env.storage().persistent().get(&MockKey::Price(address))
            }
            basket::OracleAsset::Other(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const USD: i128 = 100_000_000_000_000;

struct Setup<'a> {
    env: Env,
    admin: Address,
    factory: BasketFactoryClient<'a>,
    oracle: MockOracleClient<'a>,
}

impl<'a> Setup<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let wasm_hash = env.deployer().upload_contract_wasm(basket::WASM);
        let factory_id = env.register(BasketFactory, (admin.clone(), wasm_hash));
        let oracle_id = env.register(MockOracle, ());
        Setup {
            factory: BasketFactoryClient::new(&env, &factory_id),
            oracle: MockOracleClient::new(&env, &oracle_id),
            env,
            admin,
        }
    }

    /// Registers tokens with the given `(decimals, price, units)` and
    /// returns a config with equal weights.
    fn config(&self, assets: &[(u32, i128, i128)]) -> (BasketConfig, StdVec<Address>) {
        let weight = 10_000 / assets.len() as u32;
        let mut tokens = StdVec::new();
        let mut components = Vec::new(&self.env);
        for (decimals, price, units) in assets {
            let token = self.env.register(MockToken, (*decimals,));
            self.oracle.set_price(&token, price);
            components.push_back(ComponentSpec {
                token: token.clone(),
                units: *units,
                weight_bps: weight,
            });
            tokens.push(token);
        }
        let config = BasketConfig {
            name: String::from_str(&self.env, "Five Asset Index"),
            symbol: String::from_str(&self.env, "FIVE"),
            decimals: 7,
            components,
            oracle: self.oracle.address.clone(),
            max_price_age: 600,
            rebalance_incentive_bps: 10,
        };
        (config, tokens)
    }
}

/// Five tokens with precisions 7/6/8/18/9, each worth $20 per basket token.
fn five_assets() -> [(u32, i128, i128); 5] {
    [
        (7, USD / 10, 2_000_000_000),
        (6, USD, 20_000_000),
        (8, 60_000 * USD, 33_333),
        (18, 3_000 * USD, 6_666_666_666_666_667),
        (9, 150 * USD, 133_333_333),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// End-to-end validation through the deployed WASM: deploy a 5-asset basket
/// via the factory, issue, drift prices, rebalance back to 20% and redeem.
#[test]
fn factory_deploys_five_asset_basket_that_rebalances_to_target() {
    let s = Setup::new();
    let creator = Address::generate(&s.env);
    let (config, tokens) = s.config(&five_assets());
    let salt = BytesN::random(&s.env);

    let predicted = s.factory.basket_address(&creator, &salt);
    let addr = s.factory.create_basket(&creator, &salt, &config);
    assert_eq!(addr, predicted);
    assert!(s.factory.is_basket(&addr));
    assert_eq!(s.factory.basket_count(), 1);
    assert_eq!(
        s.factory.baskets(&0, &10),
        Vec::from_array(&s.env, [addr.clone()])
    );

    let basket = basket::Client::new(&s.env, &addr);
    assert_eq!(basket.admin(), creator);
    assert_eq!(basket.symbol(), String::from_str(&s.env, "FIVE"));
    let decimals: StdVec<u32> = basket.components().iter().map(|c| c.decimals).collect();
    assert_eq!(decimals, [7, 6, 8, 18, 9]);

    // Issue 500 basket tokens against the exact underlying ratios.
    let user = Address::generate(&s.env);
    let shares = 500 * 10_000_000;
    let quote = basket.quote_issue(&shares);
    for (token, amount) in tokens.iter().zip(quote.iter()) {
        MockTokenClient::new(&s.env, token).mint(&user, &amount);
    }
    basket.issue(&user, &shares, &quote);
    assert_eq!(basket.balance(&user), shares);

    // Price drift pushes weights away from 20%.
    s.oracle.set_price(&tokens[0], &(USD * 3 / 10));
    s.oracle.set_price(&tokens[2], &(40_000 * USD));
    s.oracle.set_price(&tokens[3], &(3_600 * USD));
    let max_dev = |b: &basket::Client| {
        b.allocations()
            .iter()
            .map(|a| a.weight_ppm.abs_diff(a.target_ppm))
            .max()
            .unwrap()
    };
    assert!(max_dev(&basket) > 50_000);

    let arb = Address::generate(&s.env);
    basket.set_arbitrageur(&arb, &true);
    for _ in 0..32 {
        let allocs = basket.allocations();
        let dev = |i: u32| {
            let a = allocs.get_unchecked(i);
            a.weight_ppm as i64 - a.target_ppm as i64
        };
        let under = allocs.get_unchecked((0..allocs.len()).min_by_key(|&i| dev(i)).unwrap());
        let over = allocs.get_unchecked((0..allocs.len()).max_by_key(|&i| dev(i)).unwrap());
        let q = basket.quote_rebalance(&under.token, &over.token);
        if q.max_amount_in == 0 || q.amount_out == 0 {
            break;
        }
        MockTokenClient::new(&s.env, &under.token).mint(&arb, &q.max_amount_in);
        basket.rebalance(
            &arb,
            &under.token,
            &q.max_amount_in,
            &over.token,
            &q.amount_out,
        );
    }
    assert!(
        max_dev(&basket) <= 1,
        "residual drift {} ppm",
        max_dev(&basket)
    );
    for a in basket.allocations().iter() {
        assert_eq!(a.target_ppm, 200_000);
    }

    // Fully collateralised: redeeming everything empties the basket exactly.
    let reserves = basket.reserves();
    let mut zeros = Vec::new(&s.env);
    for _ in 0..5 {
        zeros.push_back(0i128);
    }
    assert_eq!(basket.redeem(&user, &shares, &zeros), reserves);
    for (token, reserve) in tokens.iter().zip(reserves.iter()) {
        let t = MockTokenClient::new(&s.env, token);
        assert_eq!(t.balance(&addr), 0);
        assert_eq!(t.balance(&user), reserve);
    }
}

#[test]
fn invalid_config_aborts_deployment_without_registering() {
    let s = Setup::new();
    let creator = Address::generate(&s.env);
    let (mut config, _) = s.config(&five_assets());
    let mut c = config.components.get_unchecked(0);
    c.weight_bps += 1;
    config.components.set(0, c);

    let salt = BytesN::random(&s.env);
    assert!(s
        .factory
        .try_create_basket(&creator, &salt, &config)
        .is_err());
    assert_eq!(s.factory.basket_count(), 0);
    assert!(!s
        .factory
        .is_basket(&s.factory.basket_address(&creator, &salt)));
}

#[test]
fn salts_are_namespaced_per_creator() {
    let s = Setup::new();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    let (config, _) = s.config(&five_assets());
    let salt = BytesN::from_array(&s.env, &[7; 32]);

    let a = s.factory.create_basket(&alice, &salt, &config);
    let b = s.factory.create_basket(&bob, &salt, &config);
    assert_ne!(a, b);
    assert_eq!(basket::Client::new(&s.env, &b).admin(), bob);
    // Reusing a salt for the same creator collides with the existing basket.
    assert!(s.factory.try_create_basket(&alice, &salt, &config).is_err());
    assert_eq!(s.factory.basket_count(), 2);
    assert_eq!(s.factory.baskets(&1, &5), Vec::from_array(&s.env, [b]));
    assert_eq!(s.factory.baskets(&5, &5).len(), 0);
}

#[test]
fn create_basket_requires_creator_auth() {
    let s = Setup::new();
    let creator = Address::generate(&s.env);
    let (config, _) = s.config(&five_assets());
    s.factory
        .create_basket(&creator, &BytesN::random(&s.env), &config);
    assert_eq!(s.env.auths()[0].0, creator);
}

#[test]
fn admin_can_update_wasm_hash_and_transfer_admin() {
    let s = Setup::new();
    let new_hash = BytesN::random(&s.env);
    s.factory.set_basket_wasm_hash(&new_hash);
    assert_eq!(s.env.auths()[0].0, s.admin);
    assert_eq!(s.factory.basket_wasm_hash(), new_hash);

    let next = Address::generate(&s.env);
    s.factory.set_admin(&next);
    assert_eq!(s.factory.admin(), next);
}

#[test]
fn admin_functions_reject_unauthorised_callers() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let hash = env.deployer().upload_contract_wasm(basket::WASM);
    let factory = BasketFactoryClient::new(&env, &env.register(BasketFactory, (admin, hash)));
    assert!(factory
        .try_set_basket_wasm_hash(&BytesN::random(&env))
        .is_err());
    assert!(factory.try_set_admin(&Address::generate(&env)).is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn baskets_rejects_oversized_page() {
    let s = Setup::new();
    s.factory.baskets(&0, &51);
}
