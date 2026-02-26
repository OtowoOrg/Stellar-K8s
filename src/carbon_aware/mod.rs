//! Carbon-aware scheduling for Stellar-K8s
//!
//! This module implements carbon intensity monitoring and scheduling
//! to optimize Stellar node placement for minimal CO2 footprint.

pub mod api;
pub mod scheduler;
pub mod types;

pub use api::CarbonIntensityAPI;
pub use scheduler::CarbonAwareScheduler;
pub use types::{CarbonIntensityData, RegionCarbonData};
