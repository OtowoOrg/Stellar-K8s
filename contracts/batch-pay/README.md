# Batch Pay

`batch-pay` transfers a bounded list of token payments from a funded processor
contract instance. The configured admin must authorize initialization and every
batch. Batch identifiers are unique per processor instance while their
persistent Soroban entries remain live.

## Sharding and transaction footprints

Deploy and fund multiple instances of `BatchPayContract` to create independent
payout shards. Submit each transaction to exactly one instance, and use distinct
recipient accounts and transaction source accounts across transactions intended
to execute concurrently. A separate instance has a separate contract address
and token balance; merely using different storage keys inside one instance does
not remove conflicts on that instance's token balance. Transactions paying the
same recipient also contend on that recipient's token balance.

Soroban contracts cannot set a transaction's ledger footprint. The transaction
builder must simulate each invocation, include the resulting read/write
footprint in the transaction envelope, and submit the signed transaction. Token
balances remain shared ledger entries, so transactions that pay the same
recipient or use the same processor instance can conflict and be retried.

The contract bounds each call to 100 transfers. It does not execute those
transfers in parallel; parallelism is between compatible transactions scheduled
by Stellar Core. No throughput claim is made by the unit tests.

## Checks

Run the contract tests with `cargo test --manifest-path contracts/batch-pay/Cargo.toml`.
Build the Wasm artifact with
`cargo build --manifest-path contracts/batch-pay/Cargo.toml --target wasm32-unknown-unknown --release`.

To validate concurrency, deploy and fund independent processor instances on a
local Stellar network, build and simulate 500 transactions with disjoint
processor and recipient balances, submit them under the network's transaction
and resource limits, and compare Stellar Core execution logs and throughput
against an equivalent sequential workload. That network-level result is not
established by this contract crate.
