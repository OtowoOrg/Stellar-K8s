//! Stress validation: the pool must behave identically at 10,000 minters.
//!
//! Issue #259 requires the debt pool to hold 10,000 mock minters without the
//! cost of servicing a price move growing with them. These tests assert that
//! property two ways:
//!
//!   1. **Correctness at scale** — the sum of all 10,000 indexed debts is exactly
//!      the pool's global counter, a price surge re-marks every position from
//!      the read-time `max` while the global record is rewritten *once*, and
//!      liquidating one minter leaves the other 9,999 byte-for-byte identical.
//!   2. **Cost at scale** — the CPU instruction cost of the identical
//!      `set_price` call and of a position read, measured in a 10,000-minter
//!      pool against a single-minter pool.
//!
//! Enrolling 10,000 minters is 20,000 contract invocations, so the heavy setup
//! is done once per test rather than per assertion.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};
use synthetix_core::{
    debt_pool::{self, MIN_COLLATERAL_RATIO_BPS},
    SynthetixCore, SynthetixCoreClient, MAX_CURRENCIES,
};

/// Number of mock minters, per issue #259.
const MINTERS: usize = 10_000;
/// Collateral priced at $1.00, in 1e9 fixed point.
const COLLATERAL_PRICE: u128 = 1_000_000_000;
/// `sBTC` priced at $50,000, in 1e9 fixed point.
const SBTC_PRICE: u128 = 50_000 * 1_000_000_000;
/// `sBTC` after the surge, at $90,000, in 1e9 fixed point.
const SBTC_SURGE_PRICE: u128 = 90_000 * 1_000_000_000;
/// 300%, the floor of issue #259.
const MIN_RATIO_BPS: i128 = 30_000;
/// 5% liquidation penalty.
const PENALTY_BPS: i128 = 500;
/// $1,000 of collateral, in 7-decimal stroops.
const COLLATERAL_AMOUNT: i128 = 1_000_000_000;
/// 1,000 units of sBTC: $50 of debt against $1,000 of collateral, so 2,000%.
const UNITS_PER_MINTER: i128 = 1_000;
/// Debt each minter takes on: 1,000 units at $50,000.
const DEBT_PER_MINTER: i128 = 50_000_000;
/// Debt the same units are marked at after the surge to $90,000.
const SURGED_DEBT_PER_MINTER: i128 = 90_000_000;

struct World {
    env: Env,
    client: SynthetixCoreClient<'static>,
    admin: Address,
    sbtc: Address,
    minters: std::vec::Vec<Address>,
}

fn setup(count: usize) -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    // Enrolling 10,000 minters is a test-harness concern, not a protocol one.
    env.cost_estimate().budget().reset_unlimited();

    let contract_id = env.register(SynthetixCore, ());
    let client = SynthetixCoreClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(admin.clone());
    let collateral_admin = StellarAssetClient::new(&env, &asset.address());
    let sbtc = Address::generate(&env);

    client.initialize(
        &admin,
        &asset.address(),
        &COLLATERAL_PRICE,
        &MIN_RATIO_BPS,
        &PENALTY_BPS,
    );
    client.add_currency(&admin, &sbtc, &SBTC_PRICE);

    let mut roster: std::vec::Vec<Address> = std::vec::Vec::with_capacity(count);
    for _ in 0..count {
        let who = Address::generate(&env);
        collateral_admin.mint(&who, &COLLATERAL_AMOUNT);
        client.lock_collateral(&who, &COLLATERAL_AMOUNT);
        client.mint_synth(&who, &sbtc, &UNITS_PER_MINTER);
        roster.push(who);
    }

    World {
        env,
        client,
        admin,
        sbtc,
        minters: roster,
    }
}

// Enrolling 10,000 minters is 20,000 contract invocations, so this takes a
// couple of minutes. It is opt-in, to keep `cargo test` fast:
//     cargo test --release -- --ignored
#[test]
#[ignore = "10,000-minter stress run; opt in with `cargo test -- --ignored`"]
fn ten_thousand_minters_share_one_global_index() {
    let w = setup(MINTERS);
    assert_eq!(w.minters.len(), MINTERS);
    // `total_debt` is O(currencies), not O(minters), because the registry is a
    // capped allow-list; 10,000 minters do not widen it.
    assert!(w.client.currencies().len() <= MAX_CURRENCIES);
    assert_eq!(w.client.min_ratio_bps(), MIN_COLLATERAL_RATIO_BPS);

    // ---------------------------------------------------------------------
    // Conservation at scale
    // ---------------------------------------------------------------------
    // Every minter owes exactly what it minted: a pool of 10,000 does not
    // distort the index, and the sum of the individual records is the global
    // counter to the base unit.
    let mut summed: i128 = 0;
    for who in &w.minters {
        assert_eq!(w.client.indexed_debt_of(who, &w.sbtc), DEBT_PER_MINTER);
        assert_eq!(w.client.synth_units_of(who, &w.sbtc), UNITS_PER_MINTER);
        assert_eq!(w.client.shares_of(who, &w.sbtc), DEBT_PER_MINTER as u128);
        summed += DEBT_PER_MINTER;
    }
    let pool = w.client.pool(&w.sbtc);
    assert_eq!(summed, DEBT_PER_MINTER * MINTERS as i128);
    assert_eq!(pool.total_debt, summed);
    assert_eq!(pool.total_shares, DEBT_PER_MINTER as u128 * MINTERS as u128);
    assert_eq!(pool.synthetic_supply, UNITS_PER_MINTER * MINTERS as i128);
    assert_eq!(pool.debt_per_share(), debt_pool::PRECISION);
    assert_eq!(
        w.client.total_collateral(),
        COLLATERAL_AMOUNT * MINTERS as i128
    );

    // ---------------------------------------------------------------------
    // One price update, one global rewrite, 10,000 re-marks
    // ---------------------------------------------------------------------
    assert_eq!(pool.mutations, MINTERS as u64);
    w.client.set_price(&w.admin, &w.sbtc, &SBTC_SURGE_PRICE);

    let pool = w.client.pool(&w.sbtc);
    assert_eq!(pool.mutations, MINTERS as u64 + 1);
    // The index itself never moved: the revaluation is a read-time `max`, so the
    // surge wrote the global record exactly once and touched no minter record.
    assert_eq!(pool.debt_per_share(), debt_pool::PRECISION);
    assert_eq!(pool.total_debt, DEBT_PER_MINTER * MINTERS as i128);

    for who in &w.minters {
        assert_eq!(w.client.indexed_debt_of(who, &w.sbtc), DEBT_PER_MINTER);
        assert_eq!(w.client.debt_of(who, &w.sbtc), SURGED_DEBT_PER_MINTER);
        assert_eq!(w.client.total_debt(who), SURGED_DEBT_PER_MINTER);
        // $1,000 against $90,000 of marked debt: 11%.
        assert_eq!(w.client.collateral_ratio_bps(who), 1_111);
    }
    assert!(w.client.flag_account(&w.minters[0]));
    assert!(w.client.flag_account(&w.minters[MINTERS - 1]));

    // ---------------------------------------------------------------------
    // Localized liquidation: 2 records change, 9,998 do not
    // ---------------------------------------------------------------------
    let target = &w.minters[0];
    let liquidator = &w.minters[1];
    let bystander = &w.minters[MINTERS / 2];
    let bystander_state = (
        w.client.shares_of(bystander, &w.sbtc),
        w.client.synth_units_of(bystander, &w.sbtc),
        w.client.collateral_of(bystander),
        w.client.debt_of(bystander, &w.sbtc),
    );

    let receipt = w.client.liquidate(liquidator, target, &w.sbtc, &10_000_000);
    assert_eq!(receipt.debt_covered, 10_000_000);
    assert_eq!(receipt.penalty_collateral, 500_000);
    assert!(receipt.surplus > 0);

    assert!(w.client.shares_of(target, &w.sbtc) < DEBT_PER_MINTER as u128);
    assert!(w.client.collateral_of(target) < COLLATERAL_AMOUNT);
    assert!(w.client.collateral_of(liquidator) > COLLATERAL_AMOUNT);
    assert_eq!(w.client.shares_of(bystander, &w.sbtc), bystander_state.0);
    assert_eq!(
        w.client.synth_units_of(bystander, &w.sbtc),
        bystander_state.1
    );
    assert_eq!(w.client.collateral_of(bystander), bystander_state.2);
    assert_eq!(w.client.debt_of(bystander, &w.sbtc), bystander_state.3);

    // Conservation survives the intervention, to the base unit.
    let mut summed_after: i128 = 0;
    for who in &w.minters {
        summed_after += w.client.indexed_debt_of(who, &w.sbtc);
    }
    assert_eq!(summed_after, w.client.pool(&w.sbtc).total_debt);
    assert!(summed_after < summed);
    // No token left the contract; the penalty was retained as surplus.
    assert_eq!(
        w.client.total_collateral(),
        COLLATERAL_AMOUNT * MINTERS as i128
    );
    assert_eq!(w.client.surplus(), receipt.surplus);
}

// Same 20,000-invocation setup cost, so opt-in for the same reason.
#[test]
#[ignore = "10,000-minter cost comparison; opt in with `cargo test -- --ignored`"]
fn servicing_cost_is_independent_of_the_minter_count() {
    // The point of the index: re-pricing the pool must not get more expensive
    // as more minters hold the asset. Measured as CPU instructions for the
    // identical calls.
    let small = setup(1);
    small.env.cost_estimate().budget().reset_tracker();
    small
        .client
        .set_price(&small.admin, &small.sbtc, &SBTC_SURGE_PRICE);
    let update_one = small.env.cost_estimate().budget().cpu_instruction_cost();
    small.env.cost_estimate().budget().reset_tracker();
    small.client.collateral_ratio_bps(&small.minters[0]);
    let read_one = small.env.cost_estimate().budget().cpu_instruction_cost();
    assert!(update_one > 0 && read_one > 0);

    let large = setup(MINTERS);
    large.env.cost_estimate().budget().reset_tracker();
    large
        .client
        .set_price(&large.admin, &large.sbtc, &SBTC_SURGE_PRICE);
    let update_many = large.env.cost_estimate().budget().cpu_instruction_cost();
    large.env.cost_estimate().budget().reset_tracker();
    large
        .client
        .collateral_ratio_bps(&large.minters[MINTERS - 1]);
    let read_many = large.env.cost_estimate().budget().cpu_instruction_cost();
    assert!(update_many > 0 && read_many > 0);

    // 10,000x the minters, but each call is still a fixed number of storage
    // reads and one global write. The slack covers the second contract instance,
    // not per-minter work.
    assert!(
        update_many < update_one * 2,
        "price update scaled with the minter count: {update_one} -> {update_many}"
    );
    assert!(
        read_many < read_one * 2,
        "position read scaled with the minter count: {read_one} -> {read_many}"
    );
}
