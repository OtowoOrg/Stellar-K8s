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
//! Multi-tier intelligent caching for Horizon query optimization.
//!
//! # Cache Topology
//!
//! ```text
//! L1 – In-memory LRU     (sub-microsecond, hot queries)
//! L2 – Redis             (millisecond, shared across replicas)
//! L3 – CDN edge cache    (regional, static/historical queries)
//! ```

pub mod cache;
pub mod invalidation;
pub mod metrics;
pub mod optimizer;
pub mod prefetch;
pub mod streaming;

pub use cache::{CacheStats, HorizonCache, HorizonCacheConfig};
pub use invalidation::{InvalidationEvent, LedgerInvalidator};
pub use optimizer::{QueryOptimizer, QueryPlan, QueryType};
pub use prefetch::{PrefetchEngine, PrefetchPrediction};
pub use streaming::{CompressedResponse, ResponseStreamer};
