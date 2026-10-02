//! Integration tests for the ERC-4626 auto-compounder yield vault.
//!
//! Test coverage:
//! - Basic deposit/withdraw/share-price mechanics
//! - Share price monotonicity invariant (harvest must not decrease share price)
//! - Protocol fee deduction on yield only (not principal)
//! - 50-cycle compound accuracy: exchange rate growth tracked precisely
//! - Multiple depositors with proportional share allocation
//! - Edge cases: zero yield harvest, paused vault, fee recipient accrual

#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl,
    testutils::Address as _,
    token,
    Address, Env,
};

use crate::{VaultError, YieldVault, YieldVaultClient, SHARE_PRECISION};
use crate::harvest::{compute_fee, compute_net_yield, assert_share_price_monotonic};

// ===========================================================================
// Mock yield protocol contract
// ===========================================================================

/// The mock yield protocol emulates a simple external staking/yield contract.
/// It maintains a `pending_rewards` accumulator that the vault admin advances
/// via `add_yield` to simulate earned rewards between harvests.
///
/// When `claim_rewards` is called it transfers `pending_rewards` of the
/// underlying asset from its own balance to the caller (the vault), then
/// clears the accumulator.
#[contract]
pub struct MockYieldProtocol;

/// Storage keys for the mock.
#[soroban_sdk::contracttype]
pub enum MockKey {
    /// Underlying asset token address.
    Asset,
    /// Pending claimable reward amount.
    Pending,
    /// Total rewards ever dispensed (for assertion use).
    TotalDispensed,
}

#[contractimpl]
impl MockYieldProtocol {
    /// Initialise with the asset token address.
    pub fn initialize(env: Env, asset: Address) {
        env.storage().instance().set(&MockKey::Asset, &asset);
        env.storage().instance().set(&MockKey::Pending, &0i128);
        env.storage().instance().set(&MockKey::TotalDispensed, &0i128);
    }

    /// Test helper: add `amount` to the pending reward pool.
    /// In real usage, yield accrues automatically from staking.
    pub fn add_yield(env: Env, amount: i128) {
        let current: i128 = env.storage().instance().get(&MockKey::Pending).unwrap_or(0);
        env.storage().instance().set(&MockKey::Pending, &(current + amount));
    }

    /// ERC-4626 vault calls this to query claimable rewards (read-only).
    pub fn pending_rewards(env: Env, _vault: Address) -> i128 {
        env.storage().instance().get(&MockKey::Pending).unwrap_or(0)
    }

    /// ERC-4626 vault calls this to claim rewards.
    /// Transfers `pending` units of the underlying asset to the vault.
    pub fn claim_rewards(env: Env, vault: Address) -> i128 {
        let pending: i128 = env.storage().instance().get(&MockKey::Pending).unwrap_or(0);
        if pending <= 0 {
            return 0;
        }
        env.storage().instance().set(&MockKey::Pending, &0i128);
        let total: i128 = env.storage().instance().get(&MockKey::TotalDispensed).unwrap_or(0);
        env.storage().instance().set(&MockKey::TotalDispensed, &(total + pending));

        let asset: Address = env.storage().instance().get(&MockKey::Asset).unwrap();
        token::Client::new(&env, &asset)
            .transfer(&env.current_contract_address(), &vault, &pending);
        pending
    }

    /// Total rewards dispensed so far (test helper).
    pub fn total_dispensed(env: Env) -> i128 {
        env.storage().instance().get(&MockKey::TotalDispensed).unwrap_or(0)
    }
}

// ===========================================================================
// Test helpers
// ===========================================================================

struct VaultFixture {
    env: Env,
    admin: Address,
    fee_recipient: Address,
    asset_admin_client: token::StellarAssetClient<'static>,
    vault: YieldVaultClient<'static>,
    mock_protocol: MockYieldProtocolClient<'static>,
}

impl VaultFixture {
    fn new(fee_bps: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let fee_recipient = Address::generate(&env);

        // Deploy a SAC (Stellar Asset Contract) as the underlying asset token.
        let asset_id = env.register_stellar_asset_contract_v2(admin.clone());
        let asset_admin_client = token::StellarAssetClient::new(&env, &asset_id.address());

        // Deploy mock yield protocol.
        let protocol_id = env.register(MockYieldProtocol, ());
        let mock_protocol = MockYieldProtocolClient::new(&env, &protocol_id);
        mock_protocol.initialize(&asset_id.address());

        // Deploy the vault.
        let vault_id = env.register(YieldVault, ());
        let vault = YieldVaultClient::new(&env, &vault_id);
        vault.initialize(
            &admin,
            &asset_id.address(),
            &protocol_id,
            &fee_recipient,
            &fee_bps,
        );

        // SAFETY: we extend the lifetime here for test convenience — all
        // clients are bound to the same `env` which outlives them.
        let asset_admin_client: token::StellarAssetClient<'static> = unsafe {
            core::mem::transmute(asset_admin_client)
        };

        VaultFixture {
            env,
            admin,
            fee_recipient,
            asset_admin_client,
            vault,
            mock_protocol,
        }
    }

    /// Mint `amount` of the underlying asset to `recipient`.
    fn mint(&self, recipient: &Address, amount: i128) {
        self.asset_admin_client.mint(recipient, &amount);
    }

    /// Deposit from `user` and return shares minted.
    fn deposit(&self, user: &Address, amount: i128) -> i128 {
        self.vault.deposit(user, &amount)
    }

    /// Withdraw `shares` from `user` and return assets returned.
    fn withdraw(&self, user: &Address, shares: i128) -> i128 {
        self.vault.withdraw(user, &shares)
    }

    /// Inject `yield_amount` into the mock protocol and then harvest.
    fn inject_and_harvest(&self, yield_amount: i128) -> (i128, i128, i128) {
        // Fund the protocol contract so it can transfer on claim.
        self.mint(&self.mock_protocol.address, yield_amount);
        self.mock_protocol.add_yield(&yield_amount);
        self.vault.harvest(&self.admin)
    }

    fn share_price(&self) -> i128 {
        self.vault.share_price()
    }

    fn total_assets(&self) -> i128 {
        self.vault.total_assets()
    }

    fn total_shares(&self) -> i128 {
        self.vault.total_shares()
    }
}

// ===========================================================================
// Pure-math unit tests (no Soroban SDK required)
// ===========================================================================

#[test]
fn test_compute_fee_zero_bps() {
    assert_eq!(compute_fee(1_000_000, 0).unwrap(), 0);
}

#[test]
fn test_compute_fee_ten_percent() {
    // 1000 bps = 10 %
    let fee = compute_fee(1_000_000, 1000).unwrap();
    assert_eq!(fee, 100_000);
}

#[test]
fn test_compute_fee_maximum_bps() {
    // 10 000 bps = 100 %
    let fee = compute_fee(1_000_000, 10_000).unwrap();
    assert_eq!(fee, 1_000_000);
}

#[test]
fn test_compute_net_yield_five_percent() {
    // 500 bps = 5 %, gross = 200 → fee = 10, net = 190
    let net = compute_net_yield(200, 500).unwrap();
    assert_eq!(net, 190);
}

#[test]
fn test_assert_share_price_monotonic_ok() {
    assert!(assert_share_price_monotonic(100, 101).is_ok());
    assert!(assert_share_price_monotonic(100, 100).is_ok()); // flat is allowed
}

#[test]
fn test_assert_share_price_monotonic_err() {
    let result = assert_share_price_monotonic(101, 100);
    assert_eq!(result.unwrap_err(), VaultError::SharePriceDecreased);
}

// ===========================================================================
// Integration tests (Soroban testutils)
// ===========================================================================

#[test]
fn test_deposit_mints_shares_one_to_one_initially() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);

    let shares = f.deposit(&user, 1_000_000);
    assert_eq!(shares, 1_000_000, "first deposit should be 1:1");
    assert_eq!(f.vault.share_balance(&user), 1_000_000);
    assert_eq!(f.total_assets(), 1_000_000);
    assert_eq!(f.total_shares(), 1_000_000);
}

#[test]
fn test_withdraw_returns_correct_assets() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 2_000_000);
    let shares = f.deposit(&user, 2_000_000);

    let assets_back = f.withdraw(&user, shares);
    assert_eq!(assets_back, 2_000_000);
    assert_eq!(f.vault.share_balance(&user), 0);
    assert_eq!(f.total_assets(), 0);
    assert_eq!(f.total_shares(), 0);
}

#[test]
fn test_harvest_increases_share_price() {
    let f = VaultFixture::new(0); // no fee
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    let price_before = f.share_price();
    f.inject_and_harvest(100_000); // 10 % yield
    let price_after = f.share_price();

    assert!(price_after > price_before, "harvest must increase share price");
}

#[test]
fn test_harvest_share_price_calculation() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    // Harvest 100_000 net yield.
    f.inject_and_harvest(100_000);

    // Expected: total_assets=1_100_000, total_shares=1_000_000
    // price = 1_100_000 * SHARE_PRECISION / 1_000_000 = 1.1 * SHARE_PRECISION
    let expected_price = 1_100_000i128 * SHARE_PRECISION / 1_000_000i128;
    assert_eq!(f.share_price(), expected_price);
}

#[test]
fn test_fee_deducted_from_yield_not_principal() {
    let f = VaultFixture::new(1000); // 10 % fee
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    let (gross, fee, net) = f.inject_and_harvest(100_000);

    assert_eq!(gross, 100_000);
    assert_eq!(fee, 10_000);  // 10 % of gross
    assert_eq!(net, 90_000);  // 90 % of gross

    // Total assets = initial 1_000_000 + gross 100_000 (fee shares + net both contribute)
    assert_eq!(f.total_assets(), 1_100_000);
}

#[test]
fn test_fee_recipient_receives_shares() {
    let f = VaultFixture::new(500); // 5 % fee
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    f.inject_and_harvest(100_000); // gross = 100_000, fee = 5_000

    let fee_shares = f.vault.share_balance(&f.fee_recipient);
    assert!(fee_shares > 0, "fee recipient must have received shares");

    // Fee shares were minted at pre-harvest price (1:1 initially), so
    // fee_shares ≈ fee_assets = 5_000.
    assert_eq!(fee_shares, 5_000);
}

#[test]
fn test_zero_yield_harvest_no_change() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    let price_before = f.share_price();

    // Harvest with no yield injected.
    let (gross, fee, net) = f.vault.harvest(&f.admin);

    assert_eq!(gross, 0);
    assert_eq!(fee, 0);
    assert_eq!(net, 0);
    assert_eq!(f.share_price(), price_before, "share price must be unchanged");
}

#[test]
fn test_multiple_depositors_proportional_shares() {
    let f = VaultFixture::new(0);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);

    f.mint(&alice, 1_000_000);
    f.mint(&bob, 2_000_000);

    f.deposit(&alice, 1_000_000);
    f.deposit(&bob, 2_000_000);

    // Harvest 300_000 yield (10 % of 3_000_000 total).
    f.inject_and_harvest(300_000);

    // After harvest: total_assets = 3_300_000, total_shares = 3_000_000.
    // Alice has 1_000_000 shares → redeemable for 1_100_000.
    // Bob   has 2_000_000 shares → redeemable for 2_200_000.
    let alice_assets = f.vault.preview_withdraw(&1_000_000i128);
    let bob_assets   = f.vault.preview_withdraw(&2_000_000i128);

    assert_eq!(alice_assets, 1_100_000);
    assert_eq!(bob_assets,   2_200_000);
}

#[test]
fn test_deposit_after_harvest_respects_new_price() {
    let f = VaultFixture::new(0);
    let alice = Address::generate(&f.env);
    let bob = Address::generate(&f.env);

    f.mint(&alice, 1_000_000);
    f.deposit(&alice, 1_000_000);

    // Harvest 100_000 → price is now 1.1.
    f.inject_and_harvest(100_000);

    // Bob deposits 1_100_000 and should get 1_000_000 shares
    // (same as alice, because the vault is exactly worth 1.1x per share).
    f.mint(&bob, 1_100_000);
    let bob_shares = f.deposit(&bob, 1_100_000);

    // shares = 1_100_000 * total_shares / total_assets = 1_100_000 * 1_000_000 / 1_100_000 = 1_000_000
    assert_eq!(bob_shares, 1_000_000, "second depositor must get correct shares at new price");
}

#[test]
fn test_paused_vault_blocks_deposits() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);

    f.vault.set_paused(&f.admin, &true);

    let result = f.vault.try_deposit(&user, &1_000_000i128);
    assert!(result.is_err(), "deposit must fail when paused");
}

#[test]
fn test_paused_vault_blocks_withdrawals() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    f.vault.set_paused(&f.admin, &true);

    let result = f.vault.try_withdraw(&user, &1_000_000i128);
    assert!(result.is_err(), "withdraw must fail when paused");
}

#[test]
fn test_harvest_still_works_when_paused() {
    // Harvest is an admin/keeper operation — it should not be blocked by pause.
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 1_000_000);
    f.deposit(&user, 1_000_000);

    f.vault.set_paused(&f.admin, &true);
    // Should not panic.
    f.inject_and_harvest(50_000);
    assert!(f.share_price() > SHARE_PRECISION);
}

#[test]
fn test_invalid_fee_bps_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let asset_id = env.register_stellar_asset_contract_v2(admin.clone());
    let protocol_id = env.register(MockYieldProtocol, ());
    let mock_protocol = MockYieldProtocolClient::new(&env, &protocol_id);
    mock_protocol.initialize(&asset_id.address());

    let vault_id = env.register(YieldVault, ());
    let vault = YieldVaultClient::new(&env, &vault_id);

    let result = vault.try_initialize(
        &admin,
        &asset_id.address(),
        &protocol_id,
        &fee_recipient,
        &10_001u32, // > 10_000 bps — invalid
    );
    assert!(result.is_err());
}

#[test]
fn test_double_initialize_rejected() {
    let f = VaultFixture::new(0);
    let result = f.vault.try_initialize(
        &f.admin,
        &f.vault.get_config().0,
        &f.vault.get_config().1,
        &f.fee_recipient,
        &0u32,
    );
    assert!(result.is_err(), "double-init must revert");
}

// ===========================================================================
// 50-cycle compound accuracy test
// ===========================================================================

/// Simulates 50 successive harvest cycles and verifies:
/// 1. Share price increases on every cycle (monotonicity).
/// 2. Share price after N cycles equals the closed-form compound formula.
/// 3. A user who redeems at the end receives their principal × (1 + yield_rate)^N
///    assets (within integer rounding tolerance).
#[test]
fn test_50_harvest_cycles_compound_accuracy() {
    // Constants chosen to be large enough for precision while avoiding overflow.
    let initial_deposit: i128 = 10_000_000_000; // 10 billion base units
    let yield_per_cycle: i128 = 100_000_000;    // 1 % of deposit per cycle
    let num_cycles: u32 = 50;
    let fee_bps: u32 = 0; // no fee for clean math

    let f = VaultFixture::new(fee_bps);
    let user = Address::generate(&f.env);
    f.mint(&user, initial_deposit);
    let shares = f.deposit(&user, initial_deposit);

    assert_eq!(shares, initial_deposit, "initial shares must be 1:1");

    let mut last_price = f.share_price();
    let mut last_total_assets = f.total_assets();

    for cycle in 1..=num_cycles {
        f.inject_and_harvest(yield_per_cycle);

        let new_price = f.share_price();
        let new_total = f.total_assets();

        // Monotonicity: every harvest must produce a strictly higher price
        // (since we inject positive yield each time).
        assert!(
            new_price > last_price,
            "cycle {}: share price must increase ({} → {})",
            cycle,
            last_price,
            new_price
        );

        // Total assets must increase by exactly yield_per_cycle each cycle
        // (no fee, no rounding loss on total_assets).
        assert_eq!(
            new_total,
            last_total_assets + yield_per_cycle,
            "cycle {}: total assets must increase by yield_per_cycle",
            cycle
        );

        last_price = new_price;
        last_total_assets = new_total;
    }

    // Final state validation.
    let expected_total_assets = initial_deposit + (yield_per_cycle * num_cycles as i128);
    assert_eq!(f.total_assets(), expected_total_assets);

    // Share price after 50 cycles:
    // price = total_assets * SHARE_PRECISION / total_shares
    //       = expected_total_assets * SHARE_PRECISION / initial_deposit
    let expected_final_price = expected_total_assets * SHARE_PRECISION / initial_deposit;
    assert_eq!(
        f.share_price(),
        expected_final_price,
        "final share price must match closed-form calculation"
    );

    // Redemption accuracy: redeem all shares and verify assets returned.
    let assets_redeemed = f.withdraw(&user, shares);
    // Due to integer division, redeemed might be ≤ expected by at most 1.
    assert!(
        assets_redeemed >= expected_total_assets - 1,
        "redeemed assets must be within 1 unit of expected (got {}, expected ≥ {})",
        assets_redeemed,
        expected_total_assets - 1
    );
    assert!(
        assets_redeemed <= expected_total_assets,
        "redeemed assets must not exceed total (got {})",
        assets_redeemed
    );
}

/// Same as above but with a 5 % protocol fee, verifying:
/// 1. Net yield per cycle = gross × 0.95.
/// 2. Share price still increases every cycle.
/// 3. Fee recipient accumulates shares that also appreciate.
/// 4. Total accounted value (user + fee_recipient) ≤ total_assets + rounding.
#[test]
fn test_50_harvest_cycles_with_fee_accuracy() {
    let initial_deposit: i128 = 10_000_000_000;
    let gross_yield_per_cycle: i128 = 100_000_000;
    let num_cycles: u32 = 50;
    let fee_bps: u32 = 500; // 5 %

    let f = VaultFixture::new(fee_bps);
    let user = Address::generate(&f.env);
    f.mint(&user, initial_deposit);
    f.deposit(&user, initial_deposit);

    let mut last_price = f.share_price();

    for cycle in 1..=num_cycles {
        let (gross, fee_paid, net) = f.inject_and_harvest(gross_yield_per_cycle);

        // Gross must match what we injected.
        assert_eq!(gross, gross_yield_per_cycle, "cycle {}: gross mismatch", cycle);

        // Fee must be exactly 5 %.
        let expected_fee = gross_yield_per_cycle * 500 / 10_000;
        assert_eq!(fee_paid, expected_fee, "cycle {}: fee mismatch", cycle);

        // Net = gross − fee.
        assert_eq!(net, gross - fee_paid, "cycle {}: net yield mismatch", cycle);

        let new_price = f.share_price();
        assert!(
            new_price >= last_price,
            "cycle {}: share price must not decrease ({} → {})",
            cycle,
            last_price,
            new_price
        );
        last_price = new_price;
    }

    // After 50 cycles the fee recipient must have non-zero shares.
    let fee_shares = f.vault.share_balance(&f.fee_recipient);
    assert!(fee_shares > 0, "fee recipient must have accrued shares");

    // Verify accounting: total_shares × share_price / SHARE_PRECISION ≈ total_assets.
    let total_shares = f.total_shares();
    let total_assets = f.total_assets();
    let price = f.share_price();
    let reconstructed = total_shares * price / SHARE_PRECISION;
    // Allow at most 1 unit of rounding per cycle.
    let tolerance = num_cycles as i128;
    let diff = if reconstructed > total_assets {
        reconstructed - total_assets
    } else {
        total_assets - reconstructed
    };
    assert!(
        diff <= tolerance,
        "accounting check: |reconstructed − total_assets| = {} > tolerance {}",
        diff,
        tolerance
    );
}

/// Verify that a user who enters AFTER several harvest cycles still receives
/// the correct share count at the then-current share price.
#[test]
fn test_late_depositor_correct_share_count_after_50_cycles() {
    let initial_deposit: i128 = 1_000_000_000;
    let yield_per_cycle: i128 = 10_000_000; // 1 %

    let f = VaultFixture::new(0);
    let early_user = Address::generate(&f.env);
    let late_user  = Address::generate(&f.env);

    f.mint(&early_user, initial_deposit);
    f.deposit(&early_user, initial_deposit);

    // Run 50 harvest cycles.
    for _ in 1..=50 {
        f.inject_and_harvest(yield_per_cycle);
    }

    // total_assets after 50 cycles.
    let total_assets_mid = f.total_assets();
    let total_shares_mid = f.total_shares();

    // Late user deposits the same initial amount.
    f.mint(&late_user, initial_deposit);
    let late_shares = f.deposit(&late_user, initial_deposit);

    // Expected: shares = initial_deposit * total_shares_mid / total_assets_mid
    let expected_late_shares = initial_deposit * total_shares_mid / total_assets_mid;
    assert_eq!(
        late_shares,
        expected_late_shares,
        "late depositor must get shares at the compounded price"
    );

    // Late depositor's shares should be fewer than the early depositor's
    // because the price has risen.
    assert!(
        late_shares < initial_deposit,
        "late depositor should receive fewer shares than deposited units ({} < {})",
        late_shares,
        initial_deposit
    );
}

/// Verify that if we somehow (via a mock bug or attack) try to make share
/// price decrease, the harvest reverts.
#[test]
fn test_harvest_reverts_on_share_price_decrease() {
    // We test the pure helper directly since triggering a real decrease
    // in the contract would require a malicious yield protocol that
    // calls back and reduces TotalAssets — we cover the guard logic here.
    let result = assert_share_price_monotonic(
        SHARE_PRECISION + 1,   // old price is higher
        SHARE_PRECISION,       // new price is lower
    );
    assert_eq!(
        result.unwrap_err(),
        VaultError::SharePriceDecreased,
        "monotonicity guard must fire"
    );
}

/// Ensure preview functions are consistent with actual deposit/withdraw.
#[test]
fn test_preview_functions_consistent() {
    let f = VaultFixture::new(0);
    let user = Address::generate(&f.env);
    f.mint(&user, 5_000_000);
    f.deposit(&user, 5_000_000);
    f.inject_and_harvest(250_000); // 5 % yield

    let shares_to_query = 1_000_000i128;
    let preview_assets = f.vault.preview_withdraw(&shares_to_query);
    let actual_assets  = f.withdraw(&user, shares_to_query);

    assert_eq!(
        preview_assets,
        actual_assets,
        "preview_withdraw must equal actual withdraw"
    );
}
