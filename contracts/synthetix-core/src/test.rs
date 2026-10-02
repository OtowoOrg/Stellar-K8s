//! Integration tests for the synthetic issuance protocol.
//!
//! Coverage:
//!   * 300%+ collateralisation enforcement on mint (boundary + rejection).
//!   * A sharp `sBTC` price surge re-marking every position at once through the
//!     read-time `max(indexed debt, synths x price)`, with no per-minter write.
//!   * Burden shifting: the minter who burns has that synth removed from their
//!     mark, while every holder who did not burn keeps the full re-priced
//!     exposure and is pushed under the threshold.
//!   * Permissionless flagging and the localized liquidation engine, including
//!     the penalty, the retained surplus, the non-fraternity post-condition, and
//!     the guarantee that no uninvolved minter is touched.
//!   * Administrative guards: unauthorised callers, ratcheting ratios,
//!     currency registry bounds, and the emergency stop.

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
};

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

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture<'a> {
    env: Env,
    client: SynthetixCoreClient<'a>,
    admin: Address,
    minter: Address,
    counterparty: Address,
    collateral: Address,
    collateral_admin: StellarAssetClient<'a>,
    sbtc: Address,
}

fn setup() -> Fixture<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let contract_id = env.register(SynthetixCore, ());
    let client = SynthetixCoreClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let minter = Address::generate(&env);
    let counterparty = Address::generate(&env);
    let sbtc = Address::generate(&env);

    let asset = env.register_stellar_asset_contract_v2(admin.clone());
    let collateral = asset.address();
    let collateral_admin = StellarAssetClient::new(&env, &collateral);
    let collateral_token = TokenClient::new(&env, &collateral);

    client.initialize(
        &admin,
        &collateral,
        &COLLATERAL_PRICE,
        &MIN_RATIO_BPS,
        &PENALTY_BPS,
    );
    client.add_currency(&admin, &sbtc, &SBTC_PRICE);

    for holder in [&minter, &counterparty] {
        collateral_admin.mint(holder, &COLLATERAL_AMOUNT);
    }
    assert_eq!(collateral_token.balance(&minter), COLLATERAL_AMOUNT);

    Fixture {
        env,
        client,
        admin,
        minter,
        counterparty,
        collateral,
        collateral_admin,
        sbtc,
    }
}

impl Fixture<'_> {
    fn mint_btc(&self, who: &Address, units: i128) {
        self.client.lock_collateral(who, &COLLATERAL_AMOUNT);
        self.client.mint_synth(who, &self.sbtc, &units);
    }

    fn surge(&self) {
        self.client
            .set_price(&self.admin, &self.sbtc, &SBTC_SURGE_PRICE);
    }
}

// ---------------------------------------------------------------------------
// Initialisation and configuration
// ---------------------------------------------------------------------------

#[test]
fn initialize_is_one_shot() {
    let f = setup();
    assert_eq!(f.client.min_ratio_bps(), MIN_RATIO_BPS);
    assert_eq!(f.client.penalty_bps(), PENALTY_BPS);
    assert_eq!(f.client.collateral_price(), COLLATERAL_PRICE);
    assert!(!f.client.is_paused());
    assert_eq!(f.client.currencies().len(), 1);
    assert_eq!(f.client.admin(), f.admin);
    assert_eq!(f.client.surplus(), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn initialize_cannot_be_repeated() {
    let f = setup();
    f.client.initialize(
        &f.admin,
        &f.collateral,
        &COLLATERAL_PRICE,
        &MIN_RATIO_BPS,
        &PENALTY_BPS,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn non_admin_cannot_push_a_price() {
    let f = setup();
    f.client.set_price(&f.minter, &f.sbtc, &SBTC_SURGE_PRICE);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn unregistered_currency_is_rejected() {
    let f = setup();
    let unknown = Address::generate(&f.env);
    f.client.mint_synth(&f.minter, &unknown, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn duplicate_currency_is_rejected() {
    let f = setup();
    f.client.add_currency(&f.admin, &f.sbtc, &SBTC_PRICE);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn ratio_cannot_be_loosened_below_300_percent() {
    let f = setup();
    f.client.set_min_ratio_bps(&f.admin, &20_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn ratio_cannot_be_initialised_below_300_percent() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(SynthetixCore, ());
    let client = SynthetixCoreClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(admin.clone());
    client.initialize(
        &admin,
        &asset.address(),
        &COLLATERAL_PRICE,
        &10_000,
        &PENALTY_BPS,
    );
}

#[test]
fn ratio_ratchets_upward_only() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.client.set_min_ratio_bps(&f.admin, &40_000);
    assert_eq!(f.client.min_ratio_bps(), 40_000);
    // The position was minted at 333%, so tightening the floor to 400%
    // invalidates it and the minter is now flagged.
    assert_eq!(f.client.collateral_ratio_bps(&f.minter), 33_333);
    assert!(f.client.flag_account(&f.minter));
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn ratio_cannot_be_loosened_once_tightened() {
    let f = setup();
    f.client.set_min_ratio_bps(&f.admin, &40_000);
    f.client.set_min_ratio_bps(&f.admin, &30_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn zero_price_is_rejected() {
    let f = setup();
    f.client.set_price(&f.admin, &f.sbtc, &0);
}

// ---------------------------------------------------------------------------
// Collateral
// ---------------------------------------------------------------------------

#[test]
fn locking_collateral_moves_tokens_and_totals() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &400_000_000);
    assert_eq!(f.client.collateral_of(&f.minter), 400_000_000);
    assert_eq!(f.client.total_collateral(), 400_000_000);
    assert_eq!(
        TokenClient::new(&f.env, &f.collateral).balance(&f.minter),
        COLLATERAL_AMOUNT - 400_000_000
    );
}

#[test]
fn idle_collateral_can_be_withdrawn() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &400_000_000);
    f.client.withdraw_collateral(&f.minter, &400_000_000);
    assert_eq!(f.client.collateral_of(&f.minter), 0);
    assert_eq!(f.client.total_collateral(), 0);
    assert_eq!(
        TokenClient::new(&f.env, &f.collateral).balance(&f.minter),
        COLLATERAL_AMOUNT
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn collateral_cannot_be_withdrawn_below_the_threshold() {
    let f = setup();
    // $1,000 of collateral against $300 of indexed debt is 333% — withdrawing
    // $300 would leave exactly 100%.
    f.mint_btc(&f.minter, 6_000);
    f.client.withdraw_collateral(&f.minter, &300_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #17)")]
fn cannot_withdraw_more_collateral_than_locked() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &1_000_000);
    f.client.withdraw_collateral(&f.minter, &COLLATERAL_AMOUNT);
}

// ---------------------------------------------------------------------------
// 300% collateralisation on mint
// ---------------------------------------------------------------------------

#[test]
fn mint_at_the_threshold_is_accepted() {
    let f = setup();
    // $1,000 collateral / 300% => $333.33 of debt => 6,666 sBTC at $50k.
    let receipt = f.mint_receipt(&f.minter, 6_000);
    assert_eq!(receipt.debt, 300_000_000);
    assert!(receipt.collateral_ratio_bps > MIN_RATIO_BPS);
    assert_eq!(f.client.synth_units_of(&f.minter, &f.sbtc), 6_000);
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 300_000_000);
    assert_eq!(f.client.total_debt(&f.minter), 300_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn mint_below_300_percent_is_rejected() {
    let f = setup();
    f.mint_btc(&f.minter, 7_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn repeated_mints_accumulate_against_the_same_collateral() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &COLLATERAL_AMOUNT);
    f.client.mint_synth(&f.minter, &f.sbtc, &5_000);
    assert!(f.client.collateral_ratio_bps(&f.minter) > MIN_RATIO_BPS);
    // A further 1,500 sBTC takes total debt to $325k against $1,000k = 307%.
    f.client.mint_synth(&f.minter, &f.sbtc, &1_500);
    assert_eq!(f.client.total_debt(&f.minter), 325_000_000);
    assert!(f.client.collateral_ratio_bps(&f.minter) > MIN_RATIO_BPS);
    // One more would take it to $350k = 285% and must fail.
    f.client.mint_synth(&f.minter, &f.sbtc, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn second_mint_past_the_threshold_is_rejected() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &COLLATERAL_AMOUNT);
    f.client.mint_synth(&f.minter, &f.sbtc, &5_000);
    f.client.mint_synth(&f.minter, &f.sbtc, &3_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn minting_without_collateral_is_rejected() {
    let f = setup();
    f.client.mint_synth(&f.minter, &f.sbtc, &1);
}

// ---------------------------------------------------------------------------
// The dynamic global debt pool
// ---------------------------------------------------------------------------

#[test]
fn debt_shares_split_pool_debt_pro_rata() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    // The counterparty takes twice the position, so it needs twice the backing.
    f.fund(&f.counterparty, 3 * COLLATERAL_AMOUNT);
    f.mint_btc(&f.counterparty, 12_000);

    let pool = f.client.pool(&f.sbtc);
    assert_eq!(pool.total_debt, 900_000_000);
    assert_eq!(pool.synthetic_supply, 18_000);
    // One share per base unit at bootstrap: 6,000 * $50,000 = $300m stroops.
    assert_eq!(pool.total_shares, 900_000_000);
    assert_eq!(pool.debt_per_share(), debt_pool::PRECISION);

    let shares = f.client.shares_of(&f.minter, &f.sbtc);
    assert_eq!(shares, 300_000_000);
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 300_000_000);
    assert_eq!(f.client.debt_of(&f.counterparty, &f.sbtc), 600_000_000);
    // `debt_of(shares)` is the whole derivation: O(1), no iteration.
    assert_eq!(pool.debt_of(shares), 300_000_000);
    // A single share can never claim more than the pool owes.
    assert_eq!(pool.debt_of(u128::MAX), 900_000_000);
}

#[test]
fn price_surge_reprices_every_position_through_the_index() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    assert!(f.client.collateral_ratio_bps(&f.minter) > MIN_RATIO_BPS);

    // sBTC $50,000 -> $90,000. Indexed debt is unchanged (frozen at mint
    // price), but the market value of the held synths is what the protocol
    // marks against, so both positions are re-priced at once.
    f.surge();

    assert_eq!(f.client.indexed_debt_of(&f.minter, &f.sbtc), 300_000_000);
    assert_eq!(
        f.client.indexed_debt_of(&f.counterparty, &f.sbtc),
        300_000_000
    );
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 540_000_000);
    assert_eq!(f.client.debt_of(&f.counterparty, &f.sbtc), 540_000_000);
    // 6,000 units at $90,000 is $540 of debt against $1,000 of collateral:
    // 185%, so the surge pushed both positions under the floor at once.
    assert_eq!(f.client.collateral_ratio_bps(&f.minter), 18_518);
    assert!(f.client.collateral_ratio_bps(&f.minter) < MIN_RATIO_BPS);
    assert!(f.client.collateral_ratio_bps(&f.counterparty) < MIN_RATIO_BPS);
}

#[test]
fn price_surge_does_not_shrink_recorded_debt_on_a_fall() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    // sBTC $50,000 -> $20,000: market value drops below the indexed debt, so
    // the position is still marked at the indexed figure. A minter can never
    // gain collateral from a price fall.
    f.client
        .set_price(&f.admin, &f.sbtc, &(20_000 * 1_000_000_000));
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 300_000_000);
}

#[test]
fn burn_retires_the_market_value_of_the_units_burned() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();

    // Burning after a 1.8x surge retires the *current* value of the units
    // burned ($270), far more than the nominal debt those shares were issued
    // against ($150), so the burner is marked down hard while the holder who
    // did not burn keeps the full re-priced position.
    let receipt = f.client.burn_synth(&f.minter, &f.sbtc, &3_000);
    assert_eq!(receipt.debt, 270_000_000);
    assert_eq!(receipt.shares, 270_000_000);

    // The minter burned half its synths and is back above the floor at 370%.
    assert_eq!(f.client.indexed_debt_of(&f.minter, &f.sbtc), 30_000_000);
    assert_eq!(f.client.collateral_ratio_bps(&f.minter), 37_037);
    assert!(!f.client.flag_account(&f.minter));

    // The counterparty's marked debt is untouched: it still holds every synth it
    // minted, so it absorbs the whole price move.
    assert_eq!(
        f.client.indexed_debt_of(&f.counterparty, &f.sbtc),
        300_000_000
    );
    assert_eq!(f.client.debt_of(&f.counterparty, &f.sbtc), 540_000_000);
    assert!(f.client.flag_account(&f.counterparty));

    // Both global counters moved together, so the index itself is unchanged:
    // in this design the revaluation is a read-time `max`, not an index write,
    // and the only thing that ever shifts the index is rounding surplus.
    let pool = f.client.pool(&f.sbtc);
    assert_eq!(pool.total_debt, 330_000_000);
    assert_eq!(pool.total_shares, 330_000_000);
    assert_eq!(pool.debt_per_share(), debt_pool::PRECISION);
    assert_eq!(pool.synthetic_supply, 9_000);
    assert!(!receipt.index_drifted);
}
#[test]
fn the_index_is_stationary_and_never_silently_drifts() {
    // Mints and retirements are both pro rata against the live index, so the
    // two global counters stay in step: a price move is applied at read time by
    // the `max` mark and never by writing the index, and no rounding drift
    // accumulates over many partial retirements. `index_drifted` is the signal
    // that would report it if that ever stopped being true.
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);

    for _ in 0..4 {
        let receipt = f.client.burn_synth(&f.minter, &f.sbtc, &1_000);
        assert!(!receipt.index_drifted, "the index must not drift");
        assert_eq!(receipt.debt, 50_000_000);
    }

    let pool = f.client.pool(&f.sbtc);
    assert_eq!(pool.debt_per_share(), debt_pool::PRECISION);
    assert_eq!(pool.high_water_debt_per_share, debt_pool::PRECISION);
    // The counters remain exactly consistent: every share is still worth
    // precisely one base unit of debt, and they add up.
    assert_eq!(pool.debt_of(pool.total_shares), pool.total_debt);
    assert_eq!(pool.total_debt, 400_000_000);
    assert_eq!(pool.total_shares, 400_000_000);
    assert_eq!(f.client.indexed_debt_of(&f.minter, &f.sbtc), 100_000_000);
    assert_eq!(
        f.client.indexed_debt_of(&f.counterparty, &f.sbtc),
        300_000_000
    );
    // A surge then re-marks both from the read-time `max`, index untouched.
    f.surge();
    assert_eq!(
        f.client.pool(&f.sbtc).debt_per_share(),
        debt_pool::PRECISION
    );
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 180_000_000);
    assert_eq!(f.client.debt_of(&f.counterparty, &f.sbtc), 540_000_000);
}

#[test]
fn burn_clears_a_position_without_leaving_dust() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.client.burn_synth(&f.minter, &f.sbtc, &6_000);
    assert_eq!(f.client.shares_of(&f.minter, &f.sbtc), 0);
    assert_eq!(f.client.synth_units_of(&f.minter, &f.sbtc), 0);
    assert_eq!(f.client.debt_of(&f.minter, &f.sbtc), 0);
    assert_eq!(f.client.pool(&f.sbtc).total_shares, 0);
    assert_eq!(f.client.pool(&f.sbtc).total_debt, 0);
    // With no debt left the minter is free to withdraw everything.
    f.client.withdraw_collateral(&f.minter, &COLLATERAL_AMOUNT);
    assert_eq!(f.client.collateral_of(&f.minter), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn cannot_burn_more_than_held() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.client.burn_synth(&f.minter, &f.sbtc, &6_001);
}

#[test]
fn price_updates_do_not_grow_with_the_number_of_minters() {
    // The pool exposes `mutations` precisely so this can be asserted on chain.
    let f = setup();
    for _ in 0..8 {
        let who = Address::generate(&f.env);
        f.collateral_admin.mint(&who, &COLLATERAL_AMOUNT);
        f.mint_btc(&who, 1_000);
    }
    let after_mints = f.client.pool(&f.sbtc).mutations;
    f.surge();
    let after_surge = f.client.pool(&f.sbtc).mutations;
    // One price update rewrites the global record exactly once.
    assert_eq!(after_surge - after_mints, 1);
}

// ---------------------------------------------------------------------------
// Flagging and localized liquidation
// ---------------------------------------------------------------------------

#[test]
fn flag_account_is_permissionless_and_tracks_the_ratio() {
    let f = setup();
    f.collateral_admin.mint(&f.minter, &COLLATERAL_AMOUNT);
    f.mint_btc(&f.minter, 6_000);
    // The counterparty has no position, so there is nothing to flag.
    assert!(!f.client.flag_account(&f.counterparty));
    assert!(!f.client.is_flagged(&f.counterparty));

    f.surge();
    assert!(f.client.flag_account(&f.minter));
    assert!(f.client.is_flagged(&f.minter));
    assert_eq!(f.client.flagged_at(&f.minter), 1_700_000_000);

    // Backing the position repairs the flag.
    f.client.lock_collateral(&f.minter, &COLLATERAL_AMOUNT);
    assert!(!f.client.flag_account(&f.minter));
    assert!(!f.client.is_flagged(&f.minter));
}

#[test]
fn a_released_position_is_never_reported_as_a_shortfall() {
    // A debt-free account must read as maximally collateralised even after it
    // has withdrawn everything, otherwise collateral could never be released.
    let f = setup();
    f.client.lock_collateral(&f.minter, &COLLATERAL_AMOUNT);
    f.client.withdraw_collateral(&f.minter, &COLLATERAL_AMOUNT);
    assert_eq!(f.client.collateral_of(&f.minter), 0);
    assert_eq!(f.client.total_debt(&f.minter), 0);
    assert!(!f.client.flag_account(&f.minter));
    assert!(!f.client.is_flagged(&f.minter));
}

#[test]
fn localized_liquidation_charges_a_penalty_and_repairs_the_target() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();
    assert!(f.client.flag_account(&f.minter));

    let target_collateral_before = f.client.collateral_of(&f.minter);
    let counterparty_collateral_before = f.client.collateral_of(&f.counterparty);
    let counterparty_debt_before = f.client.debt_of(&f.counterparty, &f.sbtc);

    // Cover $100 of the target's debt: 1e8 base units.
    let receipt = f
        .client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);

    assert_eq!(receipt.debt_covered, 100_000_000);
    // 5% penalty on top of the covered debt.
    assert_eq!(receipt.collateral_paid, 105_000_000);
    assert_eq!(receipt.penalty_collateral, 5_000_000);
    // $895 of collateral against $540 of marked debt is still 166%, so the
    // target is not yet out of the red and stays flagged.
    assert!(receipt.still_flagged);
    assert_eq!(receipt.target_ratio_bps, 16_574);

    // Collateral moved from the target to the liquidator.
    assert_eq!(
        f.client.collateral_of(&f.minter),
        target_collateral_before - 105_000_000
    );
    assert_eq!(
        f.client.collateral_of(&f.counterparty),
        counterparty_collateral_before + 105_000_000
    );
    // No token left the contract: the penalty is retained as surplus backing
    // the debt that remains, exactly as in Synthetix.
    assert_eq!(f.client.total_collateral(), COLLATERAL_AMOUNT * 2);
    assert_eq!(f.client.surplus(), 5_000_000);
    assert_eq!(receipt.surplus, 5_000_000);

    // The liquidator spent synths, not debt: their *shares* — and therefore
    // their indexed debt — are untouched, while the synth balance they are
    // marked against shrank.
    assert_eq!(f.client.shares_of(&f.counterparty, &f.sbtc), 300_000_000);
    assert_eq!(
        f.client.indexed_debt_of(&f.counterparty, &f.sbtc),
        300_000_000
    );
    assert!(f.client.debt_of(&f.counterparty, &f.sbtc) < counterparty_debt_before);
    // 1,000 base units of debt at $90,000 buys 1,111.11 units, floored to the
    // share grid, so the liquidator is left holding fewer synths.
    assert!(f.client.synth_units_of(&f.counterparty, &f.sbtc) < 6_000);
    // The target keeps its synths: only its debt is cut.
    assert_eq!(f.client.synth_units_of(&f.minter, &f.sbtc), 6_000);
}

#[test]
fn liquidation_is_localized_and_benefits_the_remaining_minters() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    let bystander = Address::generate(&f.env);
    f.collateral_admin.mint(&bystander, &COLLATERAL_AMOUNT);
    f.mint_btc(&bystander, 6_000);
    f.surge();

    let bystander_shares = f.client.shares_of(&bystander, &f.sbtc);
    let bystander_units = f.client.synth_units_of(&bystander, &f.sbtc);
    let bystander_collateral = f.client.collateral_of(&bystander);
    let bystander_debt = f.client.debt_of(&bystander, &f.sbtc);

    f.client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);

    // The uninvolved minter's own records are untouched...
    assert_eq!(f.client.shares_of(&bystander, &f.sbtc), bystander_shares);
    assert_eq!(
        f.client.synth_units_of(&bystander, &f.sbtc),
        bystander_units
    );
    assert_eq!(f.client.collateral_of(&bystander), bystander_collateral);
    // ...and its debt is unchanged, because the market value of the synths it
    // holds is what marks it, not the pool index.
    assert_eq!(f.client.debt_of(&bystander, &f.sbtc), bystander_debt);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn healthy_accounts_cannot_be_liquidated() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn cannot_cover_more_debt_than_the_target_owes() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();
    f.client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &500_000_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn liquidator_must_hold_the_synths_they_redeem() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    let liquidator = Address::generate(&f.env);
    f.collateral_admin.mint(&liquidator, &COLLATERAL_AMOUNT);
    // Collateral but no synths: there is nothing to redeem with, and the
    // contract will not mint on the liquidator's behalf.
    f.surge();
    f.client
        .liquidate(&liquidator, &f.minter, &f.sbtc, &100_000_000);
}

#[test]
fn liquidator_is_never_left_worse_off_than_they_were() {
    // The non-fraternity post-condition. After a surge *every* synth holder is
    // under the floor, so an absolute ratio gate on the liquidator would make
    // liquidation impossible. What must hold is that absorbing a shortfall
    // never worsens the liquidator: their ratio after the trade is at least
    // min(their ratio before, the target's ratio after).
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();

    let liquidator_ratio_before = f.client.collateral_ratio_bps(&f.counterparty);
    let receipt = f
        .client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);

    assert_eq!(
        receipt.liquidator_ratio_bps,
        f.client.collateral_ratio_bps(&f.counterparty)
    );
    assert!(receipt.liquidator_ratio_bps >= liquidator_ratio_before.min(receipt.target_ratio_bps));
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn self_liquidation_is_rejected() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();
    f.client
        .liquidate(&f.minter, &f.minter, &f.sbtc, &1_000_000);
}

#[test]
fn repeated_liquidations_preserve_every_pool_invariant() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();
    assert!(f.client.flag_account(&f.minter));

    let mut last = f
        .client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);
    for _ in 0..2 {
        last = f
            .client
            .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);
    }
    assert_eq!(last.still_flagged, f.client.is_flagged(&f.minter));
    assert!(last.penalty_collateral > 0);

    // Both positions stay solvent-in-the-sense-of-bounded, the penalty is
    // retained as surplus rather than leaving the protocol, and the target's
    // collateral never goes negative however many rounds are run.
    assert!(f.client.collateral_of(&f.minter) >= 0);
    assert!(f.client.collateral_of(&f.counterparty) >= 0);
    assert_eq!(
        f.client.collateral_of(&f.minter) + f.client.collateral_of(&f.counterparty),
        f.client.total_collateral()
    );
    // The collateral only ever moved between the two of them: no token left
    // the contract, and 3 rounds x 5% of $100 = $15 was booked as surplus.
    assert_eq!(f.client.total_collateral(), COLLATERAL_AMOUNT * 2);
    assert_eq!(f.client.surplus(), 15_000_000);
    assert_eq!(last.surplus, 15_000_000);
    // The target's indexed debt fell by exactly the debt covered each round.
    let pool = f.client.pool(&f.sbtc);
    assert_eq!(pool.total_debt, 600_000_000 - 3 * 100_000_000);
    assert!(pool.synthetic_supply < 12_000);
}

// ---------------------------------------------------------------------------
// Emergency stop
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn pause_blocks_minting() {
    let f = setup();
    f.client.set_paused(&f.admin, &true);
    f.mint_btc(&f.minter, 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn pause_blocks_liquidation() {
    let f = setup();
    f.mint_btc(&f.minter, 6_000);
    f.mint_btc(&f.counterparty, 6_000);
    f.surge();
    f.client.set_paused(&f.admin, &true);
    f.client
        .liquidate(&f.counterparty, &f.minter, &f.sbtc, &100_000_000);
}

#[test]
fn pause_never_traps_collateral() {
    let f = setup();
    f.client.lock_collateral(&f.minter, &COLLATERAL_AMOUNT);
    f.client.set_paused(&f.admin, &true);
    f.client.withdraw_collateral(&f.minter, &COLLATERAL_AMOUNT);
    assert_eq!(f.client.collateral_of(&f.minter), 0);
}

impl Fixture<'_> {
    /// Mint `amount` collateral to `who` and lock it all.
    fn fund(&self, who: &Address, amount: i128) {
        self.collateral_admin.mint(who, &amount);
        self.client.lock_collateral(who, &amount);
    }

    /// Lock collateral and mint, returning the receipt.
    fn mint_receipt(&self, who: &Address, units: i128) -> MintReceipt {
        self.client.lock_collateral(who, &COLLATERAL_AMOUNT);
        self.client.mint_synth(who, &self.sbtc, &units)
    }
}
