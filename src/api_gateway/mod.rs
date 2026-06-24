//! API gateway with intelligent routing, protocol transformation, versioning,
//! rate limiting, analytics, and API key management (#789).
//!
//! Architecture:
//! ```text
//! Client Request
//!   → API Key Auth
//!   → Rate Limiter
//!   → Version Router  (v1 / v2 / deprecated)
//!   → Protocol Transform  (REST / gRPC / GraphQL)
//!   → Request Validator
//!   → Upstream Proxy
//!   → Response Transform
//!   → Analytics Recorder
//! ```

pub mod analytics;
pub mod auth;
pub mod config;
pub mod router;
pub mod server;
pub mod transform;
pub mod versioning;

pub use auth::{ApiKey, ApiKeyStore};
pub use config::GatewayConfig;
pub use router::RouteMatch;
pub use server::ApiGateway;
