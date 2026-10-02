# Soroban Contract Audit Checklist

<!--
  How to use this template
  1. Copy this file next to the contract under review (for example AUDIT.md) and fill in §0.
  2. Work through every item. Set Result to one of: Pass | Fail | N/A.
     Every Fail must link to a finding in §9. Every N/A needs a one-line reason.
  3. Evidence must be verifiable: a file:line, a test name, a fuzz target, or a command.
  4. Don't ship to mainnet while any Critical or High finding is Open (see the exit criteria in §10).
  Methodology, attack-vector background and secure patterns:
  docs/security/smart-contract-audit.md (sections referenced as "Guide §x").
-->

## 0. Audit metadata

| Field | Value |
|---|---|
| Project / contract(s) | |
| Repository and commit audited | |
| `soroban-sdk` version (from `Cargo.lock`) | |
| Target protocol version | |
| Audited WASM SHA-256 (per contract) | |
| Build command (reproducible) | |
| Auditor(s) | |
| Review dates | |
| In-scope files | |
| Out-of-scope / trusted dependencies | |

### Trust model (fill before reviewing)

| Question | Answer |
|---|---|
| Privileged roles (admin, operator, oracle…), and who holds each key? | |
| Which external contracts are called? Which are trusted and which are attacker-controllable? | |
| Which functions call back into caller-supplied contracts (receivers, hooks, tokens)? | |
| Protocol invariants (for example "sum of balances == token balance", "debt ≤ LTV × collateral") | |
| Is the contract upgradeable? Who can upgrade it? | |

---

## 1. Build and deployment integrity (Guide §6.1)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| BLD-01 | `[profile.release]` sets `overflow-checks = true` for every contract crate. | | |
| BLD-02 | `soroban-sdk` is pinned and `Cargo.lock` is committed. The SDK version supports the target protocol. | | |
| BLD-03 | The WASM is built reproducibly (pinned toolchain; `stellar contract build`, then `stellar contract optimize` if used), and the audited SHA-256 is recorded in §0. | | |
| BLD-04 | Tests also run against the **built WASM** (registered from bytes), not only the native Rust contract. Native test builds can differ from release WASM (for example overflow behaviour). | | |
| BLD-05 | Deployment plan: the deployed code hash is verified against §0 (`stellar contract fetch` + `sha256sum`) and the correct network passphrase is used. | | |

## 2. Authorization (Guide §3.1)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| AUTH-01 | Every function that moves value, changes ownership, or changes permissions calls `require_auth()` / `require_auth_for_args()`. No state-changing function is missing it. | | |
| AUTH-02 | Auth is required from the **address whose assets or rights are affected** (for example `from`, `owner`), not from the recipient, the caller, or some other parameter. | | |
| AUTH-03 | Privileged functions load the admin/role address **from storage** and require its auth. They never trust an `admin: Address` argument supplied by the caller. | | |
| AUTH-04 | Initialization happens in `__constructor`, or an `init` function that can't be front-run and can't be called a second time. | | |
| AUTH-05 | `require_auth_for_args` (if used) covers every security-relevant argument (amount, recipient, limits, deadline). | | |
| AUTH-06 | **Confused deputy:** no function lets a caller make this contract call an arbitrary contract/function/args while the contract itself is the authorizer (`current_contract_address()` as `from`). The same applies to `authorize_as_current_contract`. | | |
| AUTH-07 | Custom account `__check_auth` (if any) verifies each signature against `signature_payload`, rejects duplicate or unknown signers, and restricts `auth_contexts` to the intended contracts and functions. There is no path that returns `Ok` unconditionally. | | |
| AUTH-08 | Custom signature schemes (`ed25519_verify`, `secp256k1`/`secp256r1` recovery) bind the contract address, network ID, a nonce, and an expiry. Nonces are kept in **persistent** storage. | | |
| AUTH-09 | Admin handover is two-step (propose, then accept). Admin keys are multisig / custom accounts, and all role changes emit events. | | |
| AUTH-10 | Tests use targeted `mock_auths` (not only `mock_all_auths`) and assert `env.auths()` for every privileged and value-moving function, including negative tests where the wrong signer is rejected. | | |

## 3. Cross-contract calls and reentrancy (Guide §3.2)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| XCC-01 | Every external call is listed, and each callee is marked trusted or attacker-controllable. The reviewer understands that the host forbids **direct** re-entry. The remaining risk comes from *other* contracts. | | |
| XCC-02 | **Checks-Effects-Interactions across the protocol:** all state, including state mirrored into or read by sibling contracts, is final **before** any call to an attacker-controllable contract (token, receiver hook, oracle, callback). | | |
| XCC-03 | Caller-supplied contract addresses (tokens, pools, receivers) are validated against an allowlist, or treated as hostile. For untrusted tokens, amounts are measured by **balance delta**, not by the requested amount or the callee's return value. | | |
| XCC-04 | `try_*` calls handle both error layers (`Err(Ok(contract_error))` and `Err(Err(InvokeError))`) and the `Ok(Err(ConversionError))` case. The caller's own state stays consistent when the callee's changes are rolled back. | | |
| XCC-05 | One failing external call (a panicking receiver, a frozen trustline, a missing trustline, clawback) can't block other users' funds. Batch payouts use pull-based claims. | | |
| XCC-06 | Assumptions about upgradeable dependencies are documented. A dependency can change its code with `update_current_contract_wasm`. | | |
| XCC-07 | Return values from cross-contract calls are range- and sanity-checked before use (prices, balances, share amounts). | | |

## 4. State, storage types, and TTL (Guide §3.3)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| STATE-01 | Each datum uses the right storage type: **persistent** for balances, ownership and nonces; **temporary** only for data whose loss is harmless; **instance** only for small, contract-wide configuration. | | |
| STATE-02 | **Temporary** storage is never used for replay protection, nonces, locks, "already claimed" flags, or anything else where disappearing (`has() == false` after expiry) would weaken security. | | |
| STATE-03 | The TTL of persistent entries is extended when they are used (`extend_ttl(key, threshold, extend_to)`), and so is the TTL of the contract instance and code, so important state and the contract itself don't get archived. | | |
| STATE-04 | No security property depends on an entry **expiring**. Anyone can extend any entry's TTL. | | |
| STATE-05 | Defaults for missing keys (`unwrap_or(0)`, `unwrap_or_default()`) are safe. This matters especially for temporary entries, which silently read as missing after expiry. | | |
| STATE-06 | No single storage entry grows without bound (a `Vec`/`Map` under one key). Growing collections use per-item keys. | | |
| STATE-07 | Instance storage stays small and bounded. It is loaded and paid for on every invocation. | | |
| STATE-08 | Storage key and value layouts (`#[contracttype]` enums and structs) stay compatible across upgrades, or there is a migration. | | |

## 5. Resource limits and denial of service (Guide §3.4)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| RES-01 | No loop runs over a user-growable collection in a transaction path. Iteration is paginated or bounded by a constant. | | |
| RES-02 | The worst-case number of ledger entries read and written per call is bounded and below the network's per-transaction limits. | | |
| RES-03 | Worst-case CPU instructions, memory and I/O are measured in tests (`env.cost_estimate()`) with a documented margin under the network limits. | | |
| RES-04 | Event payloads and returned values are bounded in size. | | |

## 6. Arithmetic and value handling (Guide §3.5)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| MATH-01 | Value arithmetic uses `checked_*` and fails with `panic_with_error!` and a typed `#[contracterror]`. It does not rely only on BLD-01. | | |
| MATH-02 | `i128` amounts reject negative values (and zero where it has no meaning) at every entry point. | | |
| MATH-03 | Rounding favours the protocol (round down on payout, up on charge). Share and price math resists first-depositor / donation inflation. | | |
| MATH-04 | Use of `env.ledger().timestamp()` / `sequence()` tolerates validator timestamp drift. `env.prng()` is not used where validators or submitters benefit from predicting or influencing the result. | | |

## 7. Upgrades, errors, and events (Guide §3.6)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| UPG-01 | `update_current_contract_wasm` is gated by admin auth (AUTH-03), ideally behind a timelock, and emits an event. If there is no upgrade path, that is stated as intended. | | |
| UPG-02 | Upgrades include a data-migration and version-check step (see STATE-08). | | |
| ERR-01 | Failure modes use a `#[contracterror]` enum with stable `u32` codes that clients can rely on. | | |
| EVT-01 | Every change to value, permissions, or configuration emits an event (`#[contractevent]`) with enough data for off-chain monitoring and reconciliation. | | |
| TOK-01 | For custom tokens: SEP-41 semantics hold (`expiration_ledger` on allowances is respected, negative amounts are rejected, `transfer_from` uses the spender's auth). For SAC assets: the issuer's clawback, revocation and trustline rules are accounted for (see XCC-05). | | |

## 8. Automated analysis (Guide §5)

| ID | Check | Result | Evidence / notes |
|---|---|---|---|
| AUTO-01 | Unit tests cover every public function, including negative authorization tests (AUTH-10). | | |
| AUTO-02 | A `cargo-fuzz` target exists for each protocol invariant from the trust model. Untrusted callbacks are fuzzer-controlled. Each target has run for at least the agreed time budget with no crash. The corpus is kept. | | |
| AUTO-03 | A static analyzer for Soroban has been run and every result has been triaged. | | |
| AUTO-04 | Every Critical or High finding has a regression test (the exploit test is flipped to assert failure after the fix). | | |

---

## 9. Findings

Copy this block for each finding. Severity uses the rubric in Guide §7.

### F-NN: <title>

| | |
|---|---|
| Severity | Critical / High / Medium / Low / Informational |
| Checklist item(s) | e.g. AUTH-02 |
| Location | `path/to/file.rs:line` |
| Status | Open / Fixed (commit) / Acknowledged (reason) |

**Description.** What is wrong, in one paragraph.

**Impact.** What an attacker gains, and under which preconditions.

**Proof.** Test name, fuzz target and crash input, or exact transaction sequence.

**Remediation.** Concrete code change.

---

## 10. Sign-off

| Exit criterion | Met? |
|---|---|
| Every checklist item is Pass or N/A (with reason). | |
| No Critical or High finding is Open. | |
| Every fixed finding has a regression test (AUTO-04). | |
| The audited WASM hash matches the release build (BLD-03). | |

| Role | Name | Date | Signature / commit |
|---|---|---|---|
| Lead auditor | | | |
| Second reviewer | | | |
| Contract owner | | | |
