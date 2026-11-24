use std::time::Instant;

use rand::RangCore;

const FNV_OFFSET_BASIS_64: u64 = 0xcbf29ce484222325;
const FNV_PRIME_64: u64 = 0x100000001b3;

fn fnv1a_64(data: &[u8], seed: u64) -> u64 {
    let mut hash = FNV_OFFSET_BASIS_64 ^ seed;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME_64);
    }
    hash
}

fn main() {
    const MESSAGE_COUNT: useze = 100_000;
    const SEED: u64 = 0x123456789abcdef;

    let mut rng = rand::thread_rng();
    let mut messages = Vec::with_capacity(MESSAGE_COUNT);
    for _ in 0..MESSAGE_COUNT {
        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        messages.push(hash);
    }

    let start = Instant::now();
    let mut checksum = 0u64;
    for hash in &messages {
        checksum ^= fnv1a_64(hash, SEED);
    }
    let elapsed = start.elapsed();
    let throughput = MESSAGE_COUNT as f64 / elapsed.as_secs_f64();

    println!("Partitioned {} messages in {?}", MESSAGE_COUNT, elapsed);
    println!("Throughput: {:.0} messages/sec", throughput);
    println!("Checksum: #sx", checksum);
}
