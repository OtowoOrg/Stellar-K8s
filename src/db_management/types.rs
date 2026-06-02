//! Core types for the database management framework.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which Stellar database this config targets
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbTarget {
    Horizon,
    Soroban,
    Custom(String),
}

impl std::fmt::Display for DbTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Horizon => write!(f, "horizon"),
            Self::Soroban => write!(f, "soroban"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

/// Top-level configuration for the database management system
#[derive(Clone, Debug)]
pub struct DbManagementConfig {
    pub database_url: String,
    pub target: DbTarget,
    /// Slow-query threshold in milliseconds
    pub slow_query_threshold_ms: u64,
    /// Bloat ratio above which VACUUM is triggered (0.0–1.0)
    pub vacuum_bloat_threshold: f64,
    /// Pool size bounds for the optimizer
    pub pool_min: u32,
    pub pool_max: u32,
}

impl Default for DbManagementConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            target: DbTarget::Horizon,
            slow_query_threshold_ms: 1_000,
            vacuum_bloat_threshold: 0.2,
            pool_min: 2,
            pool_max: 20,
        }
    }
}

/// A generic health/status level
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
}

/// A single alert emitted by any sub-system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbAlert {
    pub level: HealthStatus,
    pub subsystem: String,
    pub message: String,
    pub raised_at: DateTime<Utc>,
}

impl DbAlert {
    pub fn warn(subsystem: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: HealthStatus::Warning,
            subsystem: subsystem.into(),
            message: message.into(),
            raised_at: Utc::now(),
        }
    }
    pub fn critical(subsystem: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: HealthStatus::Critical,
            subsystem: subsystem.into(),
            message: message.into(),
            raised_at: Utc::now(),
        }
    }
}
