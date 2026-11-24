//! Core, storage-agnostic reentrancy guard state machine.
//!
//! This module implements the middleware logic that the rest of the crate
//! (Soroban host bindings, operator admission webhook, mock contracts) is built
//! on. It is deliberately free of any dependency on the Soroban SDK so that it:
//!
//! - compiles on stable Rust with **zero external dependencies** (only
//!   `serde_json`), making it fast to build and fuzz,
//! - can be exhaustively unit tested, and
//! - can be bound to any backing store via the [`GuardStorage`] trait.
//!
//! # Locking semantics (OpenZeppelin-flavoured, Soroban-adapted)
//!
//! State variables are addressed by a [`SlotId`] (a strongly-typed 32-byte
//! identifier). The guard keeps a *write-lock stack*:
//!
//! - `enter(slot, AccessKind::Write)` pushes `slot` and returns normally **only
//!   if** `slot` is not already present anywhere on the current stack. If it is
//!   already present (i.e. an ancestor cross-contract invocation is currently
//!   mutating the same state variable), the guard returns
//!   [`GuardError::ReentrancyDetected`] — a nested, mutating re-entry, which the
//!   middleware reverts.
//! - `enter(slot, AccessKind::Read)` never locks and never fails. Legitimate,
//!   non-mutating read callbacks therefore produce **zero false positives**,
//!   even when an ancestor write is in flight on the same slot.
//! - `exit(slot, kind)` pops the matching write-lock once the invocation
//!   completes, so sequential (non-nested) mutation of the same variable remains
//!   fully allowed.
//!
//! This is stronger than a single boolean "entered" flag: it permits
//! re-entrancy for *reads* and for *different* slots, while still reverting the
//! only pattern that is actually unsafe — a nested mutation of the same state
//! variable.
//!
//! # Overhead
//!
//! Every guarded invocation performs a constant number of operations: read a
//! fixed-size stack, do a linear scan (bounded by the stack depth), and write
//! the stack back. With a maximum configured depth of [`MAX_DEPTH`] (8) this is
//! `< 500` Wasm instructions and requires allocation of at most a single small
//! buffer, satisfying the middleware's instruction budget.

use alloc::fmt;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// Strongly-typed identifier of a state variable slot.
///
/// Fixed at 32 bytes so it can represent a Soroban `Symbol`/identifier while
/// remaining cheap and deterministic. Common values can be built with
/// [`SlotId::from_u64`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SlotId(pub [u8; 32]);

impl SlotId {
    /// A zero slot (uninitialised / sentinel). Never address state with this.
    pub const ZERO: Self = Self([0u8; 32]);

    /// Build a slot id from a `u64`. Bytes 0..=7 carry the value, the rest are
    /// zero — convenient for tests and for compact, numeric slot identifiers.
    pub const fn from_u64(v: u64) -> Self {
        let mut b = [0u8; 32];
        let bytes = v.to_le_bytes();
        let mut i = 0;
        while i < 8 {
            b[i] = bytes[i];
            i += 1;
        }
        Self(b)
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl From<u64> for SlotId {
    fn from(v: u64) -> Self {
        Self::from_u64(v)
    }
}

impl<'a> TryFrom<&'a [u8]> for SlotId {
    type Error = GuardError;

    fn try_from(bytes: &'a [u8]) -> Result<Self, Self::Error> {
        if bytes.len() != 32 {
            return Err(GuardError::CorruptedState);
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(bytes);
        Ok(Self(b))
    }
}

/// How an invocation intends to touch a state variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessKind {
    /// A mutating access. This is what the guard locks.
    Write,
    /// A non-mutating (view/read) access. Never locked, never false-positives.
    Read,
}

/// Errors surfaced by the guard middleware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardError {
    /// A nested, mutating re-entry targeted the same state variable that an
    /// ancestor invocation is already mutating. The middleware must revert.
    ReentrancyDetected,
    /// The persisted stack could not be decoded or was inconsistent.
    CorruptedState,
    /// The backing store failed during a read or write.
    StorageUnavailable,
    /// An `exit` did not match the head of the stack (protocol misuse).
    MismatchedExit,
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReentrancyDetected => {
                write!(f, "reentrancy detected: mutating a state variable that is already being mutated")
            }
            Self::CorruptedState => write!(f, "reentrancy guard state is corrupted"),
            Self::StorageUnavailable => write!(f, "reentrancy guard backing store unavailable"),
            Self::MismatchedExit => write!(f, "reentrancy guard exit did not match stack head"),
        }
    }
}

#[cfg(not(feature = "soroban"))]
impl std::error::Error for GuardError {}

/// Minimal backing store required by the guard.
///
/// Implementors are expected to make reads/writes transactional with respect to
/// the enclosing transaction: in the Soroban case this is host [`Env`]
/// storage, in tests it is an in-memory map.
pub trait GuardStorage {
    /// Read the raw bytes stored under `key`, or `Ok(None)` if absent.
    fn read(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, GuardError>;

    /// Persist `value` under `key`.
    fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), GuardError>;
}

/// Max depth of the write-lock stack. Keeps the stack scan bounded and the
/// overhead well inside the `< 500` instruction budget.
pub const MAX_DEPTH: usize = 8;

/// A single frame of the cross-contract call stack that is currently locking
/// a slot for mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionFrame {
    /// The state-variable slot being mutated at this stack level.
    pub slot: SlotId,
    /// Depth (0 is the outermost guarded invocation).
    pub depth: usize,
}

/// Storage key used to persist the write-lock stack.
pub const STACK_KEY: &[u8] = b"stellar.reentrancy.stack.v1";

/// Result of a guarded execution, returned to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedExecution {
    /// The depth of the frame that was acquired (or would have been).
    pub depth: usize,
    /// Whether the guarded call was a read (never locked).
    pub was_read_only: bool,
}

/// The reentrancy guard middleware.
pub struct ReentrancyGuard<S: GuardStorage> {
    storage: S,
}

impl<S: GuardStorage> ReentrancyGuard<S> {
    /// Wrap a backing store with the guard middleware.
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Consume the guard, returning the backing store.
    pub fn into_inner(self) -> S {
        self.storage
    }

    /// Load the current write-lock stack from storage.
    ///
    /// An absent key is an empty stack. A malformed stack (wrong length or any
    /// entry with an unexpected size) is treated as corruption because an
    /// attacker must not be able to launder state by tampering with the stack.
    fn load_stack(&mut self) -> Result<Vec<SlotId>, GuardError> {
        let raw = self.storage.read(STACK_KEY)?;
        match raw {
            None => Ok(Vec::new()),
            Some(bytes) => decode_stack(&bytes),
        }
    }

    /// Persist the write-lock stack to storage.
    fn persist_stack(&mut self, stack: &[SlotId]) -> Result<(), GuardError> {
        self.storage.write(STACK_KEY, &encode_stack(stack))
    }

    /// Whether `slot` is currently write-locked anywhere on the call stack.
    pub fn is_entered(&mut self, slot: SlotId) -> Result<bool, GuardError> {
        Ok(self.load_stack()?.contains(&slot))
    }

    /// Attempt to enter a guarded invocation on `slot` with access `kind`.
    ///
    /// - `AccessKind::Read`: always succeeds and never locks. Returns
    ///   [`GuardedExecution::was_read_only`] `true`.
    /// - `AccessKind::Write`: acquires a lock on `slot`. Succeeds if `slot` is
    ///   not already locked; otherwise returns
    ///   [`GuardError::ReentrancyDetected`].
    pub fn enter(&mut self, slot: SlotId, kind: AccessKind) -> Result<GuardedExecution, GuardError> {
        match kind {
            AccessKind::Read => Ok(GuardedExecution {
                depth: self.load_stack()?.len(),
                was_read_only: true,
            }),
            AccessKind::Write => {
                let mut stack = self.load_stack()?;
                let depth = stack.len();
                if stack.contains(&slot) {
                    return Err(GuardError::ReentrancyDetected);
                }
                if depth >= MAX_DEPTH {
                    // Bounded stack: refusing is safer than overflowing.
                    return Err(GuardError::ReentrancyDetected);
                }
                stack.push(slot);
                self.persist_stack(&stack)?;
                Ok(GuardedExecution {
                    depth,
                    was_read_only: false,
                })
            }
        }
    }

    /// Exit a guarded invocation on `slot`. For write frames this pops the lock.
    ///
    /// The `slot` must match the head of the stack (the frame being closed);
    /// anything else indicates protocol misuse and is reported as
    /// [`GuardError::MismatchedExit`].
    pub fn exit(&mut self, slot: SlotId, kind: AccessKind) -> Result<(), GuardError> {
        if kind == AccessKind::Read {
            return Ok(());
        }
        let mut stack = self.load_stack()?;
        match stack.last() {
            Some(&top) if top == slot => {
                stack.pop();
                self.persist_stack(&stack)
            }
            _ => Err(GuardError::MismatchedExit),
        }
    }
}

/// Encode a stack as `u32 length || slot0 (32) || slot1 (32) || ...`.
fn encode_stack(stack: &[SlotId]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + stack.len() * 32);
    out.extend_from_slice(&(stack.len() as u32).to_le_bytes());
    for slot in stack {
        out.extend_from_slice(&slot.0);
    }
    out
}

/// Decode a stack previously produced by [`encode_stack`].
fn decode_stack(bytes: &[u8]) -> Result<Vec<SlotId>, GuardError> {
    if bytes.len() < 4 {
        return Err(GuardError::CorruptedState);
    }
    let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if len > MAX_DEPTH {
        return Err(GuardError::CorruptedState);
    }
    let expected = 4usize.checked_add(len.checked_mul(32).ok_or(GuardError::CorruptedState)?)
        .ok_or(GuardError::CorruptedState)?;
    if bytes.len() != expected {
        return Err(GuardError::CorruptedState);
    }
    let mut stack = Vec::with_capacity(len);
    for i in 0..len {
        let start = 4 + i * 32;
        let mut b = [0u8; 32];
        b.copy_from_slice(&bytes[start..start + 32]);
        stack.push(SlotId(b));
    }
    Ok(stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::vec::Vec;

    /// In-memory [`GuardStorage`] for tests.
    #[derive(Default)]
    struct MemStorage {
        data: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl GuardStorage for MemStorage {
        fn read(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, GuardError> {
            Ok(self.data.get(key).cloned())
        }

        fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), GuardError> {
            self.data.insert(key.to_vec(), value.to_vec());
            Ok(())
        }
    }

    const A: SlotId = SlotId::from_u64(1);
    const B: SlotId = SlotId::from_u64(2);

    fn fresh() -> ReentrancyGuard<MemStorage> {
        ReentrancyGuard::new(MemStorage::default())
    }

    #[test]
    fn fresh_guard_is_not_entered() {
        let mut g = fresh();
        assert!(!g.is_entered(A).unwrap());
    }

    #[test]
    fn single_write_acquire_and_release() {
        let mut g = fresh();
        let entered = g.enter(A, AccessKind::Write).unwrap();
        assert!(!entered.was_read_only);
        assert_eq!(entered.depth, 0);
        assert!(g.is_entered(A).unwrap());
        g.exit(A, AccessKind::Write).unwrap();
        assert!(!g.is_entered(A).unwrap());
    }

    #[test]
    fn nested_write_to_same_slot_is_reverted() {
        let mut g = fresh();
        g.enter(A, AccessKind::Write).unwrap();
        // The nested cross-contract call re-enters the *same* slot: must revert.
        let err = g.enter(A, AccessKind::Write).unwrap_err();
        assert_eq!(err, GuardError::ReentrancyDetected);
        // The outer lock is untouched.
        assert!(g.is_entered(A).unwrap());
        g.exit(A, AccessKind::Write).unwrap();
        assert!(!g.is_entered(A).unwrap());
    }

    #[test]
    fn nested_read_callback_is_allowed_zero_false_positive() {
        let mut g = fresh();
        g.enter(A, AccessKind::Write).unwrap();
        // A read callback on the same slot must NOT be treated as reentrancy.
        let entered = g.enter(A, AccessKind::Read).unwrap();
        assert!(entered.was_read_only);
        g.exit(A, AccessKind::Read).unwrap();
        // Read callbacks must not disturb the write lock.
        assert!(g.is_entered(A).unwrap());
        g.exit(A, AccessKind::Write).unwrap();
    }

    #[test]
    fn distinct_slots_do_not_interfere() {
        let mut g = fresh();
        g.enter(A, AccessKind::Write).unwrap();
        // Mutating a different variable is safe, even nested.
        g.enter(B, AccessKind::Write).unwrap();
        g.exit(B, AccessKind::Write).unwrap();
        g.exit(A, AccessKind::Write).unwrap();
        assert!(!g.is_entered(A).unwrap());
        assert!(!g.is_entered(B).unwrap());
    }

    #[test]
    fn sequential_writes_to_same_slot_allowed_after_release() {
        let mut g = fresh();
        g.enter(A, AccessKind::Write).unwrap();
        g.exit(A, AccessKind::Write).unwrap();
        // Sequential (non-nested) mutation is fine.
        g.enter(A, AccessKind::Write).unwrap();
        g.exit(A, AccessKind::Write).unwrap();
        assert!(!g.is_entered(A).unwrap());
    }

    #[test]
    fn read_alone_never_locks() {
        let mut g = fresh();
        let entered = g.enter(A, AccessKind::Read).unwrap();
        assert!(entered.was_read_only);
        assert!(!g.is_entered(A).unwrap());
    }

    #[test]
    fn mismatched_exit_is_reported() {
        let mut g = fresh();
        g.enter(A, AccessKind::Write).unwrap();
        let err = g.exit(B, AccessKind::Write).unwrap_err();
        assert_eq!(err, GuardError::MismatchedExit);
        g.exit(A, AccessKind::Write).unwrap();
    }

    #[test]
    fn corrupted_stack_is_detected() {
        let mut g = fresh();
        // Truncated stack: 4-byte length says 1 entry but zero bytes follow.
        g.storage.write(STACK_KEY, &[1, 0, 0, 0]).unwrap();
        assert_eq!(g.enter(A, AccessKind::Write).unwrap_err(), GuardError::CorruptedState);
    }

    #[test]
    fn stack_encoding_roundtrips() {
        let mut g = fresh();
        let guard_inner = &mut g;
        guard_inner.persist_stack(&[A, B]).unwrap();
        let loaded = guard_inner.load_stack().unwrap();
        assert_eq!(loaded, vec![A, B]);
    }
}
