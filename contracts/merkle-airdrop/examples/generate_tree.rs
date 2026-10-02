//! Off-chain tree generator: turn a list of recipients into the single root a campaign
//! commits on-chain, plus the proofs its recipients hand out.
//!
//! ```text
//! cargo run --release --example generate_tree                  # 100 000 recipients
//! cargo run --release --example generate_tree -- 250000        # a bigger campaign
//! cargo run --release --example generate_tree -- 100000 4242   # also print one proof
//! ```
//!
//! The root printed here is the value passed to `initialize` (or `set_merkle_root`), and
//! each proof is the `proof` argument of `claim`. Because the preimages come from
//! `merkle_airdrop::claim` — the same encoder the contract verifies against — the tree this
//! prints cannot drift from the tree the contract accepts.
//!
//! Recipients here are *synthetic but well-formed*: the same arithmetic derivation the
//! tests and benchmark use, so the output validates against the contract without needing
//! a wallet, a network or a CSV. Swap `recipient_xdr` for real `Address::to_xdr` bytes to
//! run a live campaign.

use std::env;
use std::process::ExitCode;

#[path = "../src/offchain.rs"]
mod offchain;

use merkle_airdrop::claim::{MAX_LEAF_COUNT, MAX_PROOF_DEPTH};
use offchain::{build_tree, recipient_xdr, to_hex};

/// One whole token, in stroops (7 decimals), for every synthetic recipient.
const ALLOCATION: i128 = 10_000_000;

/// Salt, so the example's recipients differ from the test fixtures'.
const SALT: u32 = 0xCA_11;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let recipients: u32 = match args.first().map(|s| s.parse()) {
        Some(Ok(n)) => n,
        Some(Err(_)) => {
            eprintln!("usage: generate_tree [recipient_count] [proof_index]");
            return ExitCode::FAILURE;
        }
        None => 100_000,
    };
    let proof_index: u32 = match args.get(1).map(|s| s.parse()) {
        Some(Ok(i)) => i,
        Some(Err(_)) => {
            eprintln!("proof_index must be an integer");
            return ExitCode::FAILURE;
        }
        None => 0,
    };

    if recipients == 0 || recipients > MAX_LEAF_COUNT {
        eprintln!(
            "recipient_count must be between 1 and {MAX_LEAF_COUNT} \
             ({MAX_PROOF_DEPTH} levels of proof is the deepest the contract verifies)"
        );
        return ExitCode::FAILURE;
    }
    if proof_index >= recipients {
        eprintln!("proof_index {proof_index} is outside 0..{recipients}");
        return ExitCode::FAILURE;
    }

    println!("building a {recipients}-recipient distribution off-chain...");

    // The only part that scales with the campaign. Everything is native SHA-256 and plain
    // arithmetic — no host, no network, no contract.
    let accounts: Vec<Vec<u8>> = (0..recipients).map(|i| recipient_xdr(i, SALT)).collect();
    let amounts = vec![ALLOCATION; recipients as usize];
    let tree = build_tree(&accounts, &amounts);

    let total: i128 = ALLOCATION * i128::from(recipients);
    let depth = tree.depth();

    println!();
    println!("  recipients ............. {recipients}");
    println!("  tree leaves (padded) ... {}", tree.leaves.len());
    println!("  proof depth ............ {depth} siblings per claim");
    println!(
        "  allocation per claim ... {:.7} units",
        ALLOCATION as f64 / 1e7
    );
    println!("  total to fund .......... {:.7} units", total as f64 / 1e7);
    println!("  merkle root ............ {}", to_hex(&tree.root()));
    println!();
    println!("commit it with:");
    println!("  initialize(admin, token, root, {recipients}, {total}, claim_deadline)");
    println!("and fund the distributor with at least {total} units of the token.");
    println!();
    println!("proof for recipient {proof_index} of {recipients}:");
    println!("  claim(recipient, {proof_index}, {ALLOCATION}, proof)   # {depth} siblings");
    for (level, sibling) in tree.proof(proof_index as usize).iter().enumerate() {
        println!("    [{level:>2}] {}", to_hex(sibling));
    }

    ExitCode::SUCCESS
}
