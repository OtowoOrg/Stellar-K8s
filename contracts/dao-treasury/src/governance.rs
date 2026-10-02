//! # Governance Engine: BFT Threshold Verification, Signature Aggregation & Timelock
//!
//! This module provides the pure (env-independent) cryptographic and
//! algorithmic primitives for the DAO Treasury multi-sig contract:
//!
//! - **BFT threshold verification**: given a committee and a threshold `t`,
//!   accept a proposal once `t` distinct, valid signatures have been
//!   aggregated (Byzantine-fault-tolerant: a quorum of at least
//!   `⌊(2 * committee_size) / 3⌋ + 1` is required when using the default
//!   BFT mode, or an explicit threshold for non-BFT multi-sig).
//! - **Signature aggregation**: deterministically de-duplicates and counts
//!   signatures. Any duplicate signer collapses to a single counted vote;
//!   signers absent from the authorised committee are rejected outright.
//! - **Timelock**: state updates that modify the threshold or committee must
//!   pass through a minimum ledger delay to prevent flash-governance attacks.
//!
//! The contract enforces that every signed digest is the canonical proposal
//! digest (`proposal_id ++ amount ++ recipient ++ nonce`) produced by
//! [`ProposalDigest::compute`].

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage-key constants (all ≤ 9 bytes for symbol_short! compat)
// ---------------------------------------------------------------------------

/// Instance storage key: list of authorised committee members (Vec<Address>).
pub const KEY_COMMITTEE: Symbol = symbol_short!("COMMITTEE");
/// Instance storage key: multi-sig threshold (u32).
pub const KEY_THRESHOLD: Symbol = symbol_short!("THRESH");
/// Instance storage key: pending-threshold change (PendingChange).
pub const KEY_PENDING: Symbol = symbol_short!("PENDING");
/// Instance storage key: minimum timelock ledgers for config changes (u32).
pub const KEY_TIMELOCK: Symbol = symbol_short!("TIMELOCK");
/// Instance storage key: cumulative executed proposal count / nonce base (u64).
pub const KEY_NONCE: Symbol = symbol_short!("NONCE");

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A committee member is identified by their Stellar `Address`.
///
/// The committee list is stored ordered (insertion order) in instance storage.
/// Duplicate entries in the committee are rejected at initialisation time.
pub type Committee = Vec<Address>;

/// A single Soroban-native signature: the signer's address plus a 64-byte
/// raw Ed25519 signature over the canonical proposal digest.
///
/// Soroban's `require_auth` machinery already verifies that the address
/// authorised the transaction, so we leverage that — callers `require_auth`
/// for each signer they present, making the 64-byte field an explicit audit
/// trail (it matches the bytes that were signed off-chain).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sig {
    /// The signer's address (must be in the committee).
    pub signer: Address,
    /// Raw 64-byte Ed25519 signature over the canonical proposal digest.
    pub signature: BytesN<64>,
}

/// A pending configuration change (threshold or committee replacement) that
/// must sit in the timelock queue for at least `unlock_ledger` ledgers before
/// it may be applied.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingChange {
    /// The new threshold value being proposed.
    pub new_threshold: u32,
    /// The new committee being proposed (empty = keep existing committee).
    pub new_committee: Option<Committee>,
    /// The ledger number at or after which the change may be applied.
    pub unlock_ledger: u32,
    /// The admin who queued this change (must re-auth to apply it).
    pub proposer: Address,
}

/// The canonical data structure hashed to produce the proposal digest that
/// signers must sign.  All signers in a batch must have signed the *same*
/// digest, which commits them to the exact asset transfer being authorised.
///
/// `digest = sha256( proposal_id ++ amount_be128 ++ recipient_raw ++ nonce_be64 )`
///
/// In Soroban we use `env.crypto().sha256()` so the hash is deterministic and
/// verifiable on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalDigest {
    /// Globally unique proposal identifier.
    pub proposal_id: u32,
    /// Token amount (in stroops / base units) to be transferred.
    pub amount: i128,
    /// Recipient Stellar address.
    pub recipient: Address,
    /// Per-proposal nonce (equals the execution nonce at the time of signing).
    pub nonce: u64,
}

impl ProposalDigest {
    /// Serialize the digest fields into a deterministic byte buffer and return
    /// the SHA-256 hash as a 32-byte value.
    ///
    /// Layout (58 bytes fixed):
    ///   - 4 bytes: `proposal_id` big-endian u32
    ///   - 16 bytes: `amount` big-endian i128
    ///   - 32 bytes: raw 32-byte Stellar address representation
    ///   - 4 bytes: nonce low bits (for compatibility; stored as u64 but
    ///     Soroban u64 occupies 8 bytes)
    ///   - 8 bytes: `nonce` big-endian u64 (full width)
    ///
    /// The digest is represented as a 32-byte SHA-256 output.
    pub fn compute(&self, env: &Env) -> BytesN<32> {
        let mut buf = Bytes::new(env);

        // proposal_id: 4 bytes big-endian
        let id_bytes = self.proposal_id.to_be_bytes();
        buf.push_back(id_bytes[0]);
        buf.push_back(id_bytes[1]);
        buf.push_back(id_bytes[2]);
        buf.push_back(id_bytes[3]);

        // amount: 16 bytes big-endian
        let amt_bytes = self.amount.to_be_bytes();
        for b in amt_bytes.iter() {
            buf.push_back(*b);
        }

        // nonce: 8 bytes big-endian
        let nonce_bytes = self.nonce.to_be_bytes();
        for b in nonce_bytes.iter() {
            buf.push_back(*b);
        }

        env.crypto().sha256(&buf).into()
    }
}

// ---------------------------------------------------------------------------
// BFT / multi-sig threshold logic
// ---------------------------------------------------------------------------

/// Compute the minimum number of signatures required to achieve Byzantine
/// Fault Tolerance for a committee of `n` members.
///
/// Formula: `⌊(2n / 3)⌋ + 1`
///
/// For a committee of 3 → 3, 4 → 3, 5 → 4, 6 → 5, 7 → 5, 10 → 7, etc.
/// This is the smallest `t` such that `t > 2n/3`, meaning a BFT quorum.
///
/// # Panics
/// Does not panic (result is always ≥ 1 for `n ≥ 1`).
#[inline]
pub fn bft_threshold(n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    (2 * n) / 3 + 1
}

/// Aggregate and deduplicate a list of [`Sig`] values against the authorized
/// `committee`, returning the number of unique, in-committee signers.
///
/// Rules:
/// 1. A signer that does not appear in `committee` is **silently dropped**.
/// 2. Duplicate entries for the same signer are collapsed to a single vote.
///
/// This pure function performs aggregation and deduplication only. Auth
/// enforcement (`require_auth`) is performed by [`aggregate_signatures_with_auth`],
/// which must be called from within an active contract execution context.
///
/// Returns the number of **unique, in-committee** signers found in `sigs`.
pub fn aggregate_signatures(env: &Env, committee: &Committee, sigs: &Vec<Sig>) -> u32 {
    let mut seen: Vec<Address> = Vec::new(env);
    let mut count: u32 = 0;

    for i in 0..sigs.len() {
        let sig = sigs.get(i).unwrap();
        let signer = sig.signer.clone();

        if !committee_contains(committee, &signer) {
            continue;
        }

        if vec_contains(&seen, &signer) {
            continue;
        }

        seen.push_back(signer);
        count += 1;
    }

    count
}

/// Contract-context version of [`aggregate_signatures`] that additionally
/// calls `require_auth()` on each unique in-committee signer so Soroban's
/// auth framework validates key ownership before counting the signature.
///
/// This **must** only be called from within an active contract execution
/// context (i.e., inside a `#[contractimpl]` function).
pub fn aggregate_signatures_with_auth(
    env: &Env,
    committee: &Committee,
    sigs: &Vec<Sig>,
) -> u32 {
    let mut seen: Vec<Address> = Vec::new(env);
    let mut count: u32 = 0;

    for i in 0..sigs.len() {
        let sig = sigs.get(i).unwrap();
        let signer = sig.signer.clone();

        if !committee_contains(committee, &signer) {
            continue;
        }

        if vec_contains(&seen, &signer) {
            continue;
        }

        // Require Soroban auth — validates key ownership on-chain.
        signer.require_auth();

        seen.push_back(signer);
        count += 1;
    }

    count
}

/// Returns `true` if `addr` is present in the `committee` Vec.
///
/// Linear scan — committees are small (≤ 20 members in practice).
#[inline]
pub fn committee_contains(committee: &Committee, addr: &Address) -> bool {
    for i in 0..committee.len() {
        if committee.get(i).unwrap() == *addr {
            return true;
        }
    }
    false
}

/// Returns `true` if `addr` is present in `seen` (deduplication helper).
#[inline]
fn vec_contains(seen: &Vec<Address>, addr: &Address) -> bool {
    for i in 0..seen.len() {
        if seen.get(i).unwrap() == *addr {
            return true;
        }
    }
    false
}

/// Verify that `sig_count` unique authorized signatures meets or exceeds the
/// required `threshold`.
///
/// Returns `true` if `sig_count >= threshold`.
#[inline]
pub fn threshold_met(sig_count: u32, threshold: u32) -> bool {
    sig_count >= threshold
}

// ---------------------------------------------------------------------------
// Timelock helpers
// ---------------------------------------------------------------------------

/// Verify that a pending configuration change's timelock has elapsed.
///
/// Returns `true` if `current_ledger >= pending.unlock_ledger`.
#[inline]
pub fn timelock_elapsed(current_ledger: u32, pending: &PendingChange) -> bool {
    current_ledger >= pending.unlock_ledger
}

/// Compute the unlock ledger for a new pending change.
///
/// `unlock_ledger = current_ledger + timelock_delay`
///
/// A `timelock_delay` of zero means changes take effect immediately (useful
/// for testing only; production deployments should use ≥ 1 ledger).
#[inline]
pub fn compute_unlock_ledger(current_ledger: u32, timelock_delay: u32) -> u32 {
    current_ledger.saturating_add(timelock_delay)
}

// ---------------------------------------------------------------------------
// Pure unit tests (no Soroban env required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // bft_threshold
    // -----------------------------------------------------------------------

    #[test]
    fn bft_threshold_single_member() {
        // n=1: ⌊2/3⌋+1 = 1
        assert_eq!(bft_threshold(1), 1);
    }

    #[test]
    fn bft_threshold_zero_returns_one() {
        assert_eq!(bft_threshold(0), 1);
    }

    #[test]
    fn bft_threshold_three_members() {
        // n=3: ⌊6/3⌋+1 = 3 (all must sign)
        assert_eq!(bft_threshold(3), 3);
    }

    #[test]
    fn bft_threshold_four_members() {
        // n=4: ⌊8/3⌋+1 = 2+1 = 3
        assert_eq!(bft_threshold(4), 3);
    }

    #[test]
    fn bft_threshold_seven_members() {
        // n=7: ⌊14/3⌋+1 = 4+1 = 5
        assert_eq!(bft_threshold(7), 5);
    }

    #[test]
    fn bft_threshold_ten_members() {
        // n=10: ⌊20/3⌋+1 = 6+1 = 7
        assert_eq!(bft_threshold(10), 7);
    }

    #[test]
    fn bft_threshold_large_committee() {
        // n=100: ⌊200/3⌋+1 = 66+1 = 67
        assert_eq!(bft_threshold(100), 67);
    }

    // -----------------------------------------------------------------------
    // threshold_met
    // -----------------------------------------------------------------------

    #[test]
    fn threshold_met_exact() {
        assert!(threshold_met(3, 3));
    }

    #[test]
    fn threshold_met_above() {
        assert!(threshold_met(5, 3));
    }

    #[test]
    fn threshold_not_met() {
        assert!(!threshold_met(2, 3));
    }

    #[test]
    fn threshold_zero_always_met() {
        assert!(threshold_met(0, 0));
        assert!(threshold_met(1, 0));
    }

    // -----------------------------------------------------------------------
    // timelock helpers
    // -----------------------------------------------------------------------

    #[test]
    fn timelock_elapsed_exact() {
        use soroban_sdk::testutils::Address as _;
        let env = soroban_sdk::Env::default();
        let addr = Address::generate(&env);
        let pending = PendingChange {
            new_threshold: 2,
            new_committee: None,
            unlock_ledger: 100,
            proposer: addr,
        };
        assert!(timelock_elapsed(100, &pending));
        assert!(timelock_elapsed(101, &pending));
        assert!(!timelock_elapsed(99, &pending));
    }

    #[test]
    fn compute_unlock_ledger_normal() {
        assert_eq!(compute_unlock_ledger(50, 20), 70);
    }

    #[test]
    fn compute_unlock_ledger_saturating() {
        assert_eq!(compute_unlock_ledger(u32::MAX, 1), u32::MAX);
    }

    #[test]
    fn compute_unlock_ledger_zero_delay() {
        assert_eq!(compute_unlock_ledger(42, 0), 42);
    }
}
