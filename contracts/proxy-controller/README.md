# proxy-controller

Timelocked upgrade governance for Soroban contracts, addressing issue #36
("Contract Upgradeability Proxy Controller with Delayed Timelock").

This is a standalone Soroban workspace under `contracts/proxy-controller/`.
It is intentionally **not** a member of the top-level `Stellar-K8s` Cargo
workspace (which builds the Kubernetes operator binaries) — it has its own
`Cargo.toml`/`Cargo.lock` and its own build target (`wasm32v1-none`).

## Why this isn't an EVM-style proxy

Soroban has no `delegatecall`. A contract can only ever replace *its own*
installed Wasm, via `env.deployer().update_current_contract_wasm(hash)`
(see [`proxy_controller/src/deployer.rs`](proxy_controller/src/deployer.rs)).
There is no way to deploy a thin "proxy" contract at a stable address that
forwards calls into a separate, swappable "implementation" contract the way
you would on EVM chains.

So instead of a proxy *contract*, `proxy_controller` (see
[`proxy_controller/src/lib.rs`](proxy_controller/src/lib.rs)) is a proxy
*library*: a reusable propose/execute/cancel state machine that gets
compiled directly into every version of the contract you want to make
upgradeable. The governed contract's own address never changes; only the
Wasm installed at that address changes, and only after the timelock. That
is what "transparent" means here — from a caller's perspective the contract
address, and everything already in its storage, is unaffected by an
upgrade.

## Modules

- `proxy_controller/src/lib.rs` — the state machine: `init`, `admin`,
  `security_council`, `pending_upgrade`, `propose_upgrade`,
  `cancel_upgrade`, `execute_upgrade`, plus the `ProxyError` and
  `PendingUpgrade` types and `ProposeUpgradeEvent` / `CancelUpgradeEvent` /
  `ExecuteUpgradeEvent` contract events.
- `proxy_controller/src/deployer.rs` — the two `env.deployer()` calls this
  crate needs: `upload` (stage new bytecode, get its hash) and `apply`
  (swap the current contract's Wasm to a previously-uploaded hash).
- `mock_v1/`, `mock_v2/` — a worked example: two versions of the same toy
  contract (a stored `u32` counter), used as the "before" and "after" of an
  upgrade in the test suite.
- `proxy_controller_tests/` — end-to-end tests that build `mock_v1` and
  `mock_v2` to real Wasm and drive a full propose → wait → execute cycle
  through `env.deployer()`, not a simulated/native stand-in.

## Building and testing

```sh
cd contracts/proxy-controller
make test     # builds mock_v1/mock_v2 to real Wasm, then runs all tests
```

`make build` builds `proxy_controller` itself to Wasm (for publishing as a
crate other contracts depend on, it doesn't need to be deployed on its
own). Both targets use `wasm32v1-none`, not `wasm32-unknown-unknown` —
newer Rust toolchains enable Wasm `reference-types`/`multi-value` by
default on `wasm32-unknown-unknown`, which the Soroban host environment
(`soroban-env-host` 23.x) rejects at Wasm-load time
(`HostError: Error(WasmVm, InvalidAction)`, "reference-types not enabled").
`wasm32v1-none` is the fixed-feature-set target Soroban tooling has moved
to for exactly this reason.

## Migration guide: making a contract upgradeable

1. Add `proxy-controller` as a path/crate dependency.
2. Define your own `#[contracttype] enum DataKey { ... }` for your
   business data. **Never** name a variant `ProxyAdmin`,
   `ProxySecurityCouncil`, or `ProxyPendingUpgrade` (see "storage layout"
   below).
3. In your `__constructor` (not a plain callable `initialize` — see
   "front-running" below), call `proxy_controller::init(&env, &admin,
   &security_council)?`.
4. Add three thin passthrough methods to your `#[contractimpl]` block:
   ```rust
   pub fn propose_upgrade(env: Env, new_wasm: Bytes) -> Result<BytesN<32>, ProxyError> {
       proxy_controller::propose_upgrade(&env, new_wasm)
   }
   pub fn cancel_upgrade(env: Env, caller: Address) -> Result<(), ProxyError> {
       proxy_controller::cancel_upgrade(&env, caller)
   }
   pub fn execute_upgrade(env: Env) -> Result<BytesN<32>, ProxyError> {
       proxy_controller::execute_upgrade(&env)
   }
   ```
5. When you build v2, **keep the `DataKey` enum's existing variants in the
   same order with the same names** (you may append new variants at the
   end) and **keep the three passthrough methods** so the contract can be
   upgraded again later. Everything else — new methods, changed method
   bodies, new `DataKey` variants for new fields — is free to change.
6. To ship v2: `admin` calls `propose_upgrade(v2_wasm_bytes)`, wait 48
   hours (`PendingUpgrade.execute_after`), then `admin` calls
   `execute_upgrade()`. `admin` or `security_council` may call
   `cancel_upgrade(caller)` at any point before execution.

`mock_v1` → `mock_v2` in this repo is a complete worked example of steps
2–5, and `proxy_controller_tests/tests/upgrade.rs` is a complete worked
example of step 6.

## Security analysis

### Storage layout collision prevention

Soroban's `env.storage()` is keyed by **contract address**, not by the
Wasm code currently installed there. Swapping code with
`update_current_contract_wasm` therefore never touches existing storage by
itself — the risk is entirely in whether the *new* code interprets
existing keys the same way the old code did.

`#[contracttype]` enums with unit (fieldless) variants serialize to
storage as an `ScVal` vector tagged by the variant's **name**, not its
declaration order or discriminant value. Two consequences this design
relies on:

- `proxy_controller`'s own keys (`ProxyAdmin`, `ProxySecurityCouncil`,
  `ProxyPendingUpgrade`) live in a completely separate namespace from
  whatever `DataKey` enum the host contract defines, because they are a
  *different Rust enum type* with names a host contract's own key enum is
  documented never to reuse. Renaming, reordering, or adding variants to
  the host's `DataKey` cannot collide with them.
- Within the host's own `DataKey` enum, a variant keeps resolving to the
  same storage slot across an upgrade as long as its **name** doesn't
  change. Reordering variants is safe (encoding is by name); *renaming* a
  variant that already has stored data is not — that is a data migration,
  not a routine upgrade, and needs an explicit one-time migration method
  in the new version that reads the old key and writes the new one.
- Storage *durability class* (`instance()` vs `persistent()` vs
  `temporary()`) is part of a key's identity too. Moving an existing field
  from one durability class to another between versions is equally a
  migration, not a routine upgrade — the old entries won't be visible
  under the new class.

The `mock_v1` → `mock_v2` test asserts this directly: `DataKey::Value` is
declared identically in both crates, `v1.initialize` writes 42 under it,
and after the real Wasm swap `v2.get_value()` reads back 42 with no
migration step.

### Timelock and authorization

- `propose_upgrade` requires `admin.require_auth()`; only the configured
  admin can start the clock on new bytecode.
- `execute_upgrade` independently requires `admin.require_auth()` *and*
  checks `env.ledger().timestamp() >= pending.execute_after`
  (`TIMELOCK_SECONDS = 48 * 60 * 60`). Either check failing aborts the
  call — there's no way to execute early even as the admin.
- `cancel_upgrade` takes an explicit `caller: Address`, requires
  `caller.require_auth()`, and then checks `caller` against *both* the
  stored admin and the stored security council, rejecting anyone else with
  `ProxyError::Unauthorized`. This is what gives the security multi-sig an
  independent emergency veto that doesn't depend on the admin key at all
  (see `proxy_controller_tests::security_council_can_cancel_a_pending_upgrade`).
- Only one upgrade may be pending at a time (`UpgradeAlreadyPending`);
  cancel first to replace an in-flight proposal rather than being able to
  quietly overwrite it.

### Known limitation: `initialize` vs. a real constructor

`mock_v1::initialize` in this repo is a plain callable method for test
simplicity, guarded only by "already initialized" — it is **not** itself
auth-gated, because at the moment it runs there is no admin in storage yet
to check against. On a real network this is front-runnable: whoever's
transaction to call `initialize` lands first wins, which could let an
attacker set themselves as admin on a deployed-but-not-yet-initialized
contract. Soroban's actual fix for this is a `__constructor` function,
which the host guarantees runs atomically as part of contract creation and
therefore cannot be front-run by a separate transaction. Production use of
`proxy_controller::init` should call it from `__constructor`, not from a
method like `mock_v1::initialize` — this repo uses the latter purely
because it keeps the test harness (which needs to register Wasm and then
separately invoke setup) straightforward; see `mock_v1/src/lib.rs` for the
inline note at the call site.

### What this does not cover (out of scope for this pass)

- No governance beyond a single admin key plus a single security-council
  address — no threshold/M-of-N signing is implemented here (Soroban
  account contracts can provide that under the same `Address` type, but
  this crate doesn't assume or require one).
- No on-chain diffing/validation of the proposed bytecode's shape (e.g.
  checking it doesn't remove existing public functions) — the timelock is
  the review mechanism; automated shape-checking would be a reasonable
  follow-up.
- No pause/circuit-breaker separate from cancellation.
