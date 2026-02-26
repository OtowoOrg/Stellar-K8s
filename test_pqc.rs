use pqcrypto_kyber::kyber512::*;
use pqcrypto_dilithium::dilithium2::*;

fn main() {
    let (pk, sk) = keypair();
    let msg = b"hello";
    let sig = detached_sign(msg, &sk);
    let ok = verify_detached_signature(&sig, msg, &pk).is_ok();
    println!("dilithium ok: {}", ok);

    let (k_pk, k_sk) = pqcrypto_kyber::kyber512::keypair();
    let (ct, ss) = encapsulate(&k_pk);
    let ss2 = decapsulate(&ct, &k_sk);
    println!("kyber ok: {}", ss == ss2);
}
