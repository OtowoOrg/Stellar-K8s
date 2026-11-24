# fee-bumper

A **partial** implementation of issue #29 ("Automated Fee-Bump Transaction
Wrapper Sub-Contract"). Same posture as `contracts/proxy-controller` and
`wasm-plugins/cache-plugin`: a working, tested slice of the issue, with an
explicit accounting of what a Soroban contract genuinely cannot do and
what's left out of scope here.

## What a Soroban contract can and can't do here

The issue asks for a contract that "validates an inner transaction
envelope and wraps it in a fee-bump transaction structure." **That part is
impossible inside a Soroban contract.** A contract executes inside a
transaction that has already been built, fee-bid, and submitted — there
is no host function to construct a `FeeBumpTransactionEnvelope`, sign one,
or broadcast one. That is necessarily off-chain client code (e.g. a
relayer service using `stellar-sdk` / `js-stellar-sdk` against Horizon or
RPC), not a contract, on Soroban as it exists today.

What *can* live on-chain — and is what this contract implements — is the
reimbursement accounting around that off-chain step: deciding, before the
relayer fronts a real network fee, whether it will actually get paid back
for doing so, and then collecting exactly that reimbursement afterward.

### The off-chain relayer flow this contract is designed for

```
 1. Sponsored account, once: approve(account, fee_bumper_address, allowance, ledger)
                                 on the reimbursement token (SEP-41).
 2. Relayer:  fee_bumper.authorize_sponsorship(account, estimated_fee)
                 -> rejects if rate-limited or underfunded/under-approved;
                    otherwise escrows `estimated_fee` immediately and
                    returns an expiry timestamp.
 3. Relayer:  builds the inner transaction, wraps it in a real
                 FeeBumpTransactionEnvelope (off-chain, stellar-sdk),
                 signs it as fee source, and submits it to the network.
 4. Relayer:  once it observes the actual network fee charged,
              fee_bumper.settle_reimbursement(account, actual_fee)
                 -> pays itself `actual_fee` out of escrow, refunds the
                    sponsored account any overestimate.
 5. If the relayer never broadcasts (step 3 abandoned), anyone can call
    fee_bumper.expire_stale_sponsorship(account) once the escrow's TTL
    (10 minutes) has passed, refunding it back to the account.
```

## What's implemented

- **`fee_bumper/src/lib.rs`** — the contract: `__constructor`,
  `authorize_sponsorship`, `settle_reimbursement`,
  `expire_stale_sponsorship`, plus view functions (`admin`,
  `reimbursement_token`, `rate_limit_config`, `account_window`,
  `pending_sponsorship`).
- **`fee_bumper/src/relayer.rs`** — the rate-limiting state machine
  (`RateLimitConfig`, `AccountWindow`, `check_and_advance`), kept
  `Env`-free and unit tested directly (7 tests: fresh window, staying
  under cap, hitting cap, the exact window boundary, one tick before the
  boundary, cap-then-reset, invalid configs).
- **`fee_bumper_tests/tests/sponsorship.rs`** — 8 integration tests
  against the contract registered natively alongside a real Stellar Asset
  Contract (`Env::register_stellar_asset_contract_v2`) as the
  reimbursement token, so `approve`/`transfer_from`/`transfer`/`balance`
  all run through an actual SEP-41 token, not a stand-in. Covers: exact
  collection + refund of the overestimate; **100 sequential
  authorize/settle cycles** (the issue's required validation scenario);
  rate-limit rejection once the window cap is reached; rejecting a second
  authorization while one is already pending; rejecting an underfunded
  account before anything is escrowed; rejecting a settlement that asks
  for more than was escrowed (and confirming the escrow survives that
  rejection intact); reclaiming an expired, unsettled escrow (and
  confirming it can't be reclaimed early); an account "self-healing" a
  stale escrow by simply authorizing again.

Run it:

```sh
cd contracts/fee-bumper
cargo test --manifest-path fee_bumper/Cargo.toml         # 7 unit tests
cargo test --manifest-path fee_bumper_tests/Cargo.toml   # 8 integration tests
cargo build --manifest-path fee_bumper/Cargo.toml --target wasm32v1-none --release  # real .wasm
```

(`wasm32v1-none`, not `wasm32-unknown-unknown` — see
`contracts/proxy-controller/README.md` for why: current Rust defaults to
Wasm features the Soroban host VM rejects on the older target name.)

## Security analysis

### Front-running: why this contract escrows instead of only checking `allowance`

An earlier draft of this contract only checked `token.allowance(account,
contract) >= estimated_fee` and `token.balance(account) >= estimated_fee`
during `authorize_sponsorship`, deferring the actual `transfer_from` to
`settle_reimbursement`. That has a real front-running hole: there is an
unavoidable gap between the on-chain pre-flight check and the relayer's
off-chain broadcast of the real fee-bump transaction. In that gap, the
account (or anyone it cooperates with) can submit a separate transaction
that spends the balance down or calls `approve(..., 0, ...)` to revoke the
allowance — passing the pre-flight check but leaving `settle_reimbursement`
unable to collect anything, after the relayer has already fronted the
real network fee it can never get back.

This contract closes that hole structurally: **`authorize_sponsorship`
calls `transfer_from` immediately**, moving `estimated_fee` into the
contract's own custody atomically with the rate-limit reservation, in the
same transaction. Once that call returns successfully, there is no later
transaction the account can submit that un-escrows the funds — they are
already gone from its balance and held by the contract. This is also what
lets `settle_reimbursement` refund the *exact* difference (the issue's own
"collects exact reimbursement" wording) rather than just capping what it
collects.

### Bounding relayer exposure ("relayer exhaustion")

- **Per-account rate limiting** (`relayer::check_and_advance`) caps how
  many sponsorships a single account can be authorized for per window,
  regardless of how many requests it (or an attacker routing many
  low-value accounts through it) sends. Worst case per account per window
  is `max_per_window * (largest estimated_fee it ever gets approved for)`
  — a bound the operator sets via the constructor's `max_per_window`/
  `window_seconds`, and can tune per deployment.
- **One pending escrow per account at a time**
  (`SponsorshipAlreadyPending`) prevents an account from stacking up
  multiple simultaneous escrows beyond what the rate limit alone would
  imply, and keeps the accounting (one `PendingSponsorship` per account)
  simple enough to reason about.
- **`settle_reimbursement` can never collect more than what was escrowed**
  (`ActualFeeExceedsEscrow`) — this is what protects the *sponsored
  account* from "maximum network fee turbulence": if the live network fee
  spikes well above the relayer's estimate, the relayer eats that
  difference (or should re-run its own pre-flight estimate before
  broadcasting); it cannot pass an unbounded fee spike through to the
  account after the fact.
- **Escrow expiry** (`PENDING_SPONSORSHIP_TTL_SECONDS = 600`) bounds how
  long funds can sit in limbo if a relayer authorizes and then never
  settles (crash, decided not to broadcast, etc.) — `expire_stale_sponsorship`
  is callable by anyone once the deadline passes, so the account is never
  permanently reliant on the relayer's admin key to get its escrow back.

### A bug caught and fixed during review, worth calling out explicitly

An earlier version of `settle_reimbursement` deleted the pending escrow
from storage *before* validating `actual_fee` against it. On the
`ActualFeeExceedsEscrow` rejection path, that meant: the escrowed tokens
stayed in the contract's balance (never paid to admin, since the function
returned `Err` before the `transfer` call), but the `PendingSponsorship`
entry that both `settle_reimbursement` and `expire_stale_sponsorship`
depend on to find and move those funds was already gone — permanently
stranding the escrow with no function able to reach it. The fix
(`fee_bumper/src/lib.rs`) validates fully *before* removing the storage
entry or moving any funds, so a rejected settlement leaves the escrow
exactly as it was: settleable again with a corrected amount, or
reclaimable via `expire_stale_sponsorship` once it passes its deadline.
`settling_more_than_the_escrowed_amount_is_rejected_and_leaves_the_escrow_intact`
in the test suite asserts this directly.

### Known limitations (out of scope for this slice)

- **No actual fee-bump construction, signing, or submission** — see
  "What a Soroban contract can and can't do here" above. This is not a
  scope cut so much as a hard boundary of the platform; a real deployment
  needs off-chain relayer software built around this contract, which is
  not included here.
- **Single admin/relayer key**, not an M-of-N multisig or a pool of
  relayer identities — `admin` is one `Address` set at construction.
  `settle_reimbursement` requires `admin.require_auth()`, so a compromised
  admin key can settle arbitrary pending escrows at whatever `actual_fee`
  it likes up to the escrowed cap (bounded — it still can't exceed the
  escrow — but it can shortchange refunds within that bound). Real
  deployments should use a Soroban account contract as `admin` for actual
  multisig, which this contract's `Address`-typed admin already supports
  without any code change.
- **No per-token-decimals or price-oracle awareness.** `estimated_fee`
  and `actual_fee` are raw token units the relayer computes off-chain
  (e.g. converting the real XLM network fee into its reimbursement token
  at whatever rate it uses); this contract does no conversion or
  oracle-based validation of whether that rate is fair.
- **Global rate-limit config, not per-account tiers.** One
  `RateLimitConfig` applies to every account; there's no allowlist/tiering
  for, say, higher-volume trusted accounts.
