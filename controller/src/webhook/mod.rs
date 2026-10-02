// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Webhook sub-modules for the controller crate.
//!
//! The [`wasm_mutator`] module implements the MutatingAdmissionWebhook handler
//! that intercepts StellarNode WASM deployment payloads and routes them through
//! the wasm-opt bytecode optimizer sidecar before they are persisted to etcd.

pub mod wasm_mutator;

pub use wasm_mutator::{wasm_mutate_handler, WasmMutatorState};
