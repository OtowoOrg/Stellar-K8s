# Soroban Smart Contract Security Audit Framework

This framework is a standard way to review Soroban smart contracts before
deploying them. It covers **Soroban-specific** risks: the host's authorization
framework, cross-contract calls, storage types and TTL, resource limits, and
upgrades. General Rust code quality is out of scope. It applies here only where
Soroban changes the risk, for example with `i128` token amounts or the
`overflow-checks` profile setting of a WASM release build.

It has three parts that work together:

| Part | Purpose |
|---|---|
| This guide | Methodology, attack vectors, secure patterns, fuzzing guidelines, severity rubric |
| [`docs/templates/audit-checklist.md`](../templates/audit-checklist.md) | The checklist to copy and fill in for each audit. Item IDs (e.g. `AUTH-02`) are referenced throughout this guide. |
| [`examples/soroban-audit-sample/`](../../examples/soroban-audit-sample/) | A deliberately vulnerable contract pair, its completed checklist (`AUDIT.md`), exploit tests, and fuzz targets. It is the framework's validation (§8). |

Examples target `soroban-sdk` 28. Earlier SDKs use slightly different test-utility
names (for example `env.budget()` instead of `env.cost_estimate().budget()`).

---

## Contents

- [1. When to audit](#1-when-to-audit)
- [2. Audit process](#2-audit-process)
- [3. Soroban attack vectors](#3-soroban-attack-vectors)
  - [3.1 Authorization bypasses](#31-authorization-bypasses)
  - [3.2 Reentrancy and cross-contract calls](#32-reentrancy-and-cross-contract-calls)
  - [3.3 State, storage types, and TTL expiration](#33-state-storage-types-and-ttl-expiration)
  - [3.4 Resource exhaustion](#34-resource-exhaustion)
  - [3.5 Arithmetic and value handling](#35-arithmetic-and-value-handling)
  - [3.6 Upgrades, errors, and events](#36-upgrades-errors-and-events)
- [4. Secure coding patterns](#4-secure-coding-patterns)
- [5. Automated analysis and fuzzing](#5-automated-analysis-and-fuzzing)
- [6. Build and deployment verification](#6-build-and-deployment-verification)
- [7. Severity rubric](#7-severity-rubric)
- [8. Validation against a flawed sample](#8-validation-against-a-flawed-sample)
- [9. References](#9-references)

---

## 1. When to audit

An audit using this framework is **required** before:

- the first mainnet deployment of a contract,
- any `update_current_contract_wasm` upgrade on mainnet,
- enabling a new integration with an external contract (a new token, oracle,
  or receiver type).

A **delta audit** is enough for small changes: re-run the checklist sections
the diff touches, plus sections 1 (BLD) and 8 (AUTO) in full.

---

## 2. Audit process

| Phase | Output | Checklist |
|---|---|---|
| **1. Scope and freeze** | Commit hash, `Cargo.lock`, SDK version, audited WASM hash | §0, BLD-01…03 |
| **2. Trust model** | Roles and keys, external contracts (trusted or hostile), callback surfaces, **written protocol invariants** | §0 trust model |
| **3. Manual review** | One Pass/Fail/N/A with evidence for every item | §1–§7 |
| **4. Automated analysis** | Negative auth tests, one fuzz target per invariant, resource measurements, static analysis triage | §8 (AUTO) |
| **5. Report** | A finding for every Fail, rated with the §7 rubric, with a proof | §9 |
| **6. Remediation check** | Every exploit test flipped into a regression test; checklist re-run on the fix commit | AUTO-04 |
| **7. Deployment check** | Deployed code hash equals the audited hash | BLD-05, §10 |

Work through the manual review **one function at a time**. For each public
function, ask:

1. Who must authorize this, and is that exactly the address passed to `require_auth`?
2. Which contracts does it call, and could any of them be attacker-controlled?
3. Which storage entries does it read or write? What is their type and TTL,
   and what happens if an entry is missing or archived?
4. What is the worst-case size of every loop and every value it writes?

**Exit criteria:** no Critical or High finding is Open, every item is Pass or
justified N/A, and every fix has a regression test.

---

## 3. Soroban attack vectors

### 3.1 Authorization bypasses

Soroban has no `msg.sender`. A contract states *which address* must approve an
action by calling `address.require_auth()`. The host then checks that this
address signed an authorization tree covering the invocation, and handles
nonces and expiry for built-in accounts. When a contract calls another contract
directly, its own address is authorized automatically for that call. Most
Soroban authorization bugs come from asking the wrong address to approve, or
from not asking at all.

| Vector | What goes wrong | Checklist |
|---|---|---|
| **Missing auth** | A state-changing function has no `require_auth` at all. | AUTH-01 |
| **Auth on the wrong address** | The function requires auth from `to`, the caller, or another parameter, instead of the owner of the assets. Any attacker can then satisfy the check with their own signature. | AUTH-02 |
| **Caller-supplied admin** | `fn set_fee(env, admin: Address, fee: u32) { admin.require_auth(); … }`: the attacker passes *their own* address as `admin` and signs. The admin must be read from storage. | AUTH-03 |
| **Front-runnable initializer** | A public `init(admin)` that anyone can call first, or call again. Use `__constructor`, which runs atomically when the contract is deployed. | AUTH-04 |
| **Partial `require_auth_for_args`** | Custom auth args leave out the amount or recipient, so a signature approved for one action can be reused for a larger or redirected one. | AUTH-05 |
| **Confused deputy** | The contract acts as the authorizer (`current_contract_address()` as `from`) on a call whose target or arguments the caller controls. Examples: an arbitrary "execute" or "multicall" function, or a router that forwards caller-chosen token addresses. Anyone can then spend the contract's assets. | AUTH-06 |
| **Weak `__check_auth`** | A custom account that doesn't verify every signature against `signature_payload`, accepts duplicate signers, or ignores `auth_contexts` (so a signature for contract A can be replayed against contract B). | AUTH-07 |
| **Replayable custom signatures** | A contract verifies `ed25519_verify(pk, msg, sig)` over a message with no contract address, network ID (`env.ledger().network_id()`), nonce, or expiry. The signature can then be replayed across contracts, networks, or repeatedly. Nonces in *temporary* storage are also replayable once the entry expires (STATE-02). | AUTH-08 |

```rust
// ❌ AUTH-02: any recipient can pull anyone's funds (this is finding F-01 in the sample)
pub fn withdraw(env: Env, from: Address, to: Address, amount: i128) {
    to.require_auth();
    // ...
}

// ✅ the owner of the funds authorizes; the recipient is just a parameter
pub fn withdraw(env: Env, from: Address, to: Address, amount: i128) {
    from.require_auth();
    // ...
}

// ❌ AUTH-03: attacker passes their own address as `admin`
pub fn set_admin(env: Env, admin: Address, new_admin: Address) {
    admin.require_auth();
    env.storage().instance().set(&DataKey::Admin, &new_admin);
}

// ✅ the admin comes from storage
pub fn set_admin(env: Env, new_admin: Address) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
    env.storage().instance().set(&DataKey::PendingAdmin, &new_admin); // two-step (AUTH-09)
}
```

**How to test.** `mock_all_auths()` accepts *any* signer, so it can't catch
AUTH-02 or AUTH-03. For every privileged or value-moving function, write at
least one test with targeted `env.mock_auths(&[MockAuth { … }])` that signs as
the *attacker*, and assert that the call fails. Also assert `env.auths()` on
the success path to prove which address was actually required (AUTH-10). The
sample's `exploit_auth_bypass_attacker_withdraws_victim_deposit` shows this
pattern.

### 3.2 Reentrancy and cross-contract calls

**What the host guarantees.** Soroban **rejects re-entry into a contract that is
already on the call stack**. If A calls B and B tries to call A again, the call
fails with `Error(Context, InvalidAction)`. The sample's
`host_rejects_direct_reentry_into_the_same_contract` test demonstrates this. The
classic single-contract reentrancy drain is not possible, and a re-entry
"mutex" in storage adds nothing.

**What the host does *not* guarantee.** A protocol is often split across
several contracts (vault + lending market, pool + router, token + staking).
While contract A is paused mid-function waiting on an external call, an
attacker-controlled callee can freely call **contract B**. If B relies on state
that A has not yet updated, whether that is state mirrored into B, or state B
cached from A, or state A pushes to B after the call, the attacker acts on
stale data. This is **cross-contract reentrancy**, and the checklist targets it.

| Vector | What goes wrong | Checklist |
|---|---|---|
| **Cross-contract reentrancy** | Interactions come before effects. A receiver hook, a malicious token's `transfer`, or an oracle callback runs while sibling contracts still see the old state. | XCC-02 |
| **Untrusted token / receiver** | A caller-supplied `token: Address` is used as if it were the SAC. It can lie about `balance`, skip the transfer, or call back. | XCC-03 |
| **`try_*` partial failure** | `try_foo()` rolls back the *callee's* changes on error, but the caller keeps running with whatever it already wrote. The client returns `Result<Result<T, ConversionError>, Result<E, InvokeError>>`, and code that only checks `is_ok()` misses the inner conversion error. | XCC-04 |
| **Failure-induced DoS** | One recipient in a payout loop causes the whole transaction to revert: a panicking receiver, a classic account without a trustline for the asset, a revoked authorization, or a clawback-enabled asset. Every other user is then blocked. | XCC-05 |
| **Mutable dependencies** | A trusted dependency can upgrade its WASM, and its behaviour can change after your audit. | XCC-06 |
| **Unvalidated return values** | Prices, balances, or share amounts returned by another contract are used without bounds checks. | XCC-07 |

```rust
// ❌ XCC-02: interaction before effects (this is finding F-02 in the sample)
pub fn withdraw_and_notify(env: Env, from: Address, receiver: Address, amount: i128) {
    from.require_auth();
    let balance = read_balance(&env, &from);
    assert!(balance >= amount);
    token.transfer(&env.current_contract_address(), &receiver, &amount);
    WithdrawReceiverClient::new(&env, &receiver).on_withdraw(&from, &amount); // attacker code runs here…
    write_balance(&env, &from, balance - amount);                            // …before this
    lending.sync_collateral(&from, &(balance - amount));                     // …and this
}

// ✅ every effect, including state pushed to sibling contracts, happens before any untrusted call
pub fn withdraw_and_notify(env: Env, from: Address, receiver: Address, amount: i128) {
    from.require_auth();
    let balance = read_balance(&env, &from);
    assert!(balance >= amount);
    write_balance(&env, &from, balance - amount);
    lending.sync_collateral(&from, &(balance - amount));
    token.transfer(&env.current_contract_address(), &receiver, &amount);
    WithdrawReceiverClient::new(&env, &receiver).on_withdraw(&from, &amount);
}
```

**Measuring what an untrusted token actually transferred (XCC-03):**

```rust
let token = token::Client::new(&env, &token_addr);
let before = token.balance(&env.current_contract_address());
token.transfer(&from, &env.current_contract_address(), &amount);
let received = token.balance(&env.current_contract_address()) - before;
if received <= 0 { panic_with_error!(&env, Error::TransferFailed); }
// credit `received`, not `amount`
```

### 3.3 State, storage types, and TTL expiration

Every Soroban ledger entry has a time-to-live (TTL). What happens when the TTL
runs out depends on the storage type, and choosing the wrong type is a
security bug, not only a performance issue.

| Type | On TTL expiry | Suitable for | Never use for |
|---|---|---|---|
| `persistent()` | **Archived.** It must be restored (RPC simulation adds the restore, for a fee) before a transaction can use it. It is never silently read as "missing". | Balances, ownership, nonces, allowlists | Unbounded per-key blobs (STATE-06) |
| `temporary()` | **Deleted permanently.** `has()` returns `false` and `get()` returns `None`, and the key can be written again. | Oracle caches, short-lived quotes, data that is safe to lose | Nonces, "claimed" flags, locks, anything security-relevant (STATE-02) |
| `instance()` | Shares the contract instance's TTL. It is **loaded on every invocation**. | Small configuration: admin, token address, fee parameters | Per-user data or growing collections (STATE-07) |

| Vector | What goes wrong | Checklist |
|---|---|---|
| **Security state in temporary storage** | A `Claimed(user)` flag or signature nonce stored as temporary disappears at expiry. The claim or signed message can then be replayed. | STATE-01, STATE-02 |
| **Liveness loss through archival** | Balances, or the contract instance and WASM code, are never extended. They get archived, and every user transaction then needs a restore. Extend on use: `env.storage().persistent().extend_ttl(&key, threshold, extend_to)` and `env.storage().instance().extend_ttl(threshold, extend_to)`. The maximum is `env.storage().max_ttl()`. | STATE-03 |
| **Relying on expiry** | "This lock/offer times out when its entry expires." Any account can extend any entry's TTL with `ExtendFootprintTTLOp`. Store an explicit `expires_at` ledger or timestamp and check it. | STATE-04 |
| **Unsafe defaults** | `get(&key).unwrap_or(0)` on a temporary entry turns "expired" into "zero". Harmless for a cache, dangerous for a debt record. | STATE-05 |
| **Unbounded single entry** | `Vec<Address>` of all depositors under one key. Every write rewrites the whole entry, and once it passes the entry-size or write-bytes limit, every function that touches it fails permanently. | STATE-06 |
| **Instance bloat** | Per-user data in instance storage makes every call slower and more expensive for everyone, until calls exceed the limits. | STATE-07 |
| **Layout drift on upgrade** | `#[contracttype]` enum variants are stored by name, and structs by field. Renaming or re-typing one after an upgrade leaves existing entries unreadable. | STATE-08 |

```rust
// ✅ STATE-01/02/03: persistent, per-user key, TTL extended on write
fn mark_claimed(env: &Env, user: &Address) {
    let key = DataKey::Claimed(user.clone());
    env.storage().persistent().set(&key, &true);
    env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

// ✅ STATE-04: explicit deadline instead of relying on entry expiry
if env.ledger().sequence() > offer.expires_at_ledger {
    panic_with_error!(&env, Error::OfferExpired);
}
```

### 3.4 Resource exhaustion

Each transaction has network-enforced limits on CPU instructions, memory,
ledger entries read and written, bytes read and written, and event size. A
function whose cost grows with user-controlled data works in testing and then
**fails for everyone** once the data grows. On an immutable contract, that can
permanently lock funds.

| Vector | Checklist |
|---|---|
| Looping over all users, positions, or orders in one call | RES-01 |
| A footprint that grows with input (a batch function touching N entries) | RES-02 |
| No measurement of worst-case cost against network limits | RES-03 |
| Events or return values that grow with state | RES-04 |

Fix: paginate (`fn distribute(env, start: u32, limit: u32)` with a hard cap on
`limit`), use per-item storage keys, and prefer pull-based claims to push-based
distribution.

### 3.5 Arithmetic and value handling

| Vector | What goes wrong | Checklist |
|---|---|---|
| **Overflow in release WASM** | Rust's release profile wraps on overflow unless `overflow-checks = true`. Native tests (debug) panic, while the deployed WASM wraps silently. | BLD-01, MATH-01 |
| **Negative `i128` amounts** | Token amounts are signed. A `transfer(-100)` in custom logic can reverse the direction of value flow. The SAC rejects negatives, but your own bookkeeping may not. | MATH-02 |
| **Rounding / share inflation** | Rounding in the user's favour on payouts, or a first depositor inflating the share price by donating tokens straight to the vault. | MATH-03 |
| **Ledger time and PRNG** | `env.ledger().timestamp()` is proposed by validators and may drift. `env.prng()` is deterministic per transaction and, per the SDK, unsuitable for secrets or low-risk-tolerance uses such as lotteries. | MATH-04 |

```rust
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum Error {
    InvalidAmount = 1,
    Overflow = 2,
    InsufficientBalance = 3,
}

if amount <= 0 { panic_with_error!(&env, Error::InvalidAmount); }
let new_balance = balance
    .checked_add(amount)
    .unwrap_or_else(|| panic_with_error!(&env, Error::Overflow));
```

### 3.6 Upgrades, errors, and events

| Vector | Checklist |
|---|---|
| `update_current_contract_wasm` reachable without admin auth, or with a caller-supplied admin | UPG-01, AUTH-03 |
| Upgrade with no migration for changed storage layouts | UPG-02, STATE-08 |
| Bare `panic!` / `unwrap` on user-reachable paths: clients can't tell failure modes apart | ERR-01 |
| Value or permission changes without events: off-chain monitoring is blind | EVT-01 |
| Custom token deviating from SEP-41, or SAC issuer powers (clawback, revocation) not modelled | TOK-01 |

---

## 4. Secure coding patterns

The Soroban-specific patterns reviewers should expect to see. A deviation is
not automatically a finding, but it needs a justification.

| # | Pattern | Prevents |
|---|---|---|
| P1 | `require_auth()` on the affected owner is the **first statement** of every state-changing function. The admin always comes from storage. | AUTH-01…03 |
| P2 | `__constructor` for initialization. No public `init`. | AUTH-04 |
| P3 | Checks, then effects (**including calls that sync sibling contracts**), then interactions with untrusted contracts. | XCC-02 |
| P4 | Allowlist external contract addresses in instance storage. Measure untrusted token transfers by balance delta. | XCC-03 |
| P5 | Pull over push: users claim their own payouts. No loops over external calls. | XCC-05, RES-01 |
| P6 | Typed `DataKey` enum with per-entity keys (`Balance(Address)`), never growing collections under one key. | STATE-06, RES-02 |
| P7 | One helper per storage type that writes **and** extends TTL, so extension can't be forgotten. Also extend the instance TTL in hot paths. | STATE-03 |
| P8 | Security state in persistent storage only. Explicit deadlines instead of TTL-based expiry. | STATE-02, STATE-04 |
| P9 | `overflow-checks = true`, plus `checked_*` with `panic_with_error!` on value paths. Reject `amount <= 0` at the entry point. | MATH-01, MATH-02 |
| P10 | Bounded loops: take `limit` as a parameter, cap it with a constant, and paginate. | RES-01 |
| P11 | `#[contracterror]` for every failure mode, and an event for every value or permission change. | ERR-01, EVT-01 |
| P12 | Upgrade function gated by stored admin auth, emitting an event, with a stored schema version checked by the migration. | UPG-01, UPG-02 |

---

## 5. Automated analysis and fuzzing

### 5.1 Authorization tests (AUTH-10, AUTO-01)

```rust
use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};

env.mock_auths(&[MockAuth {
    address: &attacker, // sign as the attacker…
    invoke: &MockAuthInvoke {
        contract: &vault.address,
        fn_name: "withdraw",
        args: (&victim, &attacker, 1_000_i128).into_val(&env),
        sub_invokes: &[],
    },
}]);
// …and require that stealing the victim's funds FAILS
assert!(vault.try_withdraw(&victim, &attacker, &1_000).is_err());
```

### 5.2 Fuzzing with `cargo-fuzz` (AUTO-02)

Fuzzing works best on **protocol invariants**, not on single functions. Write
the invariants down in the trust model, then write one fuzz target per
invariant that:

1. deploys the full protocol (every contract, plus the SAC for tokens) in a
   fresh `Env`,
2. replays a **fuzzer-chosen sequence** of operations by several users,
3. lets the fuzzer choose what **untrusted callbacks** do (receiver hooks,
   malicious tokens). This is how cross-contract reentrancy gets found,
4. checks the invariant after **every** operation.

Setup (requires a nightly toolchain; the fuzz crate depends on the contract
crate, so the contract crate must include `rlib` in `crate-type`):

```bash
cargo install cargo-fuzz --locked
cargo fuzz init                       # creates ./fuzz; give it its own [workspace] table if needed
```

`fuzz/Cargo.toml` dependencies:

```toml
[dependencies]
arbitrary = { version = "1", features = ["derive"] }
libfuzzer-sys = "0.4"
soroban-sdk = { version = "28", features = ["testutils"] }
my-contract = { path = ".." }
```

Harness skeleton (the complete, working version is
[`examples/soroban-audit-sample/fuzz/fuzz_targets/common.rs`](../../examples/soroban-audit-sample/fuzz/fuzz_targets/common.rs)):

```rust
#[derive(Arbitrary, Debug)]
enum Op {
    Deposit { user: u8, amount: u16 },
    Withdraw { from: u8, to: u8, amount: u16 },
    WithdrawAndNotify { from: u8, amount: u16, callback: Callback }, // callback = fuzzer-controlled adversary
    Borrow { user: u8, amount: u16 },
}

fuzz_target!(|input: Input| {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited(); // avoid false crashes from long sequences
    // ... deploy protocol, mint to N users ...
    for op in input.ops.into_iter().take(MAX_OPS) {
        let before = snapshot(&env);
        let _ = apply(&env, op);       // use try_* clients: expected rejections are not bugs
        let auths = env.auths();       // read immediately: covers only the last invocation
        assert_invariants(&env, &before, &auths, op);
    }
});
```

Two invariant styles that together catch the vector classes in §3.1 and §3.2:

- **Authorization invariant:** "if a user's balance went down, that user
  appears in `env.auths()` for that call." With `mock_all_auths`, this turns
  any authorization mistake (AUTH-01/02/03) into a crash, without writing a
  targeted test for each one.
- **Economic invariant:** for example "debt ≤ 50% × collateral for every
  user", or "sum of internal balances == token balance held". Violations
  reached through fuzzer-controlled callbacks point to cross-contract
  reentrancy (XCC-02).

Running:

```bash
cargo +nightly fuzz run solvency_invariant -- -max_total_time=600
# on macOS, if libFuzzer aborts at startup under AddressSanitizer:
cargo +nightly fuzz run -s none solvency_invariant -- -max_total_time=600
# replay / minimise a crash
cargo +nightly fuzz run solvency_invariant fuzz/artifacts/solvency_invariant/crash-<hash>
cargo +nightly fuzz tmin solvency_invariant fuzz/artifacts/solvency_invariant/crash-<hash>
```

Guidelines:

- Choose the time budget per invariant before the audit starts. At least 1
  CPU-hour per target is a reasonable floor for a pre-mainnet audit. Keep the
  `corpus/` directory and run it again in CI for regression.
- Use small integer types (`u8` user index, `u16` amounts) so the fuzzer
  explores meaningful states instead of rejected ones.
- Turn every crash into a unit test (AUTO-04) before fixing it.
- A fuzz run that finds nothing only means nothing was found within that
  budget. It does not replace the manual review.

### 5.3 Resource measurement (RES-03)

Run the worst case (a maximum-size page, a fully populated user) and record
its cost:

```rust
contract.process_page(&0, &MAX_PAGE);
let res = env.cost_estimate().resources();
assert!(res.instructions < WORST_CASE_INSTRUCTION_BUDGET);
assert!(res.write_entries <= MAX_WRITE_ENTRIES);
```

Native (Rust-registered) contracts leave out VM instantiation and WASM costs.
For final numbers, register the built WASM
(`soroban_sdk::contractimport!(file = "...wasm")` and `env.register(WASM, ())`)
or use RPC `simulateTransaction` against a network running the target protocol.

### 5.4 Static analysis (AUTO-03)

Run a Soroban-aware static analyzer, such as CoinFabrik's
[Scout for Soroban](https://github.com/CoinFabrik/scout-soroban), over the
contract crate. Triage every result into a finding or a documented false
positive. Static analysis complements manual review but can't replace it:
AUTH-02 (auth on the wrong address) and cross-contract XCC-02 require
understanding what the code is meant to do.

---

## 6. Build and deployment verification

### 6.1 Build integrity (BLD-01…05)

```toml
# every contract crate
[profile.release]
opt-level = "z"
overflow-checks = true
debug = 0
strip = "symbols"
debug-assertions = false
panic = "abort"
codegen-units = 1
lto = true
```

```bash
stellar contract build                       # pinned toolchain, clean checkout of the audited commit
sha256sum target/wasm32v1-none/release/*.wasm  # record in checklist §0

# after deployment: the on-chain code must match the audited hash
stellar contract fetch --id <CONTRACT_ID> --network mainnet --out-file deployed.wasm
sha256sum deployed.wasm
```

If the hashes differ, the audit does not cover the deployed contract.

---

## 7. Severity rubric

| Severity | Definition | Soroban examples |
|---|---|---|
| **Critical** | Direct loss or theft of funds, or permanent lockup, triggerable by any unprivileged account | AUTH-02 wrong-address auth on withdraw; AUTH-03 caller-supplied admin on upgrade; XCC-02 reentrancy creating uncollateralized debt |
| **High** | Loss of funds under realistic preconditions, permanent DoS, or privilege escalation | STATE-02 replayable claim via temporary storage; RES-01 unbounded loop that bricks withdrawals; AUTH-06 confused deputy needing a specific token |
| **Medium** | Temporary DoS, value leakage from rounding, or an invariant broken without direct profit | STATE-03 instance archival requiring restores; XCC-05 payout blocked by one frozen trustline; MATH-03 rounding in the user's favour |
| **Low** | Defence in depth; unlikely to be exploitable alone | Missing events (EVT-01); bare `panic!` instead of `contracterror` (ERR-01) |
| **Informational** | Code quality, documentation, gas/fee optimizations | Oversized instance storage below the limits |

Adjust by **one level** at most for exploitability (privileged precondition,
tiny profit) and write down the reason in the finding.

---

## 8. Validation against a flawed sample

As required for this framework, the checklist was applied to a deliberately
vulnerable contract pair in
[`examples/soroban-audit-sample/`](../../examples/soroban-audit-sample/):

- **`Vault`** holds token deposits and mirrors each balance into `Lending` as
  collateral.
- **`Lending`** lends against that mirrored collateral at a 50% loan-to-value
  ratio.

Two bugs were injected:

| Injected bug | Location | Caught by checklist item | Manual proof (unit test) | Automated proof (fuzz target) |
|---|---|---|---|---|
| **Authorization bypass:** `withdraw` requires auth from `to` instead of `from` | `src/vault.rs:55` | **AUTH-02** (also AUTH-10, AUTO-01) → finding **F-01, Critical** | `exploit_auth_bypass_attacker_withdraws_victim_deposit`: an attacker-only signature drains the victim's deposit | `auth_invariant`: crashed after ~6.4k executions with `user 2's balance decreased without their authorization after Withdraw {…}` |
| **Cross-contract reentrancy:** `withdraw_and_notify` transfers and calls the receiver hook before updating its balance and syncing collateral | `src/vault.rs:79-91` | **XCC-02** (also XCC-01, XCC-03) → finding **F-02, Critical** | `exploit_cross_contract_reentrancy_leaves_uncollateralized_debt`: the attacker deposits 1,000, withdraws 1,000, and borrows 500 against collateral that no longer exists | `solvency_invariant`: crashed after ~11.7k executions with `user 2 is insolvent (debt 23942 > 50% of collateral 30827) after WithdrawAndNotify {…, callback: Borrow(23942)}` |

The sample also confirms the host behaviour this framework relies on:
`host_rejects_direct_reentry_into_the_same_contract` shows that a receiver
re-entering the *vault itself* is rejected with `Error(Context,
InvalidAction)` and the whole transaction is rolled back. That is why XCC-02 is
written in terms of *sibling* contracts.

The completed checklist with both findings is in
[`examples/soroban-audit-sample/AUDIT.md`](../../examples/soroban-audit-sample/AUDIT.md).
To reproduce:

```bash
cd examples/soroban-audit-sample
cargo test                                                         # 5 tests: exploits + controls
cd fuzz
cargo +nightly fuzz run auth_invariant     -- -max_total_time=300  # add `-s none` on macOS
cargo +nightly fuzz run solvency_invariant -- -max_total_time=300
```

Results were recorded with `soroban-sdk` 28.0.0 and `cargo-fuzz` 0.13.2. Fuzz
execution counts vary from run to run.

---

## 9. References

- [Soroban authorization](https://developers.stellar.org/docs/learn/fundamentals/contract-development/authorization)
- [State archival and TTL](https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/state-archival)
- [Persisting data: storage types](https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/persisting-data)
- [Resource limits and fees](https://developers.stellar.org/docs/networks/resource-limits-fees)
- [SEP-41 token interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [`soroban-sdk` testutils: `arbitrary` and `cargo_fuzz`](https://docs.rs/soroban-sdk/latest/soroban_sdk/testutils/arbitrary/index.html)
- [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
- [Scout for Soroban](https://github.com/CoinFabrik/scout-soroban)
