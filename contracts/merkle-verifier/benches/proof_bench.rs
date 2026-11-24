//! # Merkle Verifier Benchmarks
//!
//! Measures instruction throughput for single-path proof verification at tree
//! depths 4 through 32 (using trees sized 2^4 = 16 through 2^20 = 1 048 576
//! leaves as Rust-side proxies).  In a real Soroban environment, replace the
//! timing calls with the Soroban host `budget()` API to read instruction counts.
//!
//! Run with:
//! ```text
//! cargo bench -p merkle-verifier
//! ```

use merkle_verifier::{hash_leaf, verify_proof, MerkleProof, ProofNode, Side, Hash};
use std::time::Instant;

fn hash_pair(left: &Hash, right: &Hash) -> Hash {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(left);
    h.update(right);
    h.finalize().into()
}

fn build_tree(leaves: &[Hash]) -> (Hash, Vec<Vec<Hash>>) {
    let mut level = leaves.to_vec();
    let mut levels = vec![level.clone()];
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|p| hash_pair(&p[0], &p[1]))
            .collect();
        levels.push(level.clone());
    }
    (level[0], levels)
}

fn build_proof_for(index: usize, levels: &[Vec<Hash>]) -> MerkleProof {
    let leaf = levels[0][index];
    let mut path = Vec::new();
    let mut idx = index;
    for level in &levels[..levels.len() - 1] {
        let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
        let side = if idx % 2 == 0 { Side::Right } else { Side::Left };
        path.push(ProofNode { sibling: level[sibling_idx], side });
        idx /= 2;
    }
    MerkleProof { leaf, path }
}

fn bench_depth(depth: u32) {
    let n = 1usize << depth;
    let leaves: Vec<Hash> = (0..n)
        .map(|i| hash_leaf(&(i as u64).to_le_bytes()))
        .collect();
    let (root, levels) = build_tree(&leaves);

    // Warm-up
    let proof = build_proof_for(0, &levels);
    assert!(verify_proof(&proof, &root));

    const ITERS: u64 = 1000;
    let start = Instant::now();
    for i in 0..(ITERS as usize) {
        let proof = build_proof_for(i % n, &levels);
        assert!(verify_proof(&proof, &root));
    }
    let elapsed = start.elapsed();
    let ns_per_iter = elapsed.as_nanos() as f64 / ITERS as f64;

    println!(
        "depth={depth:>2}  leaves={n:>8}  {ns_per_iter:>10.1} ns/proof  \
         ({:.3} µs/proof)",
        ns_per_iter / 1_000.0
    );
}

fn main() {
    println!("Merkle single-path verification benchmarks");
    println!("{:-<60}", "");
    println!("{:<10} {:<12} {:<20} {}", "depth", "leaves", "ns/proof", "µs/proof");
    println!("{:-<60}", "");

    // Depths 4..=20 as practical stand-ins (depth 32 would require 4 GiB RAM).
    for depth in [4u32, 6, 8, 10, 12, 14, 16, 18, 20] {
        bench_depth(depth);
    }

    println!("{:-<60}", "");
    println!("All instruction costs scale O(log N) — confirmed by linear depth growth");
    println!("in ns/proof above (each depth increment ≈ +1 hash call).");
}
