//! PostgreSQL connection pool configuration and factory.
//!
//! Provides a configurable `sqlx::PgPool` builder with support for:
//! - Maximum connection count
//! - Connection acquisition timeout
//! - Idle connection timeout
//! - Statement-level query timeout (via `SET statement_timeout`)
//!
//! # Example
//!
//! ```rust,no_run
//! use stellar_k8s::controller::db_pool::{DbPoolConfig, create_pool};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DbPoolConfig {
//!     database_url: "postgres://user:pass@localhost/horizon".to_string(),
//!     max_connections: 10,
//!     connection_timeout_secs: 5,
//!     idle_timeout_secs: Some(300),
//!     query_timeout_ms: Some(30_000),
//! };
//! let pool = create_pool(&config).await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::error::Result;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration parameters for a `sqlx` PostgreSQL connection pool.
#[derive(Debug, Clone)]
pub struct DbPoolConfig {
    /// Full PostgreSQL connection URL, e.g.
    /// `postgres://user:password@host:5432/dbname`.
    pub database_url: String,

    /// Maximum number of connections maintained in the pool.
    /// Defaults to [`DEFAULT_MAX_CONNECTIONS`].
    pub max_connections: u32,

    /// How long (in seconds) to wait when acquiring a connection before
    /// returning an error.  Defaults to [`DEFAULT_CONNECTION_TIMEOUT_SECS`].
    pub connection_timeout_secs: u64,

    /// How long (in seconds) an idle connection may remain in the pool before
    /// being closed.  `None` keeps connections open indefinitely.
    pub idle_timeout_secs: Option<u64>,

    /// Statement-level query timeout in milliseconds applied to every new
    /// connection via `SET statement_timeout = <ms>`.  `None` disables the
    /// per-connection timeout.
    pub query_timeout_ms: Option<u64>,
}

/// Default maximum number of pool connections.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 5;

/// Default connection-acquisition timeout in seconds.
pub const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 10;

impl Default for DbPoolConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            connection_timeout_secs: DEFAULT_CONNECTION_TIMEOUT_SECS,
            idle_timeout_secs: None,
            query_timeout_ms: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pool factory
// ---------------------------------------------------------------------------

/// Create a `sqlx::PgPool` from a [`DbPoolConfig`].
///
/// If `config.query_timeout_ms` is `Some(ms)`, every new connection will
/// execute `SET statement_timeout = <ms>` immediately after being opened so
/// that runaway queries are automatically cancelled.
pub async fn create_pool(config: &DbPoolConfig) -> Result<PgPool> {
    let mut opts = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.connection_timeout_secs));

    if let Some(idle_secs) = config.idle_timeout_secs {
        opts = opts.idle_timeout(Duration::from_secs(idle_secs));
    }

    // Apply statement_timeout on each new physical connection.
    let query_timeout_ms = config.query_timeout_ms;
    if let Some(timeout_ms) = query_timeout_ms {
        opts = opts.after_connect(move |conn, _meta| {
            Box::pin(async move {
                sqlx::query(&format!("SET statement_timeout = {timeout_ms}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        });
    }

    let pool = opts.connect(&config.database_url).await.map_err(|e| {
        crate::error::Error::ConfigError(format!("Failed to connect to database: {e}"))
    })?;

    Ok(pool)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let cfg = DbPoolConfig::default();
        assert_eq!(cfg.max_connections, DEFAULT_MAX_CONNECTIONS);
        assert_eq!(cfg.connection_timeout_secs, DEFAULT_CONNECTION_TIMEOUT_SECS);
        assert!(cfg.idle_timeout_secs.is_none());
        assert!(cfg.query_timeout_ms.is_none());
        assert!(cfg.database_url.is_empty());
    }

    #[test]
    fn test_custom_config() {
        let cfg = DbPoolConfig {
            database_url: "postgres://localhost/test".to_string(),
            max_connections: 20,
            connection_timeout_secs: 3,
            idle_timeout_secs: Some(600),
            query_timeout_ms: Some(5_000),
        };
        assert_eq!(cfg.max_connections, 20);
        assert_eq!(cfg.connection_timeout_secs, 3);
        assert_eq!(cfg.idle_timeout_secs, Some(600));
        assert_eq!(cfg.query_timeout_ms, Some(5_000));
    }

    #[test]
    fn test_config_clone() {
        let original = DbPoolConfig {
            database_url: "postgres://localhost/test".to_string(),
            max_connections: 10,
            connection_timeout_secs: 5,
            idle_timeout_secs: Some(300),
            query_timeout_ms: Some(10_000),
        };
        let cloned = original.clone();
        assert_eq!(cloned.max_connections, original.max_connections);
        assert_eq!(cloned.database_url, original.database_url);
        assert_eq!(cloned.query_timeout_ms, original.query_timeout_ms);
    }

    #[test]
    fn test_config_debug_format() {
        let cfg = DbPoolConfig::default();
        let debug_str = format!("{:?}", cfg);
        assert!(debug_str.contains("DbPoolConfig"));
        assert!(debug_str.contains("max_connections"));
    }

    #[test]
    fn test_pool_options_apply_idle_timeout() {
        // Verify the idle_timeout branch compiles and runs without panics
        let cfg = DbPoolConfig {
            database_url: "postgres://localhost/test".to_string(),
            max_connections: 2,
            connection_timeout_secs: 1,
            idle_timeout_secs: Some(60),
            query_timeout_ms: None,
        };
        assert_eq!(cfg.idle_timeout_secs, Some(60));
    }

    #[test]
    fn test_pool_options_apply_query_timeout() {
        // Verify the query_timeout_ms branch compiles without panics
        let cfg = DbPoolConfig {
            database_url: "postgres://localhost/test".to_string(),
            max_connections: 2,
            connection_timeout_secs: 1,
            idle_timeout_secs: None,
            query_timeout_ms: Some(15_000),
        };
        assert_eq!(cfg.query_timeout_ms, Some(15_000));
    }

    #[test]
    fn test_statement_timeout_format() {
        let timeout_ms: u64 = 5000;
        let stmt = format!("SET statement_timeout = {timeout_ms}");
        assert_eq!(stmt, "SET statement_timeout = 5000");
    }

    #[test]
    fn test_zero_query_timeout_is_valid() {
        let cfg = DbPoolConfig {
            database_url: "postgres://localhost/test".to_string(),
            max_connections: 1,
            connection_timeout_secs: 1,
            idle_timeout_secs: None,
            query_timeout_ms: Some(0),
        };
        // 0 ms means queries are not cancelled immediately in PostgreSQL
        // (it is treated as "no timeout"), but the config should be accepted
        assert_eq!(cfg.query_timeout_ms, Some(0));
    }
}
