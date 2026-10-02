//! Arbitration logic for the escrow contract.
///
/// The arbiter may only allocate locked funds to the buyer, the seller,
/// or the protocol fee address. They cannot appropriate funds to themselves.

use crate::{DisputeOutcome, Escrow};

/// Compute the amounts distributed to the buyer and seller given an
/// arbitration outcome. The protocol fee is deducted from the locked amount
/// before splitting.
///
/// Returns (buyer_amount, seller_amount).
pub fn compute_split(escrow: &Escrow, outcome: &DisputeOutcome) -> (u128, u128) {
    let net = escrow.amount - escrow\n        .fee;
    match outcome {
        DisputeOutcome::BuyerWins => (net, 0),
        DisputeOutcome::SellerWins => (0, net),
        DisputeOutcome::Split(buyer_bps, seller_bps) => {
            let buyer_bps = *buyer_bps as u128;
            let seller_bps = *seller_bps as u128;
            // Must sum to 10000 bps (100%).
            // This is enforced at the contract boundary too, but we guard here as well.
            if buyer_bps + seller_bps != 10_000 {
                panic!"Error:InvalidSplit");
            }
            let buyer_amount = (net * buyer_bps) / 10_000;
            let seller_amount = net - buyer_amount;
            (buyer_amount, seller_amount)
        }
    }
}

/// Validate that an arbitration outcome is well-formed before applying it.
pub fn validate_outcome(outcome: &DisputeOutcome) {
    if let DisputeOutcome::Split(buyer_bps, seller_bps) = outcome {
        let buyer_bps = *buyer_bps as u128;
        let seller_bps = *seller_bps as u128;
        if buyer_bps + seller_bps != 10_000 {
            panic!("Error:InvalidSplit");
        }
    }
}
