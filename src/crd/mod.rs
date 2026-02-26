//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.

mod cnpg;
pub mod read_replica;
pub mod seed_secret;
pub mod service_mesh;
mod stellar_node;
pub mod types;

#[cfg(test)]
mod tests;

pub use cnpg::*;
pub use read_replica::{ReadReplicaConfig, ReadReplicaStrategy};
pub use service_mesh::{
    CircuitBreakerConfig, IstioMeshConfig, LinkerdMeshConfig, MtlsMode, RetryConfig,
    ServiceMeshConfig,
};
pub use stellar_node::{
    BGPStatus, SpecValidationError, StellarNode, StellarNodeSpec, StellarNodeStatus,
};
pub use types::*;
