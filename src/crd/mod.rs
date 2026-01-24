//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.

mod stellar_node;
mod types;

pub use stellar_node::{BGPStatus, StellarNode, StellarNodeSpec, StellarNodeStatus};
pub use types::*;
