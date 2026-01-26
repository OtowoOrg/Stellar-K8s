//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.

mod stellar_node;
mod types;

#[cfg(test)]
mod tests;

pub use stellar_node::{BGPStatus, StellarNode, StellarNodeSpec, StellarNodeStatus};
pub use types::*;
