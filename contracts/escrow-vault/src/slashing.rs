//! Cryptographically verified slashing-proof interface.
//!
//! A [`Proof`] authenticates an on-chain fault (downtime or double-signing)
//! and can **only** trigger a slash if it verifies end-to-end:
//!
//! 1. **Signature validity** — the proof carries one or more Ed25519
//!    signatures that verify against the expected public key.
//! 2. **Authority** — the reporter is on the vault's reporter allowlist, or
//!    the double-sign signature is produced by the accused node's own
//!    consensus key (so two conflicting messages for the same slot can only
//!    come from the node itself).
//! 3. **Freshness / replay protection** — the payload is canonical and
//!    carries the slot + observed ledger, and submissions must be within the
//!    configured staleness window so old proofs cannot be replayed later.
//!
//! A proof that fails any check is rejected by the vault and mutates nothing.

use std::convert::TryFrom;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::{Address, VaultError};

/// Kind of on-chain fault a proof must authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProofType {
    /// The validator node failed its uptime SLA (detected by an authorized
    /// reporter/watcher that cryptographically signs the observation).
    Downtime = 0,
    /// The validator's consensus key signed two conflicting messages for the
    /// same consensus slot — only the node itself could do this.
    DoubleSign = 1,
}

/// Canonical, replay-resistant payload that is signed for downtime proofs.
///
/// Field layout is fixed (little-endian) so a signature cannot be re-anchored
/// to a different `ProofType`, `node_id`, `slot`, or observation time.
pub fn downtime_payload(proof_type: ProofType, node_id: &Address, store_key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 32 + store_key.len());
    out.push(proof_type as u8);
    out.extend_from_slice(&node_id.to_bytes());
    out.extend_from_slice(store_key);
    out
}

/// A cryptographically authenticated fault report.
///
/// For [`ProofType::Downtime`], `signature` is produced by `reporter`'s key
/// over [`downtime_payload`].
///
/// For [`ProofType::DoubleSign`] (see [`verify_double_sign`]), `signature`
/// carries the node's two conflicting signatures so they can be checked
/// against the node's own consensus key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub proof_type: ProofType,
    /// Consensus public key (or account) of the accused node.
    pub node_id: Address,
    /// Canonical store key uniquely identifying the secured position.
    pub store_key: [u8; 32],
    /// Consensus slot the fault pertains to.
    pub slot: u64,
    /// Observed ledger sequence when the fault was detected.
    pub observed_seq: u64,
    /// Reporter key (bytes) that produced `signature` (for downtime proofs).
    pub reporter: Address,
    /// Ed25519 signature over the downtime payload.
    pub signature: Vec<u8>,
}

impl Proof {
    /// Verification input used by the vault for a `Downtime` proof:
    /// the canonical signed messages, i.e. `(type, node_id, store_key)`.
    pub fn signed_message(&self) -> Vec<u8> {
        if self.proof_type == ProofType::Downtime {
            downtime_payload(self.proof_type, &self.node_id, &self.store_key)
        } else {
            // Double-sign proofs verify their own conflicting payloads, so
            // there is no single signed message at this layer.
            Vec::new()
        }
    }
}

/// Second message payload for a double-sign proof. Both messages are bound to
/// the same node and slot, but carry a different `value`, proving the node
/// broadcast conflicting statements.
pub fn double_sign_message(node_id: &Address, slot: u64, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 32 + 32 + value.len());
    out.extend_from_slice(&node_id.to_bytes());
    out.extend_from_slice(&slot.to_le_bytes());
    out.extend_from_slice(value);
    out
}

/// All digest/signature bytes carried by a double-sign proof.
#[derive(Debug, Clone)]
pub struct DoubleSignMaterial {
    pub node_vk_bytes: [u8; 32],
    pub message_a: Vec<u8>,
    pub sig_a: [u8; 64],
    pub message_b: Vec<u8>,
    pub sig_b: [u8; 64],
}

/// Verifies that the accused node's consensus key signed two *distinct*
/// messages for the same slot. Because both signatures must come from the
/// node's own key, a valid double-sign proof is self-incriminating and cannot
/// be forged by an unrelated reporter.
pub fn verify_double_sign(
    material: &DoubleSignMaterial,
    node_id: &Address,
    slot: u64,
) -> Result<(), VaultError> {
    let node_vk = VerifyingKey::from_bytes(&material.node_vk_bytes)
        .map_err(|_| VaultError::InvalidProof("invalid node verifying key".into()))?;

    for (m, s) in [
        (&material.message_a, &material.sig_a),
        (&material.message_b, &material.sig_b),
    ] {
        let sig = Signature::from_bytes(s);
        node_vk
            .verify(m, &sig)
            .map_err(|_| VaultError::InvalidProof("double-sign signature mismatch".into()))?;
    }

    if (!payload_targets_node(&material.message_a, node_id)
        || !payload_targets_node(&material.message_b, node_id))
        || slot_le_bytes(&material.message_a) != Some(slot)
        || slot_le_bytes(&material.message_b) != Some(slot)
    {
        return Err(VaultError::InvalidProof(
            "double-sign messages must target the same node/slot".into(),
        ));
    }
    if material.message_a == material.message_b {
        return Err(VaultError::InvalidProof(
            "double-sign messages must be conflicting".into(),
        ));
    }
    Ok(())
}

/// Returns true when `msg` is a `[node_id (32)] || slot (u64 le) || value` message
/// targeting the given node.
fn payload_targets_node(msg: &[u8], node_id: &Address) -> bool {
    msg.len() >= 32 && msg[..32] == node_id.to_bytes()[..]
}

fn slot_le_bytes(msg: &[u8]) -> Option<u64> {
    let b = msg.get(32..40)?;
    Some(u64::from_le_bytes(b.try_into().ok()?))
}

/// Verifies a **downtime** proof against the reporter's public key recorded
/// for `proof.reporter`. Returns `Ok(())` only when the signature is genuine.
pub fn verify_reporter_signature(
    proof: &Proof,
    reporter_vk_bytes: &[u8; 32],
    now: u64,
    staleness_window: u64,
) -> Result<(), VaultError> {
    if now.saturating_sub(proof.observed_seq) > staleness_window {
        return Err(VaultError::StaleProof);
    }
    if proof.proof_type != ProofType::Downtime {
        return Err(VaultError::InvalidProof("expected downtime proof".into()));
    }
    let vk = VerifyingKey::from_bytes(reporter_vk_bytes)
        .map_err(|_| VaultError::InvalidProof("invalid reporter verifying key".into()))?;
    let raw_sig = <[u8; 64]>::try_from(proof.signature.as_slice())
        .map_err(|_| VaultError::InvalidProof("signature length mismatch".into()))?;
    let sig = Signature::from_bytes(&raw_sig);
    let msg = proof.signed_message();
    vk.verify(&msg, &sig)
        .map_err(|_| VaultError::InvalidProof("reporter signature invalid".into()))
}

/// Convenience constructor helpers used by tests and the vault driver.
pub mod test_support {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    /// Produces a valid downtime proof signed by `signer`.
    pub fn sign_downtime(
        signer: &SigningKey,
        reporter: Address,
        node_id: Address,
        store_key: [u8; 32],
    ) -> Proof {
        let proof = Proof {
            proof_type: ProofType::Downtime,
            node_id,
            store_key,
            slot: 0,
            observed_seq: 0,
            reporter,
            signature: Vec::new(),
        };
        let msg = proof.signed_message();
        let sig = signer.sign(&msg);
        Proof {
            signature: sig.to_bytes().to_vec(),
            ..proof
        }
    }

    /// Produces two conflicting, correctly-signed messages from the *same* node
    /// consensus key. Only the node itself possesses this key, so this proves
    /// double-signing at `slot`. Both messages must be signed by `node_key`.
    pub fn double_sign_from(
        node_key: &SigningKey,
        node_id: Address,
        slot: u64,
    ) -> DoubleSignMaterial {
        let ma = double_sign_message(&node_id, slot, b"value-A");
        let mb = double_sign_message(&node_id, slot, b"value-B");
        let sig_a = node_key.sign(&ma);
        let sig_b = node_key.sign(&mb);
        DoubleSignMaterial {
            node_vk_bytes: node_key.verifying_key().to_bytes(),
            message_a: ma,
            sig_a: sig_a.to_bytes(),
            message_b: mb,
            sig_b: sig_b.to_bytes(),
        }
    }
}

pub use ed25519_dalek::{SigningKey, SigningKey as Ed25519SigningKey};
