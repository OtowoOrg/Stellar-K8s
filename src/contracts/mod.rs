//! Soroban-style smart contract primitives for Stellar infrastructure.
//!
//! These modules implement the contract logic referenced by the GitHub issues
//! and are designed to compile to WASM via the Soroban SDK toolchain.  The
//! Rust source lives inside the main operator crate so that it can be exercised
//! by `cargo test` without a separate build step.
//!
//! # Modules
//!
//! | Module           | Issue | Description                                              |
//! |------------------|-------|----------------------------------------------------------|
//! | `upgrade_proxy`  | #110  | Upgradable governance proxy with storage layout guard    |
//! | `amm_pool`       | #114  | Constant-product AMM with LP tokens and fee support      |
//! | `merkle_proof`   | #113  | SHA-256 / Keccak-256 Merkle proof verification primitive |

pub mod amm_pool;
pub mod merkle_proof;
pub mod upgrade_proxy;
