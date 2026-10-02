# Audit: soroban-audit-sample (Vault + Lending)

> This is the checklist in [`docs/templates/audit-checklist.md`](../../docs/templates/audit-checklist.md)
> applied to a **deliberately vulnerable** contract pair, as the validation exercise
> for [`docs/security/smart-contract-audit.md`](../../docs/security/smart-contract-audit.md) (§8).
> Two bugs were injected: an authorization bypass and a cross-contract reentrancy.
> The checklist caught both (F-01, F-02). It also surfaced three issues that were
> **not** deliberately injected (F-03 to F-05).

## 0. Audit metadata

| Field | Value |
|---|---|
| Project / contract(s) | `Vault` (`src/vault.rs`), `Lending` (`src/lending.rs`) |
| Repository and commit audited | This directory, on the branch that introduced it |
| `soroban-sdk` version (from `Cargo.lock`) | 28.0.0 |
| Target protocol version | Whichever version `soroban-sdk` 28 targets |
| Audited WASM SHA-256 (per contract) | N/A: the crate is `rlib`-only on purpose, so it can't be built into deployable WASM |
| Build command (reproducible) | `cargo test` (native only) |
| Auditor(s) | Framework validation |
| In-scope files | `src/vault.rs`, `src/lending.rs` |
| Out-of-scope / trusted dependencies | Stellar Asset Contract (SAC) for the token |

### Trust model

| Question | Answer |
|---|---|
| Privileged roles | None. No admin. `Lending::sync_collateral` may be called only by the vault's contract address. |
| External contracts | **Trusted:** the SAC token (fixed at construction) and the sibling `Lending`/`Vault` (fixed at construction). **Attacker-controllable:** the `receiver` passed to `Vault::withdraw_and_notify`. |
| Callback surfaces | `Vault::withdraw_and_notify` → `receiver.on_withdraw(from, amount)` |
| Protocol invariants | **I1:** a user's vault balance decreases only in a call that user authorized. **I2:** for every user, `Lending.debt ≤ 50% × Lending.collateral`, where collateral must equal `Vault.balance`. |
| Upgradeable? | No. There is no upgrade entry point. |

---

## 1. Build and deployment integrity

| ID | Result | Evidence / notes |
|---|---|---|
| BLD-01 | Pass | `Cargo.toml` `[profile.release] overflow-checks = true` |
| BLD-02 | Pass | `soroban-sdk = "28"`, `Cargo.lock` committed (28.0.0) |
| BLD-03 | N/A | Not deployable by design (`crate-type = ["rlib"]`) |
| BLD-04 | N/A | Same reason; tests run natively only |
| BLD-05 | N/A | Never deployed |

## 2. Authorization

| ID | Result | Evidence / notes |
|---|---|---|
| AUTH-01 | Pass | Every state-changing function calls `require_auth` on *some* address: `vault.rs:34, 55, 73`, `lending.rs:28, 34`. (Whether it is the *right* address is AUTH-02.) |
| AUTH-02 | **Fail** | `vault.rs:55`: `withdraw` requires `to.require_auth()`, but the assets belong to `from`. → **F-01** |
| AUTH-03 | N/A | No privileged functions. `sync_collateral` checks the vault address loaded from storage (`lending.rs:27-28`), which is correct. |
| AUTH-04 | Pass | Both contracts initialize in `__constructor` |
| AUTH-05 | N/A | `require_auth_for_args` not used |
| AUTH-06 | Pass | The vault authorizes as itself only for `token.transfer` on the stored SAC, with an amount bounded by the caller's own balance. Invoker auth for the call to `receiver` doesn't carry over to calls the receiver makes. |
| AUTH-07 | N/A | No custom account |
| AUTH-08 | N/A | No custom signature schemes |
| AUTH-09 | N/A | No admin role |
| AUTH-10 | **Fail** | No negative authorization tests were shipped. A targeted `mock_auths` test added during the audit proves F-01. → **F-01** |

## 3. Cross-contract calls and reentrancy

| ID | Result | Evidence / notes |
|---|---|---|
| XCC-01 | Pass | External calls: `token.transfer` (trusted SAC) in all three vault value functions and in `borrow`; `lending.sync_collateral` (trusted sibling); `receiver.on_withdraw` (**hostile**). Direct re-entry into `Vault` is blocked by the host, as confirmed by `host_rejects_direct_reentry_into_the_same_contract`. `Lending` is **not** on the stack during the callback, so it can be reached. |
| XCC-02 | **Fail** | `vault.rs:79-91`: `withdraw_and_notify` transfers to the receiver and calls `on_withdraw` **before** writing the new balance and calling `sync_collateral`. During the callback, `Lending` still holds the pre-withdrawal collateral. → **F-02** |
| XCC-03 | **Fail** | The caller-supplied `receiver` is attacker-controllable and runs with protocol state out of sync. (The token itself is fixed at construction, so it is fine.) → **F-02** |
| XCC-04 | N/A | No `try_*` calls in contract code |
| XCC-05 | Pass | No loops over external calls. A failing receiver can only revert its own caller's withdrawal. |
| XCC-06 | N/A | Only the SAC and the fixed sibling contract are called |
| XCC-07 | N/A | No cross-contract return values are used |

## 4. State, storage types, and TTL

| ID | Result | Evidence / notes |
|---|---|---|
| STATE-01 | Pass | Balances, collateral and debt are persistent. Token and sibling addresses are in instance storage. |
| STATE-02 | Pass | No temporary storage |
| STATE-03 | **Fail** | Persistent entries are extended on write (`vault.rs` `write_balance`, `lending.rs` `write`), but **instance TTL is never extended** in either contract. → **F-03** |
| STATE-04 | Pass | Nothing depends on expiry |
| STATE-05 | Pass | `unwrap_or(0)` applies only to persistent entries, and there "absent" genuinely means zero |
| STATE-06 | Pass | Per-user keys (`Balance(Address)`, `Collateral(Address)`, `Debt(Address)`) |
| STATE-07 | Pass | Instance storage holds two addresses per contract |
| STATE-08 | N/A | Not upgradeable |

## 5. Resource limits

| ID | Result | Evidence / notes |
|---|---|---|
| RES-01 | Pass | No loops |
| RES-02 | Pass | Fixed footprint: at most about 6 entries per call |
| RES-03 | **Fail** | No resource measurements. → **F-05** |
| RES-04 | N/A | No events; return values are scalars |

## 6. Arithmetic and value handling

| ID | Result | Evidence / notes |
|---|---|---|
| MATH-01 | **Fail** | Uses `checked_add().expect(..)` and `assert!` rather than `panic_with_error!` with a `#[contracterror]`. Not exploitable, because BLD-01 holds. → **F-04** |
| MATH-02 | Pass | `amount > 0` is asserted in `deposit`, `withdraw`, `withdraw_and_notify` and `borrow` |
| MATH-03 | Pass | `collateral / 2` rounds down, in the protocol's favour |
| MATH-04 | N/A | No time or PRNG use |

## 7. Upgrades, errors, and events

| ID | Result | Evidence / notes |
|---|---|---|
| UPG-01 | N/A | Immutable by design |
| UPG-02 | N/A | Immutable by design |
| ERR-01 | **Fail** | No `#[contracterror]`. Failures are string panics. → **F-04** |
| EVT-01 | **Fail** | No events for deposits, withdrawals, collateral sync, or borrows. → **F-04** |
| TOK-01 | Pass | Uses the SAC, fixed at construction |

## 8. Automated analysis

| ID | Result | Evidence / notes |
|---|---|---|
| AUTO-01 | Pass | `src/test.rs`, added during the audit: 2 exploit tests, 2 control tests, 1 host-behaviour test |
| AUTO-02 | **Fail** | Both invariant targets crashed. `fuzz/auth_invariant` (I1) crashed after ~6.4k executions on `Withdraw { from ≠ to }`. `fuzz/solvency_invariant` (I2) crashed after ~11.7k executions on `WithdrawAndNotify { callback: Borrow(..) }`. → **F-01**, **F-02** |
| AUTO-03 | N/A | Static analysis wasn't part of this validation run |
| AUTO-04 | N/A | Fixes are deliberately not applied (see Remediation in each finding) |

---

## 9. Findings

### F-01: Any account can withdraw any depositor's funds

| | |
|---|---|
| Severity | **Critical** |
| Checklist item(s) | AUTH-02, AUTH-10 |
| Location | `src/vault.rs:55` |
| Status | Open (intentional) |

**Description.** `Vault::withdraw(from, to, amount)` calls `to.require_auth()`.
The only signature it demands comes from the recipient, who chooses to be the
recipient. The owner of the debited balance (`from`) is never asked.

**Impact.** Any account can drain every depositor's balance to itself in one
transaction per victim. No preconditions.

**Proof.** `test::exploit_auth_bypass_attacker_withdraws_victim_deposit` signs
*only* as the attacker (targeted `mock_auths`), withdraws the victim's full
deposit, and asserts that `env.auths()` contains only the attacker. Fuzz target
`auth_invariant` finds the same bug without being told where to look.

**Remediation.**

```diff
     pub fn withdraw(env: Env, from: Address, to: Address, amount: i128) {
-        to.require_auth();
+        from.require_auth();
```

Then flip the exploit test to `assert!(s.vault.try_withdraw(..).is_err())`.

### F-02: Cross-contract reentrancy in `withdraw_and_notify` creates uncollateralized debt

| | |
|---|---|
| Severity | **Critical** |
| Checklist item(s) | XCC-02, XCC-03 |
| Location | `src/vault.rs:79-91` |
| Status | Open (intentional) |

**Description.** `withdraw_and_notify` transfers tokens to a caller-supplied
`receiver` and calls `receiver.on_withdraw` before it writes the reduced
balance and calls `Lending::sync_collateral`. The host blocks re-entry into
`Vault`, but `Lending` is not on the call stack. The receiver can call
`Lending::borrow`, which still sees the full pre-withdrawal collateral.

**Impact.** An attacker deposits *C*, calls `withdraw_and_notify(C)`, and has
the receiver borrow *C/2* during the callback. They leave with *1.5 × C*, and
`Lending` is left holding *C/2* of debt against zero collateral. Repeating this
drains the lending pool. The only precondition is deploying a receiver
contract.

**Proof.** `test::exploit_cross_contract_reentrancy_leaves_uncollateralized_debt`:
it deposits 1,000 and ends with 1,500 extracted, collateral 0, and debt 500. Fuzz
target `solvency_invariant`, with a fuzzer-controlled callback, finds it without
guidance.

**Remediation.** Apply checks-effects-interactions across both contracts:

```diff
         let balance = Self::balance(env.clone(), from.clone());
         assert!(balance >= amount, "insufficient balance");
+        let new_balance = balance - amount;
+        Self::write_balance(&env, &from, new_balance);
+        Self::lending(&env).sync_collateral(&from, &new_balance);

         token::Client::new(&env, &Self::token(&env)).transfer(..);
         WithdrawReceiverClient::new(&env, &receiver).on_withdraw(&from, &amount);
-
-        let new_balance = balance - amount;
-        Self::write_balance(&env, &from, new_balance);
-        Self::lending(&env).sync_collateral(&from, &new_balance);
```

Also consider making the vault refuse withdrawals that would leave
`debt > 50% × collateral` (it would have to query `Lending` *before* the
interaction). That enforces the invariant at the source.

### F-03: Contract instance TTL is never extended

| | |
|---|---|
| Severity | Medium |
| Checklist item(s) | STATE-03 |
| Location | `src/vault.rs`, `src/lending.rs` (all entry points) |
| Status | Open (not injected, found by the checklist) |

**Description / impact.** Neither contract calls
`env.storage().instance().extend_ttl(..)`. Once the instance TTL lapses, the
instance and its stored configuration are archived, and every user transaction
needs a restore first. This costs liveness and fees but loses no data.

**Remediation.** At the top of each public entry point:
`env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);`

### F-04: No typed errors or events

| | |
|---|---|
| Severity | Low |
| Checklist item(s) | MATH-01, ERR-01, EVT-01 |
| Location | All value paths |
| Status | Open (not injected) |

**Remediation.** Add a `#[contracterror]` enum (`InvalidAmount`,
`InsufficientBalance`, `InsufficientCollateral`, `Overflow`) used through
`panic_with_error!`, and emit `#[contractevent]` events for deposit, withdraw,
collateral sync and borrow.

### F-05: Worst-case resource usage not measured

| | |
|---|---|
| Severity | Informational |
| Checklist item(s) | RES-03 |
| Status | Open (not injected) |

**Remediation.** Assert `env.cost_estimate().resources()` for `withdraw_and_notify`
(the deepest call chain) against the built WASM before any real deployment.

---

## 10. Sign-off

| Exit criterion | Met? |
|---|---|
| Every checklist item is Pass or N/A (with reason). | **No** (AUTH-02, AUTH-10, XCC-02, XCC-03, STATE-03, RES-03, MATH-01, ERR-01, EVT-01, AUTO-02) |
| No Critical or High finding is Open. | **No** (F-01, F-02) |
| Every fixed finding has a regression test. | N/A |
| The audited WASM hash matches the release build. | N/A |

**Verdict: NOT approved for deployment.** This is the expected result for the
validation sample.
