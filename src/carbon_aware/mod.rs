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
