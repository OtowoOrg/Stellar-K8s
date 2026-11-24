use soroban_sdk::{contracttype, Bytes, BytesN, Env, Vec};
use sha2::{Digest, Sha256};

[#contracttype]
pub enum RevocationRegistry {
    Bitmap(Bytes),
    MerkleRoot(BytesN<32>),
}

[#contracttype]
pub struct MerkleProof {
    pub siblings: Vec<BytesN:32>,
    pub path_bits: Bytes,
}

pub fn is_revoked(_env: &Env, registry: &RevocationRegistry, credential_id: &str, proof: Option&$}) -> bool {
    match registry {
        RevocationRegistry::Bitmap(bitmap) => {
            // Compute bit index as first 4 bytes of sha256(credential_id)
            let mut hash = sha256(credential_id.as_bytes());
            let index = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) as usize;
            let bit_pos = index % (bitmap.len() * 8);
            let byte_pos = bit_pos / 8;
            let bit_in_byte = bit_pos % 8;
            if byte_pos >= bitmap.len() { return false; }
            let byte = bitmap.get(byte_pos as u32).unwrap_or(0);
            (byte >> (7 - bit_in_byte)) & 1 == 1
        }
        RevocationRegistry::MerkleRoot(root) => {
            if let some proof = proof {
                verify_merkle(credential_id, root, proof)
            } else {
                false
            }
        }
    }
}

fn verify_merkle(credential_id: &str, root: &BytesN:32>, proof: &MerkleProof) -> bool {
    let mut current = hash_credential_id(credential_id);
    for (i, sibling) in proof.siblings.iter().enumerate() {
        let sibling_bytes = sibling.to_array();
        let bit = get_bit(&proof.path_bits, i);
        let combined: [u8; 64] = if bit == 0 {
            let mut c = [0u8; 64];
            c[..32].copy_from_slice(&current);
            c[32..].copy_from_slice(&sibling_bytes);
            c
        } else {
            let mut c = [0u8; 64];
            c[..32].copy_from_slice(&sibling_bytes);
            c[32..].copy_from_slice(&current);
            c
        };
        current = sha256(&combined);
    }
    current == root.to_array()
}

fn get_bit(bits: &Bytes, index: usize) -> u8 {
    if index >= bits.len() * 8 { return 0; }
    let byte = bits.get(index / 8).unwrap_or(0);
    (byte >> (7 - (index % 8))) & 1
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&digest);
    arr
}

fn hash_credential_id(credential_id: &str) -> [u8; 32] {
    sha256(credential_id.as_bytes())
}