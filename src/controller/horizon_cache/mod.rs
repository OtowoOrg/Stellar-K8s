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

pub use cache::{HorizonCache, HorizonCacheConfig, CacheStats};
pub use invalidation::{InvalidationEvent, LedgerInvalidator};
pub use optimizer::{QueryOptimizer, QueryPlan, QueryType};
pub use prefetch::{PrefetchEngine, PrefetchPrediction};
pub use streaming::{CompressedResponse, ResponseStreamer};
