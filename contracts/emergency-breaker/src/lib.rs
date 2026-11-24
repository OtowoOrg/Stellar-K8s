//! # Emergency Circuit Breaker Contract
//!
//! A **M-of-N multi-signature** emergency circuit breaker that allows a quorum
//! of trusted operators to instantly freeze critical infrastructure operations
//! on the Stellar/Soroban blockchain.
//!
//! ## Architecture
//!
//! ```text
//!  ┌───────────────────────────────────────────────────────────────┐
//!  │                   CircuitBreaker Contract                     │
//!  │                                                               │
//!  │  initialize(threshold M, operators[N], timelock_delay)        │
//!  │                                                               │
//!  │  freeze(scope, signatures[M])   → sets FreezeScope bitmask   │
//!  │                                   O(M) sig verify            │
//!  │                                   O(1)  scope write          │
//!  │                                                               │
//!  │  is_frozen(scope)               → O(1) bit-AND lookup        │
//!  │                                                               │
//!  │  unfreeze(signatures[M])        → only after timelock         │
//!  │                                                               │
//!  └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Freeze Lifecycle
//!
//! ```text
//!  ┌──────────┐   freeze()   ┌──────────┐   timelock expires   ┌──────────────┐
//!  │  ACTIVE  │─────────────▶│  FROZEN  │──────────────────────▶│ PENDING_THAW │
//!  └──────────┘              └──────────┘                        └──────────────┘
//!       ▲                                                               │
//!       └───────────────────── unfreeze() ────────────────────────────┘
//! ```
//!
//! ## Pause Check Performance
//!
//! The hot-path call (`is_frozen`) performs a **single bit-AND** on the stored
//! `FreezeScope` bitmask.  This is O(1) regardless of M or N and adds negligible
//! instruction cost to normal transactions.
//!
//! ## Signature Scheme
//!
//! Ed25519 signatures are used (native to Stellar).  Each freeze/unfreeze
//! message is `SHA-256(domain_tag || scope_byte || action_byte)`, making
//! signatures non-replayable across different actions and scopes.

pub mod state;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

pub use state::{BreakerState, FreezeScope, StateStore};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// All error conditions the circuit breaker can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakerError {
    /// Contract has not been initialised yet.
    NotInitialized,
    /// Contract is already initialised.
    AlreadyInitialized,
    /// Threshold M must be ≥ 1 and ≤ N.
    InvalidThreshold,
    /// The operator list must be non-empty.
    EmptyOperatorList,
    /// One of the provided signatures is cryptographically invalid.
    InvalidSignature,
    /// The provided public key is not in the authorised operator list.
    UnauthorizedSigner,
    /// Not enough valid signatures were supplied (need M).
    InsufficientSignatures,
    /// Attempted to freeze when already frozen.
    AlreadyFrozen,
    /// Attempted to unfreeze when not frozen.
    NotFrozen,
    /// Timelock has not yet expired; cannot unfreeze.
    TimelockActive,
    /// The operation requested is currently frozen.
    OperationFrozen,
    /// Duplicate signer detected in the provided signature set.
    DuplicateSigner,
}

// ---------------------------------------------------------------------------
// Signing message helpers
// ---------------------------------------------------------------------------

const DOMAIN_TAG: &[u8] = b"emergency-breaker-v1";
const ACTION_FREEZE: u8 = 0x01;
const ACTION_UNFREEZE: u8 = 0x02;

/// Build the canonical message that operators must sign for a freeze action.
///
/// `SHA-256(DOMAIN_TAG || scope_byte || ACTION_FREEZE)`
fn freeze_message(scope: FreezeScope) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DOMAIN_TAG);
    h.update([scope.0, ACTION_FREEZE]);
    h.finalize().into()
}

/// Build the canonical message that operators must sign for an unfreeze action.
///
/// `SHA-256(DOMAIN_TAG || scope_byte || ACTION_UNFREEZE)`
fn unfreeze_message(scope: FreezeScope) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DOMAIN_TAG);
    h.update([scope.0, ACTION_UNFREEZE]);
    h.finalize().into()
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

/// M-of-N emergency circuit breaker contract.
///
/// # Example
/// ```rust
/// use emergency_breaker::{CircuitBreaker, FreezeScope};
///
/// let mut cb = CircuitBreaker::new();
/// // Initialise with threshold 2, 3 dummy operator keys, 3600 s timelock
/// // (actual key bytes omitted for brevity)
/// ```
pub struct CircuitBreaker {
    store: StateStore,
    initialized: bool,
    /// Timelock duration in seconds applied after each freeze.
    timelock_delay: u64,
}

impl CircuitBreaker {
    /// Create a new, uninitialised circuit breaker.
    pub fn new() -> Self {
        CircuitBreaker {
            store: StateStore::new(),
            initialized: false,
            timelock_delay: 0,
        }
    }

    /// Initialise the contract with `threshold` M, the set of `operators` (Ed25519
    /// verifying keys as 32-byte arrays), and a `timelock_delay` in seconds.
    ///
    /// # Errors
    /// * [`BreakerError::AlreadyInitialized`] — called more than once.
    /// * [`BreakerError::EmptyOperatorList`] — no operators supplied.
    /// * [`BreakerError::InvalidThreshold`] — M < 1 or M > N.
    pub fn initialize(
        &mut self,
        threshold: u8,
        operators: Vec<[u8; 32]>,
        timelock_delay: u64,
    ) -> Result<(), BreakerError> {
        if self.initialized {
            return Err(BreakerError::AlreadyInitialized);
        }
        if operators.is_empty() {
            return Err(BreakerError::EmptyOperatorList);
        }
        if threshold == 0 || threshold as usize > operators.len() {
            return Err(BreakerError::InvalidThreshold);
        }

        self.store.set_threshold(threshold);
        self.store.set_operators(operators);
        self.store.set_freeze_scope(FreezeScope::NONE);
        self.store.set_paused_by(vec![]);
        self.timelock_delay = timelock_delay;
        self.initialized = true;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Hot-path pause check — O(1) single bit-AND
    // -----------------------------------------------------------------------

    /// Check whether a given operation scope is currently frozen.
    ///
    /// This is the **only** method that runs on every transaction.  It performs
    /// a single bit-AND operation — O(1) and minimal instruction cost.
    ///
    /// # Errors
    /// * [`BreakerError::OperationFrozen`] — the requested scope is frozen.
    pub fn assert_not_frozen(&self, op: FreezeScope) -> Result<(), BreakerError> {
        if self.store.is_frozen(op) {
            Err(BreakerError::OperationFrozen)
        } else {
            Ok(())
        }
    }

    /// Returns `true` if `op` is currently frozen.
    #[inline]
    pub fn is_frozen(&self, op: FreezeScope) -> bool {
        self.store.is_frozen(op)
    }

    /// Returns the current [`BreakerState`] given the current Unix timestamp.
    pub fn state(&self, now: u64) -> BreakerState {
        BreakerState::from_store(&self.store, now)
    }

    // -----------------------------------------------------------------------
    // Freeze
    // -----------------------------------------------------------------------

    /// Freeze `scope` operations immediately after verifying M-of-N operator
    /// signatures.
    ///
    /// `sigs` is a slice of `(operator_pubkey, ed25519_signature)` pairs.
    /// Duplicate signers are rejected.  The timelock is set to `now + delay`.
    ///
    /// # Errors
    /// * [`BreakerError::NotInitialized`]
    /// * [`BreakerError::AlreadyFrozen`] — the exact same scope is already frozen.
    /// * [`BreakerError::UnauthorizedSigner`] — a public key is not in the operator list.
    /// * [`BreakerError::DuplicateSigner`] — a public key appears more than once.
    /// * [`BreakerError::InvalidSignature`] — an Ed25519 signature is invalid.
    /// * [`BreakerError::InsufficientSignatures`] — fewer than M valid sigs provided.
    pub fn freeze(
        &mut self,
        scope: FreezeScope,
        sigs: &[([u8; 32], [u8; 64])],
        now: u64,
    ) -> Result<(), BreakerError> {
        if !self.initialized {
            return Err(BreakerError::NotInitialized);
        }
        if self.store.freeze_scope() == scope && scope != FreezeScope::NONE {
            return Err(BreakerError::AlreadyFrozen);
        }

        let msg = freeze_message(scope);
        let verified_keys = self.verify_multisig(sigs, &msg)?;

        // Apply freeze
        self.store.set_freeze_scope(scope);
        self.store.set_unpause_at(now + self.timelock_delay);
        self.store.set_paused_by(verified_keys);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Unfreeze
    // -----------------------------------------------------------------------

    /// Unfreeze all operations after the timelock has elapsed.
    ///
    /// Requires M-of-N operator signatures over the unfreeze message.
    ///
    /// # Errors
    /// * [`BreakerError::NotInitialized`]
    /// * [`BreakerError::NotFrozen`] — nothing is frozen.
    /// * [`BreakerError::TimelockActive`] — timelock has not yet expired.
    /// * signature-related errors (see [`freeze`]).
    pub fn unfreeze(
        &mut self,
        sigs: &[([u8; 32], [u8; 64])],
        now: u64,
    ) -> Result<(), BreakerError> {
        if !self.initialized {
            return Err(BreakerError::NotInitialized);
        }
        if self.store.freeze_scope() == FreezeScope::NONE {
            return Err(BreakerError::NotFrozen);
        }
        if now < self.store.unpause_at() {
            return Err(BreakerError::TimelockActive);
        }

        let scope = self.store.freeze_scope();
        let msg = unfreeze_message(scope);
        self.verify_multisig(sigs, &msg)?;

        // Clear freeze state
        self.store.set_freeze_scope(FreezeScope::NONE);
        self.store.set_unpause_at(0);
        self.store.set_paused_by(vec![]);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal: multi-sig verification
    // -----------------------------------------------------------------------

    /// Verify that at least M of the provided `(pubkey, signature)` pairs are
    /// valid Ed25519 signatures over `msg` from authorised operators.
    ///
    /// Returns the list of verified public keys on success.
    fn verify_multisig(
        &self,
        sigs: &[([u8; 32], [u8; 64])],
        msg: &[u8; 32],
    ) -> Result<Vec<[u8; 32]>, BreakerError> {
        let operators = self.store.operators();
        let threshold = self.store.threshold() as usize;
        let mut seen: Vec<[u8; 32]> = Vec::new();

        for (pk_bytes, sig_bytes) in sigs {
            // Must be an authorised operator
            if !operators.contains(pk_bytes) {
                return Err(BreakerError::UnauthorizedSigner);
            }
            // No duplicates
            if seen.contains(pk_bytes) {
                return Err(BreakerError::DuplicateSigner);
            }
            // Cryptographic verification
            let vk = VerifyingKey::from_bytes(pk_bytes)
                .map_err(|_| BreakerError::InvalidSignature)?;
            let sig = Signature::from_bytes(sig_bytes);
            use ed25519_dalek::Verifier;
            vk.verify(msg, &sig)
                .map_err(|_| BreakerError::InvalidSignature)?;

            seen.push(*pk_bytes);
        }

        if seen.len() < threshold {
            return Err(BreakerError::InsufficientSignatures);
        }
        Ok(seen)
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Generate N Ed25519 keypairs.
    fn gen_keys(n: usize) -> Vec<SigningKey> {
        (0..n).map(|_| SigningKey::generate(&mut OsRng)).collect()
    }

    /// Extract verifying-key bytes from a list of signing keys.
    fn pub_keys(keys: &[SigningKey]) -> Vec<[u8; 32]> {
        keys.iter()
            .map(|sk| sk.verifying_key().to_bytes())
            .collect()
    }

    /// Build M signatures over `msg` using the first M signing keys.
    fn sign_m(
        keys: &[SigningKey],
        msg: &[u8; 32],
        m: usize,
    ) -> Vec<([u8; 32], [u8; 64])> {
        keys[..m]
            .iter()
            .map(|sk| (sk.verifying_key().to_bytes(), sk.sign(msg).to_bytes()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Initialisation tests
    // -----------------------------------------------------------------------

    #[test]
    fn init_success_3_of_5() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        assert!(cb.initialize(3, pub_keys(&keys), 3600).is_ok());
    }

    #[test]
    fn init_rejects_double_init() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();
        cb.initialize(2, pub_keys(&keys), 3600).unwrap();
        assert_eq!(
            cb.initialize(2, pub_keys(&keys), 3600),
            Err(BreakerError::AlreadyInitialized)
        );
    }

    #[test]
    fn init_rejects_zero_threshold() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();
        assert_eq!(
            cb.initialize(0, pub_keys(&keys), 3600),
            Err(BreakerError::InvalidThreshold)
        );
    }

    #[test]
    fn init_rejects_threshold_above_n() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();
        assert_eq!(
            cb.initialize(4, pub_keys(&keys), 3600),
            Err(BreakerError::InvalidThreshold)
        );
    }

    #[test]
    fn init_rejects_empty_operators() {
        let mut cb = CircuitBreaker::new();
        assert_eq!(
            cb.initialize(1, vec![], 3600),
            Err(BreakerError::EmptyOperatorList)
        );
    }

    // -----------------------------------------------------------------------
    // Freeze tests
    // -----------------------------------------------------------------------

    #[test]
    fn freeze_with_m_of_n_signatures() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::ALL);
        let sigs = sign_m(&keys, &msg, 3);
        assert!(cb.freeze(FreezeScope::ALL, &sigs, 1000).is_ok());
        assert!(cb.is_frozen(FreezeScope::ALL));
    }

    #[test]
    fn freeze_rejects_insufficient_signatures() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::DEPOSITS);
        let sigs = sign_m(&keys, &msg, 2); // only 2, need 3
        assert_eq!(
            cb.freeze(FreezeScope::DEPOSITS, &sigs, 1000),
            Err(BreakerError::InsufficientSignatures)
        );
    }

    #[test]
    fn freeze_rejects_unauthorized_signer() {
        let keys = gen_keys(5);
        let intruder = gen_keys(1);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::ALL);
        let mut sigs = sign_m(&keys, &msg, 2);
        // Replace last sig with one from an unauthorised key
        let intruder_sig = intruder[0].sign(&msg).to_bytes();
        sigs.push((intruder[0].verifying_key().to_bytes(), intruder_sig));

        assert_eq!(
            cb.freeze(FreezeScope::ALL, &sigs, 1000),
            Err(BreakerError::UnauthorizedSigner)
        );
    }

    #[test]
    fn freeze_rejects_duplicate_signer() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::ALL);
        let mut sigs = sign_m(&keys, &msg, 2);
        // Duplicate the first signer
        sigs.push(sigs[0].clone());

        assert_eq!(
            cb.freeze(FreezeScope::ALL, &sigs, 1000),
            Err(BreakerError::DuplicateSigner)
        );
    }

    #[test]
    fn freeze_rejects_tampered_signature() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::ALL);
        let mut sigs = sign_m(&keys, &msg, 3);
        // Corrupt the first signature
        sigs[0].1[0] ^= 0xFF;

        assert_eq!(
            cb.freeze(FreezeScope::ALL, &sigs, 1000),
            Err(BreakerError::InvalidSignature)
        );
    }

    // -----------------------------------------------------------------------
    // Granular scope tests
    // -----------------------------------------------------------------------

    #[test]
    fn freeze_deposits_allows_withdrawals() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();
        cb.initialize(2, pub_keys(&keys), 3600).unwrap();

        let msg = freeze_message(FreezeScope::DEPOSITS);
        let sigs = sign_m(&keys, &msg, 2);
        cb.freeze(FreezeScope::DEPOSITS, &sigs, 1000).unwrap();

        // Deposits frozen
        assert_eq!(
            cb.assert_not_frozen(FreezeScope::DEPOSITS),
            Err(BreakerError::OperationFrozen)
        );
        // Withdrawals still permitted
        assert!(cb.assert_not_frozen(FreezeScope::WITHDRAWALS).is_ok());
    }

    // -----------------------------------------------------------------------
    // Unfreeze / timelock tests
    // -----------------------------------------------------------------------

    #[test]
    fn unfreeze_after_timelock() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 3600).unwrap();

        let freeze_msg = freeze_message(FreezeScope::ALL);
        let freeze_sigs = sign_m(&keys, &freeze_msg, 3);
        cb.freeze(FreezeScope::ALL, &freeze_sigs, 1000).unwrap();

        // Timelock: unfreeze before expiry should fail
        let unfreeze_msg = unfreeze_message(FreezeScope::ALL);
        let unfreeze_sigs = sign_m(&keys, &unfreeze_msg, 3);
        assert_eq!(
            cb.unfreeze(&unfreeze_sigs, 1000 + 3599),
            Err(BreakerError::TimelockActive)
        );

        // Unfreeze after expiry
        assert!(cb.unfreeze(&unfreeze_sigs, 1000 + 3600).is_ok());
        assert!(!cb.is_frozen(FreezeScope::ALL));
    }

    #[test]
    fn unfreeze_not_frozen_error() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();
        cb.initialize(2, pub_keys(&keys), 3600).unwrap();

        let msg = unfreeze_message(FreezeScope::ALL);
        let sigs = sign_m(&keys, &msg, 2);
        assert_eq!(cb.unfreeze(&sigs, 9999), Err(BreakerError::NotFrozen));
    }

    // -----------------------------------------------------------------------
    // 3-of-5 high-throughput simulation
    // -----------------------------------------------------------------------

    #[test]
    fn simulate_3_of_5_freeze_all_state_changes_revert() {
        let keys = gen_keys(5);
        let mut cb = CircuitBreaker::new();
        cb.initialize(3, pub_keys(&keys), 7200).unwrap();

        // Simulate high-throughput: 1000 normal calls succeed before freeze
        for _ in 0..1000 {
            assert!(cb.assert_not_frozen(FreezeScope::ALL).is_ok());
        }

        // 3-of-5 sign the freeze
        let msg = freeze_message(FreezeScope::ALL);
        let sigs = sign_m(&keys, &msg, 3);
        cb.freeze(FreezeScope::ALL, &sigs, 5000).unwrap();

        // All subsequent state-changing calls must revert
        for _ in 0..1000 {
            assert_eq!(
                cb.assert_not_frozen(FreezeScope::ALL),
                Err(BreakerError::OperationFrozen)
            );
        }

        // State machine correctly reports Frozen
        assert_eq!(cb.state(5001), BreakerState::Frozen);
        assert_eq!(cb.state(5000 + 7200), BreakerState::PendingThaw);

        // Unfreeze after timelock
        let uf_msg = unfreeze_message(FreezeScope::ALL);
        let uf_sigs = sign_m(&keys, &uf_msg, 3);
        cb.unfreeze(&uf_sigs, 5000 + 7200).unwrap();
        assert_eq!(cb.state(5000 + 7200), BreakerState::Active);
    }

    // -----------------------------------------------------------------------
    // Uninitialised contract guards
    // -----------------------------------------------------------------------

    #[test]
    fn operations_on_uninitialised_contract_fail() {
        let keys = gen_keys(3);
        let mut cb = CircuitBreaker::new();

        let msg = freeze_message(FreezeScope::ALL);
        let sigs = sign_m(&keys, &msg, 2);
        assert_eq!(
            cb.freeze(FreezeScope::ALL, &sigs, 1000),
            Err(BreakerError::NotInitialized)
        );
    }
}
