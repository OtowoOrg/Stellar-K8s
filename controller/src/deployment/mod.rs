// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Deployment pipeline modules.
//!
//! This module groups all components involved in the StellarNode deployment
//! lifecycle, including the WASM bytecode optimizer that reduces on-chain
//! storage rent before contracts are deployed.

pub mod optimizer;

pub use optimizer::{OptimizerConfig, OptimizationResult, WasmOptimizer};
