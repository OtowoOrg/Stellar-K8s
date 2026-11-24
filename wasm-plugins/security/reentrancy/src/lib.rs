//! Reentrancy Guard Sub-Contract Middleware for Soroban
//!
//! A native reentrancy guard that can be enforced through the Stellar-K8s
//! custom validation (Wasm) layer on high-value deployments.
//!
//! # Problem
//!
//! Soroban contracts call each other through cross-contract invocations
//! (`env.invoke_contract` / `env.call`). A malicious or buggy target contract
//! can, before settling its own accounting, *re-enter* a caller's mutating
//! function and mutate the same state variables a second time. Because the
//! caller's state has not yet been committed, the nested invocation observes
//! stale (or partially-mutated) state and can steal funds or corrupt
//! invariants — the classic reentrancy attack.
//!
//! # Approach
//!
//! The middleware is a thin *sub-contract* that wraps a mutating function. It
//! maintains an execution stack keyed by a *state-variable slot*. When a
//! mutating (write) access is requested for a slot that is already present on
//! the current cross-contract call stack, the middleware reverts the nested
//! invocation rather than allowing the second, unsafe mutation. Read-only
//! (`view`) callbacks that do not mutate state are never locked, so they cannot
//! produce false positives.
//!
//! The state machine itself is **storage-agnostic** (see [`guard`]): it is
//! expressed against a minimal [`GuardStorage`] trait so that it can be:
//!
//! - unit tested exhaustively on stable Rust with no external dependencies, and
//! - bound to Soroban host storage through the optional `soroban` feature, or
//!   to the operator's Wasm runtime input for admission-time enforcement.
//!
//! # Modules
//!
//! - [`config`] — ConfigMap-driven scoping (enable/disable per namespace or
//!   contract ID) for the middleware.
//! - [`guard`] — the core, storage-agnostic reentrancy guard state machine.
//! - [`vuln`] — a deliberately vulnerable mock contract plus its guarded
//!   equivalent, used to prove the middleware blocks reentrancy exploitation.
//! - [`mem`] — an in-memory [`GuardStorage`] for tests and demonstrations.
//! - [`host`] — Soroban host bindings (`soroban` feature) that adapt the core
//!   state machine to `soroban_sdk::Env` storage.
//!
//! # Design reference
//!
//! Mirrors the semantics of OpenZeppelin's `ReentrancyGuard`, adapted to
//! Soroban's explicit cross-contract execution model: a boolean
//! (per-slot) "entered/not-entered" is insufficient because a read callback is
//! a legitimate re-entry; therefore we track the full call stack of *mutating*
//! slots and only reject a mutation when the same slot is re-entered.
//!
//! # `no_std` support
//!
//! When built with the `soroban` feature the crate is a `no_std` (alloc) Soroban
//! guest and can be compiled to `wasm32-unknown-unknown`. By default it links
//! the standard library and is fully unit-testable on stable Rust.

#![cfg_attr(feature = "soroban", no_std)]

extern crate alloc;

pub mod config;
pub mod guard;
pub mod mem;
pub mod vuln;

#[cfg(feature = "soroban")]
pub mod host;

pub use config::ReentrancyGuardConfig;
pub use guard::{
    AccessKind, ExecutionFrame, GuardError, GuardStorage, GuardedExecution, ReentrancyGuard,
};
pub use mem::InMemoryStorage;

/// The middleware version, reported to the operator admission webhook.
pub const MIDDLEWARE_VERSION: &str = env!("CARGO_PKG_VERSION");
