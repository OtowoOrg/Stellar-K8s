//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.

mod read_only_pool;
mod stellar_node;
mod types;

pub use read_only_pool::{
    LedgerRange, ReadOnlyPool, ReadOnlyPoolSpec, ReadOnlyPoolStatus, ReplicaWeight,
    ShardAssignment, ShardBalancingConfig, ShardStrategy, LoadBalancingConfig,
};
pub use stellar_node::{BGPStatus, StellarNode, StellarNodeSpec, StellarNodeStatus};
pub use types::*;
