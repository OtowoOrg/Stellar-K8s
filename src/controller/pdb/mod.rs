//! Dynamic Pod Disruption Budget (PDB) Auto-Tuning Controller
//!
//! This module contains a controller thread that inspects current SCP sync status
//! across managed Stellar nodes and dynamically adjusts `podDisruptionBudget` rules.
//!
//! The controller sets `maxUnavailable: 0` for any validator that is still catching
//! up, blocking Kubernetes node drains until the node reaches full sync. It also
/// takes total active quorum capacity into account to prevent evictions that could
/// break SCP safety.

pub mod health;
pub mod reconciler;
