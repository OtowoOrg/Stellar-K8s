# NFT Bridge Vault (ERC-721 ⇄ Soroban)

This contract moves NFTs between Soroban and Ethereum. Native Soroban NFTs
are locked in this vault while they circulate on Ethereum. NFTs locked on
Ethereum are represented on Soroban by synthetic tokens.

## Status: first increment

| Part | Status |
|------|--------|
| Token registry recording each token's origin (`Native` or `Synthetic`) | ✅ [`src/vault.rs`](src/vault.rs) |
| ERC-721-style views: `name`, `symbol`, `owner_of`, `balance_of`, `token` (IDs are `U256`, like `uint256`) | ✅ |
| Admin minting of native NFTs (`mint`) | ✅ |
| `lock`: moves a native NFT into the vault and emits `native_locked` | ✅ |
| Guard: synthetic tokens can't be locked into the native vault | ✅ |
| Guard: a token can't be locked twice | ✅ |
| `mint_synthetic` by the relayer multi-sig, with double-mint protection | ⏳ |
| Burn-to-unlock, the Ethereum → Soroban unlock, and transfers and approvals | ⏳ |
| Round-trip tests and a review of relayer mint/burn privileges | ⏳ |

## Lock event

`lock(owner, token_id, dest_chain_id, dest_address)` requires the owner's
authorisation.

- It rejects:
  - a destination chain ID of `0`;
  - an all-zero Ethereum address;
  - a caller who doesn't own the token;
  - a token that isn't native;
  - a token that is already locked.
- The vault's own address becomes the token's owner.
- It emits this event:

```text
topics: ["native_locked", token_id: U256, owner: Address]
data:   { dest_chain_id: u64, dest_address: BytesN<20>, nonce: u64 }
```

`nonce` is unique for each vault and always increases. Relayers use it to
discard duplicate lock events.

## Build & test

```sh
cargo test
cargo build --target wasm32v1-none --release
```
