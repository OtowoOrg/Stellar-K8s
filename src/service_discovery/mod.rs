//! Advanced Service Discovery with Dynamic Topology Mapping
//!
//! Provides automatic service topology discovery, dependency graph generation,
//! health-based load balancing, canary routing, service mesh integration,
//! Prometheus metrics, and a service catalog.

pub mod catalog;
pub mod graph;
pub mod health;
pub mod load_balancer;
pub mod mesh;
pub mod metrics;
pub mod registry;
pub mod version;

pub use catalog::{ServiceCatalog, ServiceEntry};
pub use graph::{DependencyGraph, TopologyExport};
pub use health::{HealthScore, HealthTracker, ServiceHealth};
pub use load_balancer::{LoadBalancer, RoutingDecision};
pub use mesh::{MeshAnnotations, ServiceMeshIntegration};
pub use metrics::DiscoveryMetrics;
pub use registry::{ServiceRegistry, ServiceRegistration};
pub use version::{CanaryConfig, VersionManager};
