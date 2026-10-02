//! Shared harness: replays a fuzzer-chosen sequence of operations against the
//! vault + lending pair and checks protocol invariants after every step.

// Each fuzz binary uses only one `Invariant` variant.
#![allow(dead_code)]
use arbitrary::Arbitrary;
use soroban_audit_sample::{
    lending::{Lending, LendingClient},
    vault::{Vault, VaultClient},
};
use soroban_sdk::{
    contract, contractimpl, symbol_short, testutils::Address as _, token::StellarAssetClient,
    Address, Env,
};

const USERS: usize = 3;
const MAX_OPS: usize = 16;

/// What the untrusted receiver does when the vault calls it back.
/// Callbacks are adversarial, so the fuzzer decides their behaviour.
#[derive(Arbitrary, Debug, Clone, Copy)]
pub enum Callback {
    Nothing,
    Borrow(u16),
}

#[derive(Arbitrary, Debug, Clone, Copy)]
pub enum Op {
    Deposit {
        user: u8,
        amount: u16,
    },
    Withdraw {
        from: u8,
        to: u8,
        amount: u16,
    },
    WithdrawAndNotify {
        from: u8,
        amount: u16,
        callback: Callback,
    },
    Borrow {
        user: u8,
        amount: u16,
    },
}

#[derive(Arbitrary, Debug)]
pub struct Input {
    pub ops: Vec<Op>,
}

#[derive(Clone, Copy)]
pub enum Invariant {
    /// A user's vault balance only decreases in a call that user authorized.
    OwnerAuthorizesDebits,
    /// For every user: debt <= 50% of collateral.
    Solvency,
}

#[contract]
pub struct FuzzReceiver;

#[contractimpl]
impl FuzzReceiver {
    pub fn __constructor(env: Env, lending: Address) {
        env.storage()
            .instance()
            .set(&symbol_short!("lending"), &lending);
    }

    pub fn set_borrow(env: Env, amount: i128) {
        env.storage()
            .instance()
            .set(&symbol_short!("borrow"), &amount);
    }

    pub fn on_withdraw(env: Env, from: Address, _amount: i128) {
        let amount: i128 = env
            .storage()
            .instance()
            .get(&symbol_short!("borrow"))
            .unwrap_or(0);
        if amount > 0 {
            let lending: Address = env
                .storage()
                .instance()
                .get(&symbol_short!("lending"))
                .unwrap();
            let _ = LendingClient::new(&env, &lending).try_borrow(&from, &amount);
        }
    }
}

pub fn run(input: Input, invariant: Invariant) {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();
    let vault_id = Address::generate(&env);
    let lending_id = Address::generate(&env);
    env.register_at(&vault_id, Vault, (token.clone(), lending_id.clone()));
    env.register_at(&lending_id, Lending, (vault_id.clone(), token.clone()));
    let vault = VaultClient::new(&env, &vault_id);
    let lending = LendingClient::new(&env, &lending_id);
    let receiver = env.register(FuzzReceiver, (lending_id.clone(),));
    let receiver_client = FuzzReceiverClient::new(&env, &receiver);

    let sac = StellarAssetClient::new(&env, &token);
    sac.mint(&lending_id, &10_000_000);
    let users: [Address; USERS] = core::array::from_fn(|_| Address::generate(&env));
    for user in &users {
        sac.mint(user, &1_000_000);
    }
    let user = |i: u8| users[i as usize % USERS].clone();

    for op in input.ops.into_iter().take(MAX_OPS) {
        let before: [i128; USERS] = core::array::from_fn(|i| vault.balance(&users[i]));

        match op {
            Op::Deposit { user: u, amount } => {
                let _ = vault.try_deposit(&user(u), &amount.into());
            }
            Op::Withdraw { from, to, amount } => {
                let _ = vault.try_withdraw(&user(from), &user(to), &amount.into());
            }
            Op::WithdrawAndNotify {
                from,
                amount,
                callback,
            } => {
                let borrow = match callback {
                    Callback::Nothing => 0,
                    Callback::Borrow(b) => b.into(),
                };
                receiver_client.set_borrow(&borrow);
                let _ = vault.try_withdraw_and_notify(&user(from), &receiver, &amount.into());
            }
            Op::Borrow { user: u, amount } => {
                let _ = lending.try_borrow(&user(u), &amount.into());
            }
        }
        // Must be read before any other invocation: it only covers the last call.
        let auths = env.auths();

        for (i, u) in users.iter().enumerate() {
            match invariant {
                Invariant::OwnerAuthorizesDebits => {
                    if vault.balance(u) < before[i] {
                        assert!(
                            auths.iter().any(|(signer, _)| signer == u),
                            "user {i}'s balance decreased without their authorization after {op:?}"
                        );
                    }
                }
                Invariant::Solvency => {
                    assert!(
                        lending.debt(u) * 2 <= lending.collateral(u),
                        "user {i} is insolvent (debt {} > 50% of collateral {}) after {op:?}",
                        lending.debt(u),
                        lending.collateral(u)
                    );
                }
            }
        }
    }
}
