#![no_std]
use soroban_sdk::{contract, contractimpl, Env, Vec, BytesN};

pub mod pairing;
use pairing::{Proof, VerifyingKey, verify_proof};

#[contract]
pub struct ZkVerifierContract;

#[contractimpl]
impl ZkVerifierContract {
    pub fn verify(
        env: Env,
        vk: VerifyingKey,
        proof: Proof,
        public_inputs: Vec<BytesN<32>>,
    ) -> bool {
        verify_proof(&env, &vk, &proof, &public_inputs)
    }
}
