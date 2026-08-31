//! Soroban host bindings for the Reentrancy Guard middleware.
//!
//! This module adapts the storage-agnostic core [`crate::guard::ReentrancyGuard`]
//! to the Soroban execution environment: the write-lock stack is persisted in
//! the contract instance's storage, which is exactly the state that is shared
//! and visible across a cross-contract call within a single transaction. A
//! nested (re-entrant) invocation of the wrapped sub-contract therefore observes
//! the same stack as its ancestor and is reverted by [`crate::guard::GuardError`].
//!
//! The middleware is compiled to Wasm by building with the `soroban` feature:
//!
//! ```text
//! cargo build -p stellar-soroban-reentrancy-guard --features soroban \
//!     --target wasm32-unknown-unknown --release
//! ```
//!
//! # Overhead note
//!
//! The Soroban binding adds a single instance-storage read and write per guarded
//! invocation (the stack is a single small [`soroban_sdk::Bytes`] entry), and the
//! linear slot scan is bounded by [`crate::guard::MAX_DEPTH`] — comfortably
//! inside the `< 500` instruction budget.
//!
//! # Storage key
//!
//! The middleware stores exactly one value (the write-lock stack) under a single
//! short instance-storage key. This sidesteps Soroban's fixed-length `Symbol`
//! limits while keeping the on-ledger footprint minimal.

use crate::guard::{GuardError, GuardStorage};
use alloc::vec::Vec;

// A `no_std` (alloc) Soroban guest must supply its own global allocator. We
// provide a minimal non-freeing bump allocator that grows Wasm linear memory,
// mirroring Soroban's own contract allocator semantics: a single transaction
// only ever allocates, so reclaiming is unnecessary. This keeps the middleware
// self-contained when compiled to `wasm32-unknown-unknown`.
mod allocator {
    use core::alloc::{GlobalAlloc, Layout};

    struct Bump;

    // SAFETY: bump allocator is a valid `GlobalAlloc` for a single-shot Wasm
    // execution; `dealloc` is a no-op by design.
    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump_alloc(layout.size(), layout.align())
        }
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    const LOG_PAGE_SIZE: usize = 16;
    const PAGE_SIZE: usize = 1 << LOG_PAGE_SIZE; // 64KiB
    const MEM: u32 = 0; // memory 0 is the only legal one

    static mut CURSOR: usize = 0;
    static mut LIMIT: usize = 0;

    unsafe fn bump_alloc(bytes: usize, align: usize) -> *mut u8 {
        if LIMIT as usize == 0 {
            CURSOR = core::arch::wasm32::memory_size(MEM) * PAGE_SIZE;
            LIMIT = CURSOR;
        }
        let mask = align - 1;
        let start = (CURSOR + mask) & !mask;
        let end = start + bytes;
        if end > LIMIT {
            let pages = (bytes + PAGE_SIZE - 1) / PAGE_SIZE;
            core::arch::wasm32::memory_grow(MEM, pages);
            LIMIT += pages * PAGE_SIZE;
        }
        CURSOR = end;
        start as *mut u8
    }

    #[global_allocator]
    static GLOBAL: Bump = Bump;
}

/// Instance-storage key under which the whole write-lock stack is persisted.
///
/// A single short `Symbol` is used regardless of the logical key the core guard
/// asks for, because this binding only ever persists one value (the stack).
const STACK_SYMBOL: &str = "reentry_stack";

/// A [`GuardStorage`] backed by Soroban contract-instance storage.
///
/// Wrap an [`Env`](soroban_sdk::Env) with this and hand it to
/// [`crate::guard::ReentrancyGuard::new`]:
///
/// ```rust,ignore
/// # use soroban_sdk::Env;
/// use stellar_soroban_reentrancy_guard::guard::{ReentrancyGuard, AccessKind, SlotId};
/// let guard = ReentrancyGuard::new(
///     stellar_soroban_reentrancy_guard::host::SorobanGuardStorage::new(env.clone()),
/// );
/// match guard.enter(SlotId::from_u64(1), AccessKind::Write) { ... }
/// ```
pub struct SorobanGuardStorage {
    env: soroban_sdk::Env,
}

impl SorobanGuardStorage {
    /// Wrap a Soroban [`Env`](soroban_sdk::Env).
    pub fn new(env: soroban_sdk::Env) -> Self {
        Self { env }
    }
}

impl GuardStorage for SorobanGuardStorage {
    fn read(&mut self, _key: &[u8]) -> Result<Option<Vec<u8>>, GuardError> {
        use soroban_sdk::{Bytes, Symbol};
        let storage = self.env.storage().instance();
        let sym = Symbol::new(&self.env, STACK_SYMBOL);
        let empty = Bytes::new(&self.env);
        let bytes: Bytes = storage.get(&sym).unwrap_or(empty);
        let mut out = Vec::with_capacity(bytes.len() as usize);
        for b in bytes.iter() {
            out.push(b);
        }
        Ok(Some(out))
    }

    fn write(&mut self, _key: &[u8], value: &[u8]) -> Result<(), GuardError> {
        use soroban_sdk::{Bytes, Symbol};
        let storage = self.env.storage().instance();
        let sym = Symbol::new(&self.env, STACK_SYMBOL);
        let bytes = Bytes::from_slice(&self.env, value);
        storage.set(&sym, &bytes);
        Ok(())
    }
}

impl Clone for SorobanGuardStorage {
    fn clone(&self) -> Self {
        Self {
            env: self.env.clone(),
        }
    }
}
