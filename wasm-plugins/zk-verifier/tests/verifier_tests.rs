use stellar_zk_verifier::{hash_leaf, hash_node, MerkleProof, ProofStep, verify_merkle_proof, verify_leaf};

#[test]
fn valid_proof_in_four_leaf_tree() {
    let leaf_datas = [b"leaf0".as_slice(), b"leaf1", b"leaf2", b"leaf3"];
    let leaves: Vec<_> = leaf_datas.iter().map(|d| hash_leaf(d)).collect();

    let n01 = hash_node(&leaves[0], &leaves[1]);
    let n23 = hash_node(&leaves[2], &leaves[3]);
    let root = hash_node(&n01, &n23);

    // Proof for leaf0: right sibling leaf1, then right sibling n23
    let proof = MerkleProof {
        steps: vec![
            ProofStep { hash: leaves[1], sibling_is_left: false },
            ProofStep { hash: n23, sibling_is_left: false },
        ],
    };
    assert!(verify_merkle_proof(&proof, &leaves[0], &root));

    // Proof for leaf1: left sibling leaf0, then right sibling n23
    let proof = MerkleProof {
        steps: vec![
            ProofStep { hash: leaves[0], sibling_is_left: true },
            ProofStep { hash: n23, sibling_is_left: false },
        ],
    };
    assert!(verify_merkle_proof(&proof, &leaves[1], &root));

    // Proof for leaf2: right sibling leaf3, then left sibling n01
    let proof = MerkleProof {
        steps: vec![
            ProofStep { hash: leaves[3], sibling_is_left: false },
            ProofStep { hash: n01, sibling_is_left: true },
        ],
    };
    assert!(verify_merkle_proof(&proof, &leaves[2], &root));

    // Proof for leaf3: left sibling leaf2, then left sibling n01
    let proof = MerkleProof {
        steps: vec![
            ProofStep { hash: leaves[2], sibling_is_left: true },
            ProofStep { hash: n01, sibling_is_left: true },
        ],
    };
    assert!(verify_merkle_proof(&proof, &leaves[3], &root));
}

#[test]
fn rejects_tampered_proof() {
    let leaf0 = hash_leaf(b"alice");
    let leaf1 = hash_leaf(b"bob");
    let leaf2 = hash_leaf(b"carol");
    let leaf3 = hash_leaf(b"dave");
    let n01 = hash_node(&leaf0, &leaf1);
    let n23 = hash_node(&leaf2, &leaf3);
    let root = hash_node(&n01, &n23);

    // Forged proof for leaf0: correct first sibling, but wrong second sibling
    let forged = MerkleProof {
        steps: vec![
            ProofStep { hash: leaf1, sibling_is_left: false },
            ProofStep { hash: n01, sibling_is_left: false },
        ],
    };
    assert!(!verify_merkle_proof(&forged, &leaf0, &root));
}

#[test]
fn rejects_leaf_mismatch() {
    let leaf_data = b"state-entry";
    let root = hash_leaf(leaf_data);
    let proof = MerkleProof { steps: vec![] };
    assert!(verify_leaf(&proof, leaf_data, &root));
    assert!(!verify_leaf(&proof, b"tampered-entry", &root));
}