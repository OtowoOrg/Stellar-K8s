# Stellar Escrow & Collateral Vault

A **non-custodial escrow and collateral vault** for validator node operators who
stake assets as performance guarantees for automated node provisioning and
cross-chain relaying. The vault holds collateral during a lockup and
**automatically slashes** it if the node violates its uptime SLA — but only
when an authenticated fault proof is presented.

This crate is the smart-contract module for the Stellar-K8s project. It is
written against the exact module layout from the spec
(`contracts/escrow-vault/src/lib.rs` + `slashing.rs`) and is **Soroban /
Stellar-aligned**: `i128` amounts, a Soroban token-like transfer primitive, and
collateral that behaves like a Stellar *claimable balance* (a claim the
claimant pulls, never money the vault pushes to strangers).

> **Note on layout.** The parent operator workspace pins its own toolchain and
> profiles, so this crate declares itself a standalone workspace root. Build
> from the crate directory (see below).

---

## Features

| Requirement | Status |
| --- | --- |
| Deposit + lockup positions | ✅ `Vault::deposit` |
| Dispute resolution | ✅ `Vault::open_dispute` / `Vault::resolve_dispute` |
| Automated release on lockup expiry | ✅ `Vault::release_collateral` (pull) |
| Slashing via cryptographically signed proofs (downtime **and** double-sign) | ✅ `submit_downtime_proof` / `submit_double_sign_proof` |
| Proportional yield distribution **or** return of collateral | ✅ `distribute_yield` + `claim_yield` / `release_collateral` |
| Pull-over-push transfers | ✅ every payout is a *pull* by the recipient |
| No permanently stuck funds | ✅ `wind_down_leaves_no_funds_stuck` |
| Total-asset solvency under all state paths | ✅ property tests + formal notes |

---

## Contract surface (mirrors a Soroban `#[contractimpl]`)

Open `src/lib.rs` for the full docs. The high-level API:

- `initialize(config)` – set up admin, notifier, lockup window, slash basis
  points and the reporter allowlist.
- `add_reporter(admin, reporter, vk)` – register an authorized watcher reporter.
- `deposit(operator, node_id, node_vk, amount, now)` – create a lockup position.
- `submit_downtime_proof(proof, now)` / `submit_double_sign_proof(proof, …)` –
  **the only paths that slash**. Both verify signatures and freshness first; a
  rejected proof mutates nothing.
- `open_dispute` / `resolve_dispute` – freeze / adjudicate a position.
- `release_collateral(operator, id, now)` – operator *pulls* 100% of remaining
  (non-slashed) collateral after `lockup_until`.
- `claim_slashed(notifier, id)` – notifier *pulls* slashed collateral.
- `credit_yield` / `distribute_yield` / `claim_yield` – proportional yield.
- `assert_solvent()` – total-solvency invariant (`token_balance == liabilities`).

---

## Pull-over-push

The vault never calls an unknown recipient's `receive` hook in a first step.
Funds only move inside a claim the recipient initiates:

- `release_collateral`, `claim_slashed`, `claim_yield`, `reclaim_yield_pool`
  are all **pull** operations called by the intended recipient or the admin.
- Transfers go only to well-known, validated addresses (the operator, the
  notifier, the keeper). This is what guarantees a malformed recipient contract
  cannot lock up execution or strand vault funds.

---

## Build & test

```bash
cd contracts/escrow-vault

# The gates that CI (clippy -D warnings / rustfmt) runs:
cargo build                          # compiles
cargo fmt --all -- --check           # zero diffs
cargo clippy --all-targets -- -D warnings   # zero warnings
cargo test                           # unit + integration + property tests
```

Toolchain: any stable `Rust 1.88+` (verified on `1.92`).

---

## Tests

| Test | What it proves |
| --- | --- |
| `unit_tests::deposit_release_roundtrip` | deposit → lockup → 100% release |
| `non_faulty_operator_withdraws_100_percent_after_lockup_expiry` | non-faulty operator pulls **100%** after expiry; vault holds 0 |
| `valid_downtime_proof_slashes_but_a_forgery_never_mutates` | forged / wrong-target proofs never slash |
| `partial_slash_carves_collateral_and_operator_releases_the_rest` | 50% slash → notifier claims half, operator releases half |
| `double_sign_proof_is_self_incriminating_and_slashes` | node's own key signs two conflicting messages ⇒ slash |
| `dispute_freezes_release_and_admin_adjudicates` | dispute freezes release; dismissed ⇒ full recovery |
| `total_solvency_under_random_ops` (**proptest**) | solvency invariant holds after *every* op in 10k+ random adversarial sequences |
| `wind_down_leaves_no_funds_stuck` | after every liability is pulled, `token_balance == 0` |

Formal-verification / solvency notes: [`docs/solvency-verification.md`](docs/solvency-verification.md).