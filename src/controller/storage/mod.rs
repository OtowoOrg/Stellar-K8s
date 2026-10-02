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
//! Storage auto-resizing for Stellar nodes.
//!
//! Split into two concerns:
//!
//! - [`metrics`] — reading kubelet volume usage. Kept separate from the HTTP
//!   call so the PromQL and response parsing are testable in isolation.
//! - [`autoresize`] — reflecting the resulting PVC size back onto the parent
//!   `StellarNode`, so the CR reports the capacity it actually has.
//!
//! The decision to *grow* a volume lives in
//! [`crate::controller::volume_resizer`], which is driven by
//! [`crate::controller::pvc_autoscaler`]. This module handles what happens
//! afterwards.

pub mod autoresize;
pub mod metrics;
