use pqcrypto_kyber::kyber512::*;
fn main() {
    let (pk, _) = keypair();
    let (ct, ss): (Ciphertext, SharedSecret) = encapsulate(&pk);
}
