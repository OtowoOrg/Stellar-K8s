extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, IssuerFlags, Ledger},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, MuxedAddress, String, Vec,
};
use std::vec::Vec as StdVec;

use crate::{
    BasketConfig, ComponentSpec, IndexBasket, IndexBasketClient, OracleAsset, PriceData,
    RebalanceParams, PPM,
};

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

#[contracttype]
enum MockKey {
    Decimals,
    Balance(Address),
    Price(Address),
}

/// Minimal SEP-41 token with configurable precision.
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

/// SEP-40 compatible price feed whose prices tests can move at will.
#[contract]
struct MockOracle;

#[contractimpl]
impl MockOracle {
    pub fn set_price(env: Env, asset: Address, price: i128) {
        let data = PriceData {
            price,
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&MockKey::Price(asset), &data);
    }

    pub fn lastprice(env: Env, asset: OracleAsset) -> Option<PriceData> {
        match asset {
            OracleAsset::Stellar(address) => {
                env.storage().persistent().get(&MockKey::Price(address))
            }
            OracleAsset::Other(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Oracle prices use 14 decimals (as Reflector does).
const USD: i128 = 100_000_000_000_000;
const MAX_PRICE_AGE: u64 = 300;

struct Asset {
    decimals: u32,
    /// Price of one whole token, in oracle units.
    price: i128,
    weight_bps: u32,
    /// Base units per whole basket token.
    units: i128,
}

struct Setup<'a> {
    env: Env,
    admin: Address,
    arb: Address,
    oracle: MockOracleClient<'a>,
    tokens: StdVec<Address>,
    basket: IndexBasketClient<'a>,
}

impl<'a> Setup<'a> {
    fn new(assets: &[Asset], basket_decimals: u32, incentive_bps: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);

        let admin = Address::generate(&env);
        let arb = Address::generate(&env);
        let oracle_id = env.register(MockOracle, ());
        let oracle = MockOracleClient::new(&env, &oracle_id);

        let mut tokens = StdVec::new();
        let mut specs = Vec::new(&env);
        for a in assets {
            let token = env.register(MockToken, (a.decimals,));
            oracle.set_price(&token, &a.price);
            specs.push_back(ComponentSpec {
                token: token.clone(),
                units: a.units,
                weight_bps: a.weight_bps,
            });
            tokens.push(token);
        }

        let config = config(&env, specs, &oracle_id, basket_decimals, incentive_bps);
        let basket_id = env.register(IndexBasket, (admin.clone(), config));
        let basket = IndexBasketClient::new(&env, &basket_id);
        basket.set_arbitrageur(&arb, &true);

        Setup {
            env,
            admin,
            arb,
            oracle,
            tokens,
            basket,
        }
    }

    fn token(&self, i: usize) -> MockTokenClient<'a> {
        MockTokenClient::new(&self.env, &self.tokens[i])
    }

    fn fund(&self, who: &Address, amounts: &Vec<i128>) {
        for (i, amount) in amounts.iter().enumerate() {
            self.token(i).mint(who, &amount);
        }
    }

    /// Funds `who` with exactly the quoted deposit and issues `shares`.
    fn issue(&self, who: &Address, shares: i128) -> Vec<i128> {
        let quote = self.basket.quote_issue(&shares);
        self.fund(who, &quote);
        self.basket.issue(who, &shares, &quote)
    }

    fn balances_of(&self, who: &Address) -> StdVec<i128> {
        (0..self.tokens.len())
            .map(|i| self.token(i).balance(who))
            .collect()
    }

    /// Token balances held by the basket must always cover its reserves.
    fn assert_collateralised(&self) {
        let held = self.balances_of(&self.basket.address);
        for (i, reserve) in self.basket.reserves().iter().enumerate() {
            assert!(held[i] >= reserve, "component {i} under-collateralised");
        }
    }

    fn set_price(&self, i: usize, price: i128) {
        self.oracle.set_price(&self.tokens[i], &price);
    }

    /// Greedy arbitrageur: repeatedly trades the most underweight component
    /// in for the most overweight one, using the contract's own quote.
    fn arbitrage_until_balanced(&self, max_steps: u32) -> u32 {
        for step in 0..max_steps {
            let allocs = self.basket.allocations();
            let dev = |i: u32| {
                let a = allocs.get_unchecked(i);
                a.weight_ppm as i64 - a.target_ppm as i64
            };
            let under = (0..allocs.len()).min_by_key(|&i| dev(i)).unwrap();
            let over = (0..allocs.len()).max_by_key(|&i| dev(i)).unwrap();
            let asset_in = allocs.get_unchecked(under).token;
            let asset_out = allocs.get_unchecked(over).token;
            let quote = self.basket.quote_rebalance(&asset_in, &asset_out);
            if quote.max_amount_in == 0 || quote.amount_out == 0 {
                return step;
            }
            MockTokenClient::new(&self.env, &asset_in).mint(&self.arb, &quote.max_amount_in);
            let out = self.basket.rebalance(
                &self.arb,
                &asset_in,
                &quote.max_amount_in,
                &asset_out,
                &quote.amount_out,
            );
            assert_eq!(out, quote.amount_out);
            self.assert_collateralised();
        }
        max_steps
    }

    fn max_deviation_ppm(&self) -> u32 {
        self.basket
            .allocations()
            .iter()
            .map(|a| a.weight_ppm.abs_diff(a.target_ppm))
            .max()
            .unwrap()
    }
}

fn config(
    env: &Env,
    components: Vec<ComponentSpec>,
    oracle: &Address,
    decimals: u32,
    incentive_bps: u32,
) -> BasketConfig {
    BasketConfig {
        name: String::from_str(env, "Stellar Index"),
        symbol: String::from_str(env, "SIDX"),
        decimals,
        components,
        oracle: oracle.clone(),
        max_price_age: MAX_PRICE_AGE,
        rebalance_incentive_bps: incentive_bps,
    }
}

/// Five assets with distinct precisions (7, 6, 8, 18, 9), each targeting 20%
/// and each worth $20 per whole basket token at the initial prices.
fn five_assets() -> [Asset; 5] {
    [
        // XLM-like: $0.10, 7 decimals -> 200 XLM
        Asset {
            decimals: 7,
            price: USD / 10,
            weight_bps: 2_000,
            units: 2_000_000_000,
        },
        // USDC-like: $1, 6 decimals -> 20 USDC
        Asset {
            decimals: 6,
            price: USD,
            weight_bps: 2_000,
            units: 20_000_000,
        },
        // sBTC-like: $60k, 8 decimals -> 0.00033333 BTC
        Asset {
            decimals: 8,
            price: 60_000 * USD,
            weight_bps: 2_000,
            units: 33_333,
        },
        // ETH-like: $3k, 18 decimals -> 0.00666.. ETH
        Asset {
            decimals: 18,
            price: 3_000 * USD,
            weight_bps: 2_000,
            units: 6_666_666_666_666_667,
        },
        // SOL-like: $150, 9 decimals -> 0.1333.. SOL
        Asset {
            decimals: 9,
            price: 150 * USD,
            weight_bps: 2_000,
            units: 133_333_333,
        },
    ]
}

/// The canonical 50% XLM / 30% USDC / 20% sBTC basket.
fn three_assets() -> [Asset; 3] {
    [
        Asset {
            decimals: 7,
            price: USD / 10,
            weight_bps: 5_000,
            units: 5_000_000_000,
        },
        Asset {
            decimals: 6,
            price: USD,
            weight_bps: 3_000,
            units: 30_000_000,
        },
        Asset {
            decimals: 8,
            price: 60_000 * USD,
            weight_bps: 2_000,
            units: 33_334,
        },
    ]
}

fn zeros(env: &Env, n: u32) -> Vec<i128> {
    let mut v = Vec::new(env);
    for _ in 0..n {
        v.push_back(0);
    }
    v
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[test]
fn constructor_stores_metadata_and_token_precisions() {
    let s = Setup::new(&five_assets(), 7, 25);
    let b = &s.basket;
    assert_eq!(b.admin(), s.admin);
    assert_eq!(b.decimals(), 7);
    assert_eq!(b.name(), String::from_str(&s.env, "Stellar Index"));
    assert_eq!(b.symbol(), String::from_str(&s.env, "SIDX"));
    assert_eq!(b.total_supply(), 0);
    let decimals: StdVec<u32> = b.components().iter().map(|c| c.decimals).collect();
    assert_eq!(decimals, [7, 6, 8, 18, 9]);
    assert_eq!(b.reserves(), zeros(&s.env, 5));
    assert_eq!(b.rebalance_params().incentive_bps, 25);
    assert!(b.is_arbitrageur(&s.arb));
}

fn try_construct(env: &Env, specs: Vec<ComponentSpec>, decimals: u32, incentive_bps: u32) {
    let oracle = env.register(MockOracle, ());
    let cfg = config(env, specs, &oracle, decimals, incentive_bps);
    env.register(IndexBasket, (Address::generate(env), cfg));
}

fn specs(env: &Env, decimals: &[u32], weights: &[u32]) -> Vec<ComponentSpec> {
    let mut out = Vec::new(env);
    for (d, w) in decimals.iter().zip(weights) {
        out.push_back(ComponentSpec {
            token: env.register(MockToken, (*d,)),
            units: 1_000,
            weight_bps: *w,
        });
    }
    out
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn constructor_rejects_weights_not_summing_to_100_percent() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7, 6], &[5_000, 4_999]), 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn constructor_rejects_zero_weight() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7, 6, 8], &[5_000, 5_000, 0]), 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_duplicate_component() {
    let env = Env::default();
    let mut s = specs(&env, &[7, 6], &[5_000, 3_000]);
    let mut dup = s.get_unchecked(0);
    dup.weight_bps = 2_000;
    s.push_back(dup);
    try_construct(&env, s, 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_single_component() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7], &[10_000]), 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn constructor_rejects_too_many_components() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7; 11], &[1_000; 11]), 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn constructor_rejects_zero_units() {
    let env = Env::default();
    let mut s = specs(&env, &[7, 6], &[5_000, 5_000]);
    let mut c = s.get_unchecked(1);
    c.units = 0;
    s.set(1, c);
    try_construct(&env, s, 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_component_precision_above_18() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7, 19], &[5_000, 5_000]), 7, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_basket_precision_above_18() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7, 6], &[5_000, 5_000]), 19, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_excessive_incentive() {
    let env = Env::default();
    try_construct(&env, specs(&env, &[7, 6], &[5_000, 5_000]), 7, 101);
}

// ---------------------------------------------------------------------------
// Issuance
// ---------------------------------------------------------------------------

#[test]
fn bootstrap_issue_uses_units_across_precisions() {
    for basket_decimals in [0u32, 7, 18] {
        let s = Setup::new(&five_assets(), basket_decimals, 0);
        let user = Address::generate(&s.env);
        let one = 10i128.pow(basket_decimals);
        let deposited = s.issue(&user, 3 * one);

        let expected: StdVec<i128> = five_assets().iter().map(|a| 3 * a.units).collect();
        assert_eq!(deposited.iter().collect::<StdVec<_>>(), expected);
        assert_eq!(s.basket.balance(&user), 3 * one);
        assert_eq!(s.basket.total_supply(), 3 * one);
        assert_eq!(s.balances_of(&s.basket.address), expected);
        assert_eq!(s.basket.reserves().iter().collect::<StdVec<_>>(), expected);
    }
}

#[test]
fn bootstrap_issue_rounds_every_component_up() {
    // One base unit of an 18-decimal basket is far smaller than one base unit
    // of any component, so every deposit must round up to at least 1.
    let s = Setup::new(&five_assets(), 18, 0);
    let user = Address::generate(&s.env);
    let deposited = s.issue(&user, 1);
    assert_eq!(deposited, vec![&s.env, 1, 1, 1, 1, 1]);
    s.assert_collateralised();
}

#[test]
fn subsequent_issue_is_proportional_to_reserves() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);

    // Half of Alice's position requires exactly half of every reserve.
    let reserves = s.basket.reserves();
    let quote = s.basket.quote_issue(&5_000_000);
    for (r, q) in reserves.iter().zip(quote.iter()) {
        assert_eq!(q, (r + 1) / 2);
    }
    s.issue(&bob, 5_000_000);
    assert_eq!(s.basket.total_supply(), 15_000_000);
    s.assert_collateralised();
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn issue_enforces_max_amounts() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    let mut max = s.basket.quote_issue(&10_000_000);
    s.fund(&user, &max);
    max.set(2, max.get_unchecked(2) - 1);
    s.basket.issue(&user, &10_000_000, &max);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn issue_rejects_length_mismatch() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.basket
        .issue(&user, &10_000_000, &vec![&s.env, i128::MAX, i128::MAX]);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn issue_rejects_zero_shares() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.basket.issue(&user, &0, &zeros(&s.env, 3));
}

#[test]
fn issue_requires_user_auth() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.issue(&user, 10_000_000);
    let auths = s.env.auths();
    assert!(auths.iter().any(|(addr, _)| *addr == user));
}

#[test]
fn donations_do_not_move_share_price() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);
    let before = s.basket.quote_issue(&10_000_000);

    // Direct transfers to the basket are ignored by internal accounting.
    s.token(0).mint(&s.basket.address, &1_000_000_000_000);
    s.token(2).mint(&s.basket.address, &1_000_000_000);
    assert_eq!(s.basket.quote_issue(&10_000_000), before);
    assert_eq!(s.basket.quote_redeem(&10_000_000), s.basket.reserves());
}

// ---------------------------------------------------------------------------
// Redemption
// ---------------------------------------------------------------------------

#[test]
fn redeem_releases_pro_rata_underlying_rounded_down() {
    let s = Setup::new(&five_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 30_000_000);
    s.issue(&bob, 7);

    let reserves = s.basket.reserves();
    let supply = s.basket.total_supply();
    let shares = 10_000_001;
    let out = s.basket.redeem(&alice, &shares, &zeros(&s.env, 5));
    for (i, (r, o)) in reserves.iter().zip(out.iter()).enumerate() {
        assert_eq!(o, r * shares / supply, "component {i}");
    }
    assert_eq!(s.balances_of(&alice), out.iter().collect::<StdVec<_>>());
    assert_eq!(s.basket.balance(&alice), 30_000_000 - shares);
    assert_eq!(s.basket.total_supply(), supply - shares);
    s.assert_collateralised();
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn redeem_enforces_min_amounts() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.issue(&user, 10_000_000);
    let mut min = s.basket.quote_redeem(&4_000_000);
    min.set(1, min.get_unchecked(1) + 1);
    s.basket.redeem(&user, &4_000_000, &min);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn redeem_rejects_more_than_balance() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);
    s.issue(&bob, 10_000_000);
    s.basket.redeem(&alice, &10_000_001, &zeros(&s.env, 3));
}

#[test]
fn round_trips_never_extract_value_across_precisions() {
    let s = Setup::new(&five_assets(), 9, 0);
    let whale = Address::generate(&s.env);
    s.issue(&whale, 1_234_567_891_011);

    let user = Address::generate(&s.env);
    for shares in [1i128, 3, 7, 999, 1_000_001, 77_777_777, 5_000_000_019] {
        let paid = s.issue(&user, shares);
        let got = s.basket.redeem(&user, &shares, &zeros(&s.env, 5));
        for (i, (p, g)) in paid.iter().zip(got.iter()).enumerate() {
            assert!(g <= p, "shares {shares}, component {i}: got {g} > paid {p}");
        }
        s.assert_collateralised();
    }
    assert_eq!(s.basket.balance(&user), 0);
}

#[test]
fn final_redemption_sweeps_dust_and_carries_composition_forward() {
    let s = Setup::new(&five_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    // Odd-sized issues leave rounding dust in the reserves.
    s.issue(&alice, 13_333_333);
    s.issue(&bob, 1);
    s.basket.redeem(&bob, &1, &zeros(&s.env, 5));

    let reserves = s.basket.reserves();
    let supply = s.basket.total_supply();
    let out = s.basket.redeem(&alice, &supply, &zeros(&s.env, 5));
    assert_eq!(out, reserves);
    assert_eq!(s.basket.reserves(), zeros(&s.env, 5));
    assert_eq!(s.basket.total_supply(), 0);
    assert_eq!(s.balances_of(&s.basket.address), [0; 5]);

    // The next bootstrap issue uses the last per-share composition.
    for (c, r) in s.basket.components().iter().zip(reserves.iter()) {
        assert_eq!(c.units, r * 10_000_000 / supply);
    }
}

#[test]
fn redemption_is_atomic_when_any_transfer_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let issuer = Address::generate(&env);
    let user = Address::generate(&env);
    let oracle = env.register(MockOracle, ());

    let mut sacs = StdVec::new();
    let mut specs = Vec::new(&env);
    for _ in 0..3 {
        let sac = env.register_stellar_asset_contract_v2(issuer.clone());
        sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        specs.push_back(ComponentSpec {
            token: sac.address(),
            units: 10_000_000,
            weight_bps: 0,
        });
        sacs.push(sac.address());
    }
    for (i, w) in [5_000u32, 3_000, 2_000].iter().enumerate() {
        let mut c = specs.get_unchecked(i as u32);
        c.weight_bps = *w;
        specs.set(i as u32, c);
    }
    let basket_id = env.register(
        IndexBasket,
        (issuer.clone(), config(&env, specs, &oracle, 7, 0)),
    );
    let basket = IndexBasketClient::new(&env, &basket_id);

    for sac in &sacs {
        StellarAssetClient::new(&env, sac).mint(&user, &50_000_000);
    }
    basket.issue(
        &user,
        &20_000_000,
        &vec![&env, i128::MAX, i128::MAX, i128::MAX],
    );

    // Freeze the user's trustline on the last asset: its payout must fail
    // and take the other two transfers and the share burn down with it.
    StellarAssetClient::new(&env, &sacs[2]).set_authorized(&user, &false);
    let result = basket.try_redeem(&user, &20_000_000, &zeros(&env, 3));
    assert!(result.is_err());

    assert_eq!(basket.balance(&user), 20_000_000);
    assert_eq!(basket.total_supply(), 20_000_000);
    assert_eq!(
        basket.reserves(),
        vec![&env, 20_000_000, 20_000_000, 20_000_000]
    );
    for sac in &sacs {
        assert_eq!(TokenClient::new(&env, sac).balance(&user), 30_000_000);
        assert_eq!(TokenClient::new(&env, sac).balance(&basket_id), 20_000_000);
    }

    StellarAssetClient::new(&env, &sacs[2]).set_authorized(&user, &true);
    basket.redeem(&user, &20_000_000, &zeros(&env, 3));
    for sac in &sacs {
        assert_eq!(TokenClient::new(&env, sac).balance(&user), 50_000_000);
    }
}

// ---------------------------------------------------------------------------
// SEP-41 share token
// ---------------------------------------------------------------------------

#[test]
fn shares_are_transferable_and_redeemable_by_recipient() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);
    s.basket.transfer(&alice, &bob, &4_000_000);
    assert_eq!(s.basket.balance(&alice), 6_000_000);
    assert_eq!(s.basket.balance(&bob), 4_000_000);

    let out = s.basket.redeem(&bob, &4_000_000, &zeros(&s.env, 3));
    assert_eq!(s.balances_of(&bob), out.iter().collect::<StdVec<_>>());
}

#[test]
fn allowances_expire_and_are_spent() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);

    let expiry = s.env.ledger().sequence() + 100;
    s.basket.approve(&alice, &spender, &3_000_000, &expiry);
    s.basket.transfer_from(&spender, &alice, &bob, &1_000_000);
    assert_eq!(s.basket.allowance(&alice, &spender), 2_000_000);
    s.basket.burn_from(&spender, &alice, &500_000);
    assert_eq!(s.basket.allowance(&alice, &spender), 1_500_000);
    assert_eq!(s.basket.total_supply(), 9_500_000);

    s.env.ledger().with_mut(|l| l.sequence_number = expiry + 1);
    assert_eq!(s.basket.allowance(&alice, &spender), 0);
    assert!(s
        .basket
        .try_transfer_from(&spender, &alice, &bob, &1)
        .is_err());
}

#[test]
fn plain_burn_accrues_collateral_to_remaining_holders() {
    let s = Setup::new(&three_assets(), 7, 0);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.issue(&alice, 10_000_000);
    s.issue(&bob, 10_000_000);
    s.basket.burn(&alice, &10_000_000);

    let out = s.basket.redeem(&bob, &10_000_000, &zeros(&s.env, 3));
    let expected: StdVec<i128> = three_assets().iter().map(|a| 2 * a.units).collect();
    assert_eq!(out.iter().collect::<StdVec<_>>(), expected);
}

// ---------------------------------------------------------------------------
// Rebalancing
// ---------------------------------------------------------------------------

/// Issue validation scenario: deploy a 5-asset basket, simulate price drift
/// and verify arbitrageurs restore the 20% target weights.
#[test]
fn five_asset_basket_is_rebalanced_back_to_20_percent_weights() {
    let s = Setup::new(&five_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.issue(&user, 1_000 * 10_000_000);
    assert!(s.max_deviation_ppm() <= 10, "starts on target");

    // Simulate drift: XLM doubles, sBTC halves, ETH +50%, SOL -20%.
    s.set_price(0, USD / 5);
    s.set_price(2, 30_000 * USD);
    s.set_price(3, 4_500 * USD);
    s.set_price(4, 120 * USD);
    let drift = s.max_deviation_ppm();
    assert!(drift > 100_000, "drift was {drift} ppm");

    // Without an incentive, rebalancing is value-neutral up to rounding,
    // which always favours the basket.
    let nav_before = total_value(&s);
    let steps = s.arbitrage_until_balanced(32);
    assert!(steps < 32, "arbitrage did not converge");
    assert!(
        s.max_deviation_ppm() <= 1,
        "residual drift {} ppm",
        s.max_deviation_ppm()
    );
    for a in s.basket.allocations().iter() {
        assert_eq!(a.target_ppm, 200_000);
    }
    let nav_after = total_value(&s);
    assert!(nav_after.abs_diff(nav_before) <= (nav_before / 1_000_000_000) as u128);

    // The basket stays fully redeemable after rebalancing.
    let supply = s.basket.total_supply();
    let reserves = s.basket.reserves();
    assert_eq!(s.basket.redeem(&user, &supply, &zeros(&s.env, 5)), reserves);
    assert_eq!(s.balances_of(&s.basket.address), [0; 5]);
}

/// Total basket value in oracle units, normalised to 18 decimals.
fn total_value(s: &Setup) -> i128 {
    let components = s.basket.components();
    s.basket
        .reserves()
        .iter()
        .zip(components.iter())
        .map(|(r, c)| {
            let price = s
                .oracle
                .lastprice(&OracleAsset::Stellar(c.token))
                .unwrap()
                .price;
            r * 10i128.pow(18 - c.decimals) / 1_000_000 * price / 100_000_000
        })
        .sum()
}

#[test]
fn incentive_pays_arbitrageur_premium_and_still_converges() {
    let s = Setup::new(&three_assets(), 7, 50);
    let user = Address::generate(&s.env);
    s.issue(&user, 500 * 10_000_000);
    // XLM rallies 80% -> XLM overweight, USDC and sBTC underweight.
    s.set_price(0, USD * 18 / 100);

    let usdc = s.tokens[1].clone();
    let xlm = s.tokens[0].clone();
    let quote = s.basket.quote_rebalance(&usdc, &xlm);
    assert!(quote.max_amount_in > 0);
    // 1 USDC (6 dp) buys 1/0.18 XLM (7 dp) plus the 0.5% premium.
    assert_eq!(
        quote.amount_out,
        quote.max_amount_in * 1_000 * 10_050 / (18 * 10_000)
    );

    s.arbitrage_until_balanced(32);
    assert!(
        s.max_deviation_ppm() <= 1,
        "residual drift {} ppm",
        s.max_deviation_ppm()
    );
    s.assert_collateralised();
}

#[test]
fn rebalance_handles_large_18_decimal_reserves_without_overflow() {
    let assets = [
        Asset {
            decimals: 18,
            price: 3_000 * USD,
            weight_bps: 5_000,
            units: 10i128.pow(18),
        },
        Asset {
            decimals: 0,
            price: 3_000 * USD,
            weight_bps: 5_000,
            units: 1,
        },
    ];
    let s = Setup::new(&assets, 18, 0);
    let user = Address::generate(&s.env);
    // 10^12 whole shares -> 10^30 base units of the 18-decimal asset.
    s.issue(&user, 10i128.pow(30));
    s.set_price(1, 1_500 * USD);
    s.arbitrage_until_balanced(16);
    assert!(s.max_deviation_ppm() <= 1);
}

fn drifted_three_asset_basket<'a>() -> Setup<'a> {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.issue(&user, 100 * 10_000_000);
    // sBTC doubles: sBTC overweight, XLM and USDC underweight.
    s.set_price(2, 120_000 * USD);
    s
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn rebalance_rejects_unlisted_caller() {
    let s = drifted_three_asset_basket();
    let outsider = Address::generate(&s.env);
    s.token(0).mint(&outsider, &1_000_000);
    s.basket
        .rebalance(&outsider, &s.tokens[0], &1_000_000, &s.tokens[2], &0);
}

#[test]
fn revoked_arbitrageur_cannot_rebalance() {
    let s = drifted_three_asset_basket();
    s.basket.set_arbitrageur(&s.arb, &false);
    assert!(!s.basket.is_arbitrageur(&s.arb));
    s.token(0).mint(&s.arb, &1_000_000);
    assert!(s
        .basket
        .try_rebalance(&s.arb, &s.tokens[0], &1_000_000, &s.tokens[2], &0)
        .is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #17)")]
fn rebalance_rejects_depositing_overweight_asset() {
    let s = drifted_three_asset_basket();
    s.token(2).mint(&s.arb, &1_000);
    s.basket
        .rebalance(&s.arb, &s.tokens[2], &1_000, &s.tokens[0], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn rebalance_rejects_withdrawing_underweight_asset() {
    let s = drifted_three_asset_basket();
    s.token(0).mint(&s.arb, &1_000);
    s.basket
        .rebalance(&s.arb, &s.tokens[0], &1_000, &s.tokens[1], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #19)")]
fn rebalance_rejects_overshooting_target() {
    let s = drifted_three_asset_basket();
    let q = s.basket.quote_rebalance(&s.tokens[0], &s.tokens[2]);
    let amount = q.max_amount_in * 2;
    s.token(0).mint(&s.arb, &amount);
    s.basket
        .rebalance(&s.arb, &s.tokens[0], &amount, &s.tokens[2], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn rebalance_enforces_min_amount_out() {
    let s = drifted_three_asset_basket();
    let q = s.basket.quote_rebalance(&s.tokens[0], &s.tokens[2]);
    s.token(0).mint(&s.arb, &q.max_amount_in);
    s.basket.rebalance(
        &s.arb,
        &s.tokens[0],
        &q.max_amount_in,
        &s.tokens[2],
        &(q.amount_out + 1),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn rebalance_rejects_stale_prices() {
    let s = drifted_three_asset_basket();
    s.env
        .ledger()
        .with_mut(|l| l.timestamp += MAX_PRICE_AGE + 1);
    s.token(0).mint(&s.arb, &1_000);
    s.basket
        .rebalance(&s.arb, &s.tokens[0], &1_000, &s.tokens[2], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #15)")]
fn rebalance_rejects_missing_price() {
    let s = drifted_three_asset_basket();
    let other = s.env.register(MockOracle, ());
    s.basket.set_rebalance_params(&RebalanceParams {
        oracle: other,
        max_price_age: MAX_PRICE_AGE,
        incentive_bps: 0,
    });
    s.basket
        .rebalance(&s.arb, &s.tokens[0], &1_000, &s.tokens[2], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn rebalance_rejects_same_asset() {
    let s = drifted_three_asset_basket();
    s.basket
        .rebalance(&s.arb, &s.tokens[0], &1_000, &s.tokens[0], &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn rebalance_rejects_unknown_asset() {
    let s = drifted_three_asset_basket();
    let stranger = s.env.register(MockToken, (7u32,));
    s.basket
        .rebalance(&s.arb, &stranger, &1_000, &s.tokens[2], &0);
}

#[test]
fn quote_is_zero_when_pair_cannot_improve_allocation() {
    let s = drifted_three_asset_basket();
    // XLM and USDC are both underweight: no trade between them helps.
    let q = s.basket.quote_rebalance(&s.tokens[0], &s.tokens[1]);
    assert_eq!((q.max_amount_in, q.amount_out), (0, 0));
}

#[test]
fn quoted_trade_lands_exactly_on_target_boundary() {
    let s = drifted_three_asset_basket();
    let q = s.basket.quote_rebalance(&s.tokens[1], &s.tokens[2]);
    s.token(1).mint(&s.arb, &q.max_amount_in);
    s.basket.rebalance(
        &s.arb,
        &s.tokens[1],
        &q.max_amount_in,
        &s.tokens[2],
        &q.amount_out,
    );
    let allocs = s.basket.allocations();
    let usdc = allocs.get_unchecked(1);
    let sbtc = allocs.get_unchecked(2);
    // One leg is restored to target; neither overshoots.
    assert!(usdc.weight_ppm <= usdc.target_ppm);
    assert!(sbtc.weight_ppm + 1 >= sbtc.target_ppm);
    assert!(usdc.target_ppm - usdc.weight_ppm <= 1 || sbtc.weight_ppm - sbtc.target_ppm <= 1);
    assert_eq!(s.token(1).balance(&s.arb), 0);
    assert_eq!(s.token(2).balance(&s.arb), q.amount_out);
}

// ---------------------------------------------------------------------------
// Administration
// ---------------------------------------------------------------------------

#[test]
fn admin_functions_require_admin_auth() {
    let s = Setup::new(&three_assets(), 7, 0);
    let who = Address::generate(&s.env);
    s.basket.set_arbitrageur(&who, &true);
    assert_eq!(s.env.auths()[0].0, s.admin);

    let params = RebalanceParams {
        oracle: s.oracle.address.clone(),
        max_price_age: 60,
        incentive_bps: 10,
    };
    s.basket.set_rebalance_params(&params);
    assert_eq!(s.env.auths()[0].0, s.admin);
    assert_eq!(s.basket.rebalance_params(), params);

    let new_admin = Address::generate(&s.env);
    s.basket.set_admin(&new_admin);
    assert_eq!(s.basket.admin(), new_admin);
}

#[test]
fn admin_functions_reject_unauthorised_callers() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let oracle = env.register(MockOracle, ());
    let cfg = config(&env, specs(&env, &[7, 6], &[5_000, 5_000]), &oracle, 7, 0);
    let basket = IndexBasketClient::new(&env, &env.register(IndexBasket, (admin, cfg)));
    let who = Address::generate(&env);
    assert!(basket.try_set_arbitrageur(&who, &true).is_err());
    assert!(basket.try_set_admin(&who).is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn set_rebalance_params_validates_incentive_cap() {
    let s = Setup::new(&three_assets(), 7, 0);
    s.basket.set_rebalance_params(&RebalanceParams {
        oracle: s.oracle.address.clone(),
        max_price_age: 60,
        incentive_bps: 1_000,
    });
}

#[test]
fn allocation_weights_reflect_prices_and_precisions() {
    let s = Setup::new(&three_assets(), 7, 0);
    let user = Address::generate(&s.env);
    s.issue(&user, 10_000_000);
    let allocs = s.basket.allocations();
    let targets: StdVec<u32> = allocs.iter().map(|a| a.target_ppm).collect();
    assert_eq!(targets, [500_000, 300_000, 200_000]);
    // sBTC units (33_334) are worth $20.0004, so weights are within 10 ppm.
    for a in allocs.iter() {
        assert!(a.weight_ppm.abs_diff(a.target_ppm) <= 10);
    }
    let sum: u32 = allocs.iter().map(|a| a.weight_ppm).sum();
    assert!(sum <= PPM && PPM - sum < 3);
}
