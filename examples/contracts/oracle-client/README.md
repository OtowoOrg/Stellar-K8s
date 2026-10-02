# Gas Oracle Client Examples

Clients for the gas fee oracle contract specified in
[docs/architecture/gas-oracle.md](../../../docs/architecture/gas-oracle.md).

| Directory | Contents |
|-----------|----------|
| [`rust/`](rust/) | `no_std` crate. It provides the typed `GasOracleClient` for cross-contract calls, the `quote_fee` helper, and the reference fixed-point EMA (`ema` module). |
| [`typescript/`](typescript/) | A read-only client over Soroban RPC `simulateTransaction`, plus a runnable example. |

## Rust

```sh
cd rust
cargo test
```

The tests check the EMA against the spec's worked example and precision
bounds. They also run `GasOracleClient` in the Soroban host against a mock
oracle that implements the specified ABI.

To use the crate from a contract, add it as a path or git dependency and build
the contract with `stellar contract build`.

## TypeScript

Requires Node.js 22.12 or later.

```sh
cd typescript
npm install
npm test
ORACLE_CONTRACT_ID=C... npm run example
```

`SOROBAN_RPC_URL` and `NETWORK_PASSPHRASE` override the testnet defaults.
