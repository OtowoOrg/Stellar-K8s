// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
