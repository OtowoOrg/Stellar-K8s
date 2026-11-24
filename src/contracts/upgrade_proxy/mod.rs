//! Upgradable governance proxy contract with storage layout protection.
//!
//! # Overview
//!
//! This module provides a Soroban-style upgradable proxy that enforces
//! key-prefix namespace isolation between implementation state and governance
//! state, preventing storage collisions across WASM bytecode versions.
//!
//! # Sub-modules
//!
//! - [`storage`] — typed key-prefix storage with namespace isolation
//! - [`lib`]     — upgrade lifecycle logic (propose / execute / rollback)

pub mod lib;
pub mod storage;

pub use lib::{ProxyError, UpgradeProxy};
pub use storage::{
    keys_are_isolated, namespaced_key, ContractStorage, PendingUpgrade, StorageError, WasmRecord,
    GOV_PREFIX, IMPL_PREFIX,
};
