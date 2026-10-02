//! Network traffic shaping and CNI plugin integration module.
///
/// This module exposes the zero-downtime egress traffic shaping controller.
/// It dynamically applies Linux Traffic Control (tc) bandwidth limits to pods
/// via Calico or Cilium CNI Custom Resources, and prioritizes intra-cluster
/// mTLS traffic over public-facing ingress.

pub mod cni;
pub mod shaper;

pub use cni::{CniPlugin, CniPluginKind, CniError, PodSelector, NetworkPolicySpec};
pub use shaper::{TrafficShaper, ShaperConfig, ShaperError, EgressLimit, PriorityClass, ShaperStatus};

/// Re-export the high-level controller that ties together CNI discovery and
/// traffic shaping for the network subsystem.
pub use shaper::TrafficShapingController;

/// Convenience alias for the controller type most callers will use.
pub type Controller = shaper::TrafficShapingController;
