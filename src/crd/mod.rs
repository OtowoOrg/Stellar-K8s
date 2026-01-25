//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.

// TODO: Re-enable once compilation issues are resolved
// mod read_only_pool;
mod stellar_node;
mod types;

// TODO: Re-enable once compilation issues are resolved
// pub use read_only_pool::{
//     LedgerRange, ReadOnlyPool, ReadOnlyPoolSpec, ReadOnlyPoolStatus, ReplicaWeight,
//     ShardAssignment, ShardBalancingConfig, ShardStrategy, LoadBalancingConfig,
// };
pub use stellar_node::{BGPStatus, StellarNode, StellarNodeSpec, StellarNodeStatus};
pub use types::*;
