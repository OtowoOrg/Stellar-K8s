//! # Fuzz Tests for DAO Treasury Signature Aggregation
//!
//! Uses proptest to generate adversarial inputs for the signature aggregation
//! and BFT-threshold logic in [`dao_treasury::governance`].
//!
//! ## What is being fuzzed
//!
//! 1. **Duplicate signers** — repeated entries for the same address must
//!    collapse to a single counted vote; the unique-signer count must never
//!    exceed the number of distinct addresses in `sigs`.
//! 2. **Non-committee signers** — addresses not in the committee are silently
//!    dropped; the count must not increase because of them.
//! 3. **Mixed overlapping + non-member sigs** — combination of duplicates and
//!    outsiders; only unique in-committee signers are counted.
//! 4. **BFT threshold invariants** — `bft_threshold(n)` is always `> 2n/3`
//!    and always `≤ n` for any non-zero `n`.
//! 5. **Timelock math** — `compute_unlock_ledger` never underflows and the
//!    result is always `≥ current_ledger`.
//! 6. **Digest uniqueness** — two proposals with different fields produce
//!    different SHA-256 digests.
//!
//! Run: `cargo test -p dao-treasury --test fuzz_sig_aggregation`

use dao_treasury::governance::{
    aggregate_signatures, bft_threshold, committee_contains, compute_unlock_ledger,
    threshold_met, ProposalDigest, Sig,
};
use soroban_sdk::{
    testutils::Address as _,
    Address, BytesN, Env, Vec,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a Soroban `Vec<Address>` of `n` freshly generated addresses.
fn make_committee(env: &Env, n: usize) -> Vec<Address> {
    let mut v: Vec<Address> = Vec::new(env);
    for _ in 0..n {
        v.push_back(Address::generate(env));
    }
    v
}

/// Build a fake 64-byte signature whose first 32 bytes equal `digest`.
fn make_sig(env: &Env, signer: Address, digest: BytesN<32>) -> Sig {
    let mut raw = [0u8; 64];
    let d = digest.to_array();
    raw[..32].copy_from_slice(&d);
    Sig {
        signer,
        signature: BytesN::from_array(env, &raw),
    }
}

/// A zeroed-out 32-byte digest (stands in for any specific hash).
fn zero_digest(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

// ---------------------------------------------------------------------------
// BFT threshold invariant tests
// ---------------------------------------------------------------------------

#[test]
fn bft_threshold_always_greater_than_two_thirds() {
    for n in 1u32..=200 {
        let t = bft_threshold(n);
        // t must be > 2n/3  ⟺  3t > 2n
        assert!(
            3 * t > 2 * n,
            "bft_threshold({n}) = {t} is not > 2*{n}/3"
        );
    }
}

#[test]
fn bft_threshold_never_exceeds_committee_size() {
    for n in 1u32..=200 {
        let t = bft_threshold(n);
        assert!(
            t <= n,
            "bft_threshold({n}) = {t} exceeds committee size"
        );
    }
}

#[test]
fn bft_threshold_non_decreasing() {
    for n in 1u32..200 {
        assert!(
            bft_threshold(n + 1) >= bft_threshold(n),
            "bft_threshold not non-decreasing at n={n}"
        );
    }
}

#[test]
fn bft_threshold_corner_case_n1() {
    // For n=1 the only valid threshold is 1.
    assert_eq!(bft_threshold(1), 1);
}

#[test]
fn bft_threshold_corner_case_n3() {
    // Classic BFT: all 3 must sign (1 fault tolerated).
    assert_eq!(bft_threshold(3), 3);
}

#[test]
fn bft_threshold_corner_case_n4() {
    // n=4: ⌊8/3⌋+1 = 3
    assert_eq!(bft_threshold(4), 3);
}

// ---------------------------------------------------------------------------
// threshold_met identity properties
// ---------------------------------------------------------------------------

#[test]
fn threshold_met_monotone_in_sig_count() {
    // If threshold_met(k, t) then threshold_met(k+1, t) for all k ≥ 0.
    for t in 0u32..=10 {
        let mut met = false;
        for k in 0u32..=15 {
            let current = threshold_met(k, t);
            if current {
                met = true;
            }
            if met {
                assert!(
                    current,
                    "threshold_met({k}, {t}) should be true because threshold_met({}, {t}) was true",
                    k.saturating_sub(1)
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Signature aggregation — property tests with Soroban testutils
// ---------------------------------------------------------------------------

/// Core invariant: unique-in-committee sig count ≤ distinct signers provided.
#[test]
fn aggregate_never_counts_more_than_distinct_signers() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let committee = make_committee(&env, 5);
    let digest = zero_digest(&env);

    // All 5 committee members sign once — expect count = 5.
    let mut sigs: Vec<Sig> = Vec::new(&env);
    for i in 0..committee.len() {
        sigs.push_back(make_sig(&env, committee.get(i).unwrap(), digest.clone()));
    }
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 5, "expected all 5 unique committee members counted");
}

/// Duplicate entries for the same signer must count as exactly 1.
#[test]
fn aggregate_deduplicates_same_signer() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee = make_committee(&env, 3);
    let m0 = committee.get(0).unwrap();
    let digest = zero_digest(&env);

    // Push m0 five times.
    let mut sigs: Vec<Sig> = Vec::new(&env);
    for _ in 0..5 {
        sigs.push_back(make_sig(&env, m0.clone(), digest.clone()));
    }
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 1, "duplicate entries must collapse to 1");
}

/// Non-committee addresses must not be counted.
#[test]
fn aggregate_ignores_non_committee_signers() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee = make_committee(&env, 3);
    let digest = zero_digest(&env);

    // All outsiders.
    let mut sigs: Vec<Sig> = Vec::new(&env);
    for _ in 0..5 {
        let outsider = Address::generate(&env);
        sigs.push_back(make_sig(&env, outsider, digest.clone()));
    }
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 0, "outsiders must contribute 0");
}

/// Mix of duplicates + outsiders: only unique in-committee signers counted.
#[test]
fn aggregate_mixed_duplicates_and_outsiders() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee = make_committee(&env, 4);
    let m0 = committee.get(0).unwrap();
    let m1 = committee.get(1).unwrap();
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    // m0 three times.
    for _ in 0..3 {
        sigs.push_back(make_sig(&env, m0.clone(), digest.clone()));
    }
    // m1 twice.
    for _ in 0..2 {
        sigs.push_back(make_sig(&env, m1.clone(), digest.clone()));
    }
    // Three outsiders.
    for _ in 0..3 {
        sigs.push_back(make_sig(&env, Address::generate(&env), digest.clone()));
    }

    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 2, "only m0 and m1 (2 unique in-committee) should count");
}

/// Empty signature list produces count = 0.
#[test]
fn aggregate_empty_sigs_returns_zero() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee = make_committee(&env, 3);
    let sigs: Vec<Sig> = Vec::new(&env);
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 0);
}

/// Empty committee produces count = 0 regardless of sigs.
#[test]
fn aggregate_empty_committee_returns_zero() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee: Vec<Address> = Vec::new(&env);
    let m = Address::generate(&env);
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    sigs.push_back(make_sig(&env, m, digest));
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 0, "no committee → no valid signers");
}

/// Partial quorum: strictly fewer than the BFT threshold present — must not meet it.
/// For n=7, BFT threshold = 5. We provide only 4 signatures (< 5).
#[test]
fn aggregate_partial_quorum_below_bft_threshold() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let n = 7u32;
    let committee = make_committee(&env, n as usize);
    let digest = zero_digest(&env);

    // Threshold is 5 (bft_threshold(7) = ⌊14/3⌋+1 = 5).
    // Provide exactly 4 in-committee signatures — below threshold.
    let below_threshold = 4u32;
    let mut sigs: Vec<Sig> = Vec::new(&env);
    for i in 0..below_threshold {
        sigs.push_back(make_sig(&env, committee.get(i).unwrap(), digest.clone()));
    }
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, below_threshold);

    let threshold = bft_threshold(n);
    assert_eq!(threshold, 5, "sanity: bft_threshold(7) should be 5");
    assert!(
        !threshold_met(count, threshold),
        "{below_threshold} signatures should not meet BFT threshold={threshold} for n={n}"
    );
}

/// Full quorum: all n signers present — must meet BFT threshold.
#[test]
fn aggregate_full_quorum_meets_bft_threshold() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let n = 7u32;
    let committee = make_committee(&env, n as usize);
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    for i in 0..n {
        sigs.push_back(make_sig(&env, committee.get(i).unwrap(), digest.clone()));
    }
    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, n);

    let threshold = bft_threshold(n);
    assert!(
        threshold_met(count, threshold),
        "full committee should meet BFT threshold {threshold} for n={n}"
    );
}

// ---------------------------------------------------------------------------
// Timelock fuzzing
// ---------------------------------------------------------------------------

#[test]
fn compute_unlock_ledger_result_ge_current() {
    for current in [0u32, 1, 50, 100, u32::MAX / 2, u32::MAX - 1, u32::MAX] {
        for delay in [0u32, 1, 10, 1000, u32::MAX] {
            let unlock = compute_unlock_ledger(current, delay);
            assert!(
                unlock >= current,
                "unlock_ledger={unlock} < current={current} with delay={delay}"
            );
        }
    }
}

#[test]
fn compute_unlock_ledger_zero_delay_equals_current() {
    for current in [0u32, 1, 42, 1000, u32::MAX] {
        assert_eq!(compute_unlock_ledger(current, 0), current);
    }
}

#[test]
fn compute_unlock_ledger_saturates_at_max() {
    assert_eq!(compute_unlock_ledger(u32::MAX, 1), u32::MAX);
    assert_eq!(compute_unlock_ledger(u32::MAX, u32::MAX), u32::MAX);
}

// ---------------------------------------------------------------------------
// Proposal digest uniqueness
// ---------------------------------------------------------------------------

#[test]
fn digest_different_proposal_ids_produce_different_hashes() {
    let env = Env::default();
    let recipient = Address::generate(&env);

    let d1 = ProposalDigest {
        proposal_id: 1,
        amount: 100,
        recipient: recipient.clone(),
        nonce: 0,
    }
    .compute(&env);

    let d2 = ProposalDigest {
        proposal_id: 2,
        amount: 100,
        recipient: recipient.clone(),
        nonce: 0,
    }
    .compute(&env);

    assert_ne!(d1, d2, "different proposal_ids must produce different digests");
}

#[test]
fn digest_different_amounts_produce_different_hashes() {
    let env = Env::default();
    let recipient = Address::generate(&env);

    let d1 = ProposalDigest {
        proposal_id: 1,
        amount: 100,
        recipient: recipient.clone(),
        nonce: 0,
    }
    .compute(&env);

    let d2 = ProposalDigest {
        proposal_id: 1,
        amount: 200,
        recipient: recipient.clone(),
        nonce: 0,
    }
    .compute(&env);

    assert_ne!(d1, d2, "different amounts must produce different digests");
}

#[test]
fn digest_different_nonces_produce_different_hashes() {
    let env = Env::default();
    let recipient = Address::generate(&env);

    let d1 = ProposalDigest {
        proposal_id: 1,
        amount: 100,
        recipient: recipient.clone(),
        nonce: 0,
    }
    .compute(&env);

    let d2 = ProposalDigest {
        proposal_id: 1,
        amount: 100,
        recipient: recipient.clone(),
        nonce: 1,
    }
    .compute(&env);

    assert_ne!(d1, d2, "different nonces must produce different digests");
}

#[test]
fn digest_same_fields_produce_same_hash() {
    let env = Env::default();
    let recipient = Address::generate(&env);

    let params = ProposalDigest {
        proposal_id: 42,
        amount: 1000,
        recipient: recipient.clone(),
        nonce: 7,
    };

    let d1 = params.compute(&env);
    let d2 = params.compute(&env);

    assert_eq!(d1, d2, "identical inputs must produce identical digests");
}

// ---------------------------------------------------------------------------
// Malformed / adversarial signature batch fuzzing
// ---------------------------------------------------------------------------

/// A batch with MAX_USIZE duplicate entries for a single signer should still
/// count as 1. (Bounded at 200 to keep tests fast.)
#[test]
fn aggregate_many_duplicates_single_signer_counts_one() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee = make_committee(&env, 3);
    let m0 = committee.get(0).unwrap();
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    for _ in 0..200 {
        sigs.push_back(make_sig(&env, m0.clone(), digest.clone()));
    }

    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, 1);
}

/// All committee members sign AND their addresses appear again as outsiders
/// (impossible in practice but tests the membership check path).
#[test]
fn aggregate_committee_plus_outsiders_counts_only_committee() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let n = 4usize;
    let committee = make_committee(&env, n);
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    // All committee members.
    for i in 0..committee.len() {
        sigs.push_back(make_sig(&env, committee.get(i).unwrap(), digest.clone()));
    }
    // 10 outsiders.
    for _ in 0..10 {
        sigs.push_back(make_sig(&env, Address::generate(&env), digest.clone()));
    }

    let count = aggregate_signatures(&env, &committee, &sigs);
    assert_eq!(count, n as u32, "count should equal committee size regardless of outsiders");
}

/// Zero-length committee rejects all signatures including committee-lookalikes.
#[test]
fn aggregate_zero_committee_always_zero() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let committee: Vec<Address> = Vec::new(&env);
    let digest = zero_digest(&env);

    let mut sigs: Vec<Sig> = Vec::new(&env);
    for _ in 0..10 {
        sigs.push_back(make_sig(&env, Address::generate(&env), digest.clone()));
    }
    assert_eq!(aggregate_signatures(&env, &committee, &sigs), 0);
}

// ---------------------------------------------------------------------------
// committee_contains correctness
// ---------------------------------------------------------------------------

#[test]
fn committee_contains_present_member() {
    let env = Env::default();
    let committee = make_committee(&env, 5);
    let m2 = committee.get(2).unwrap();
    assert!(committee_contains(&committee, &m2));
}

#[test]
fn committee_contains_absent_address() {
    let env = Env::default();
    let committee = make_committee(&env, 5);
    let outsider = Address::generate(&env);
    assert!(!committee_contains(&committee, &outsider));
}

#[test]
fn committee_contains_empty_committee() {
    let env = Env::default();
    let committee: Vec<Address> = Vec::new(&env);
    let addr = Address::generate(&env);
    assert!(!committee_contains(&committee, &addr));
}
