//! Integration tests for the fee-bumper sponsorship contract.
//!
//! These run against the contract registered natively (no Wasm build
//! step needed — unlike `contracts/proxy-controller`'s upgrade tests,
//! nothing here needs to prove a real `.wasm` swap), alongside a real
//! Stellar Asset Contract instance for the reimbursement token via
//! `Env::register_stellar_asset_contract_v2`, so `approve`/`transfer_from`/
//! `balance` all run through the actual SEP-41 token contract, not a
//! stand-in.

use fee_bumper::{FeeBumpError, FeeBumperContract, FeeBumperContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

struct Fixture {
    env: Env,
    admin: Address,
    token_address: Address,
    contract: Address,
}

fn setup(max_per_window: u32, window_seconds: u64) -> Fixture {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sac_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin);
    let token_address = sac.address();

    let contract = env.register(
        FeeBumperContract,
        (admin.clone(), token_address.clone(), max_per_window, window_seconds),
    );

    Fixture { env, admin, token_address, contract }
}

impl Fixture {
    fn client(&self) -> FeeBumperContractClient<'_> {
        FeeBumperContractClient::new(&self.env, &self.contract)
    }

    fn token(&self) -> token::TokenClient<'_> {
        token::TokenClient::new(&self.env, &self.token_address)
    }

    /// Fund `account` with `amount` of the reimbursement token and
    /// pre-approve this contract to pull up to `allowance` of it, as a
    /// real sponsored user would do once, out of band, before ever
    /// calling `authorize_sponsorship`.
    fn onboard(&self, account: &Address, amount: i128, allowance: i128) {
        token::StellarAssetClient::new(&self.env, &self.token_address).mint(account, &amount);
        self.token().approve(account, &self.contract, &allowance, &(self.env.ledger().sequence() + 1000));
    }
}

#[test]
fn authorize_then_settle_collects_exact_fee_and_refunds_the_difference() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    let expires_at = fx.client().authorize_sponsorship(&account, &500);
    assert!(expires_at > fx.env.ledger().timestamp());
    assert_eq!(fx.token().balance(&account), 10_000 - 500, "escrowed amount left the account immediately");
    assert_eq!(fx.token().balance(&fx.contract), 500, "escrow sits in the contract's own balance");

    fx.client().settle_reimbursement(&account, &420);

    assert_eq!(fx.token().balance(&fx.admin), 420, "relayer collected exactly the actual fee");
    assert_eq!(fx.token().balance(&account), 10_000 - 420, "the 80 unit overestimate was refunded");
    assert_eq!(fx.token().balance(&fx.contract), 0, "nothing left in escrow");
    assert!(fx.client().pending_sponsorship(&account).is_none());
}

#[test]
fn a_relayer_processing_100_sequential_sponsorships_reimburses_correctly_and_never_exceeds_the_rate_limit() {
    // Generous per-window cap: this test's point is correctness across a
    // long sequential run (the issue's "100 wrapped transactions"
    // scenario), not tripping the limiter — that's covered separately in
    // `rate_limiting_rejects_the_request_once_the_window_cap_is_reached`.
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    let starting_balance: i128 = 1_000_000;
    fx.onboard(&account, starting_balance, starting_balance);

    let mut total_collected: i128 = 0;
    for i in 0..100i128 {
        let estimated_fee = 100 + i; // a varying fee estimate per "transaction"
        let actual_fee = estimated_fee - (i % 3); // relayer's estimate is always >= the real fee

        fx.client().authorize_sponsorship(&account, &estimated_fee);
        fx.client().settle_reimbursement(&account, &actual_fee);
        total_collected += actual_fee;

        assert!(
            fx.client().pending_sponsorship(&account).is_none(),
            "sponsorship {i} must be fully settled before the next one starts"
        );
    }

    assert_eq!(fx.token().balance(&fx.admin), total_collected);
    assert_eq!(fx.token().balance(&account), starting_balance - total_collected);
    assert_eq!(fx.token().balance(&fx.contract), 0, "no residue left escrowed after 100 settled rounds");

    let window = fx.client().account_window(&account).expect("account has an active window");
    assert_eq!(window.count, 100);
}

#[test]
fn rate_limiting_rejects_the_request_once_the_window_cap_is_reached() {
    let fx = setup(2, 1000);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    // Two full authorize/settle cycles consume the window's cap of 2.
    fx.client().authorize_sponsorship(&account, &100);
    fx.client().settle_reimbursement(&account, &100);
    fx.client().authorize_sponsorship(&account, &100);
    fx.client().settle_reimbursement(&account, &100);

    let result = fx.client().try_authorize_sponsorship(&account, &100);
    assert_eq!(result, Err(Ok(FeeBumpError::RateLimited)));

    // The rejected attempt must not have escrowed anything.
    assert_eq!(fx.token().balance(&fx.contract), 0);
}

#[test]
fn cannot_authorize_a_second_sponsorship_while_one_is_already_pending() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    fx.client().authorize_sponsorship(&account, &500);
    let result = fx.client().try_authorize_sponsorship(&account, &200);
    assert_eq!(result, Err(Ok(FeeBumpError::SponsorshipAlreadyPending)));

    // Only the first escrow is held; the rejected second attempt moved nothing.
    assert_eq!(fx.token().balance(&fx.contract), 500);
}

#[test]
fn underfunded_accounts_are_rejected_before_anything_is_escrowed() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    // No `onboard`: zero balance, zero allowance.

    let result = fx.client().try_authorize_sponsorship(&account, &500);
    assert!(result.is_err(), "the token contract must reject the underlying transfer_from");
    assert!(fx.client().pending_sponsorship(&account).is_none());
}

#[test]
fn settling_more_than_the_escrowed_amount_is_rejected_and_leaves_the_escrow_intact() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    fx.client().authorize_sponsorship(&account, &500);
    let result = fx.client().try_settle_reimbursement(&account, &600);
    assert_eq!(result, Err(Ok(FeeBumpError::ActualFeeExceedsEscrow)));

    // The escrow must still be there and settleable with a corrected amount
    // — a rejected settlement must never strand the escrowed funds.
    assert_eq!(fx.token().balance(&fx.contract), 500);
    assert!(fx.client().pending_sponsorship(&account).is_some());

    fx.client().settle_reimbursement(&account, &500);
    assert_eq!(fx.token().balance(&fx.admin), 500);
}

#[test]
fn an_unsettled_escrow_can_be_reclaimed_after_it_expires_but_not_before() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    let expires_at = fx.client().authorize_sponsorship(&account, &500);

    let too_early = fx.client().try_expire_stale_sponsorship(&account);
    assert_eq!(too_early, Err(Ok(FeeBumpError::SponsorshipNotYetExpired)));

    fx.env.ledger().with_mut(|li| li.timestamp = expires_at);
    fx.client().expire_stale_sponsorship(&account);

    assert_eq!(fx.token().balance(&account), 10_000, "the full escrow was refunded");
    assert_eq!(fx.token().balance(&fx.contract), 0);
    assert!(fx.client().pending_sponsorship(&account).is_none());
}

#[test]
fn re_authorizing_after_a_stale_escrow_self_heals_by_refunding_the_old_one_first() {
    let fx = setup(1000, 60);
    let account = Address::generate(&fx.env);
    fx.onboard(&account, 10_000, 10_000);

    let expires_at = fx.client().authorize_sponsorship(&account, &500);
    fx.env.ledger().with_mut(|li| li.timestamp = expires_at);

    // Never explicitly expired — the relayer just tries to sponsor again.
    fx.client().authorize_sponsorship(&account, &300);

    // The stale 500 was refunded before the new 300 was escrowed.
    assert_eq!(fx.token().balance(&account), 10_000 - 300);
    assert_eq!(fx.token().balance(&fx.contract), 300);
    let pending = fx.client().pending_sponsorship(&account).unwrap();
    assert_eq!(pending.escrowed_amount, 300);
}
