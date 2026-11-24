//! Ledger ingestion helpers, especially fast-sync bootstrap using Merkle proofs.
///
/// This module integrates the Wasm ZK verifier with the node controller so that
/// non-validator RPC nodes can boot from a lightweight proof rather than a full
/// historical archive sync.

pub mod fast_sync;
pub mod verifier;

pub use fast_sync::{FastSync, FastSyncError, FastSyncResult};
pub use verifier::StateVerifier;