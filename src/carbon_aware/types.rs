//! Types for carbon-aware scheduling

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Carbon intensity data for a specific region
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CarbonIntensityData {
    /// Region identifier (e.g., "US-CA", "DE")
    pub region: String,
    /// Current carbon intensity in gCO2/kWh
    pub carbon_intensity: f64,
    /// Data timestamp
    pub timestamp: DateTime<Utc>,
    /// Data source (e.g., "ElectricityMap", "API")
    pub source: String,
    /// Renewable energy percentage (0-100)
    pub renewable_percentage: Option<f64>,
    /// Forecast data for next 24 hours
    pub forecast: Option<Vec<CarbonForecast>>,
}

/// Carbon intensity forecast
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CarbonForecast {
    /// Future timestamp
    pub timestamp: DateTime<Utc>,
    /// Predicted carbon intensity
    pub carbon_intensity: f64,
    /// Confidence level (0-1)
    pub confidence: f64,
}

/// Regional carbon data mapping
#[derive(Clone, Debug, Default)]
pub struct RegionCarbonData {
    /// Map of region to carbon intensity data
    pub regions: HashMap<String, CarbonIntensityData>,
    /// Last update timestamp
    pub last_updated: DateTime<Utc>,
}

impl RegionCarbonData {
    /// Create new regional carbon data
    pub fn new() -> Self {
        Self {
            regions: HashMap::new(),
            last_updated: Utc::now(),
        }
    }

    /// Add or update carbon data for a region
    pub fn update_region(&mut self, data: CarbonIntensityData) {
        self.regions.insert(data.region.clone(), data);
        self.last_updated = Utc::now();
    }

    /// Get carbon data for a region
    pub fn get_region(&self, region: &str) -> Option<&CarbonIntensityData> {
        self.regions.get(region)
    }

    /// Get regions sorted by carbon intensity (lowest first)
    pub fn get_regions_by_intensity(&self) -> Vec<&String> {
        let mut regions: Vec<&String> = self.regions.keys().collect();
        regions.sort_by(|a, b| {
            let a_intensity = self
                .regions
                .get(*a)
                .map(|d| d.carbon_intensity)
                .unwrap_or(f64::MAX);
            let b_intensity = self
                .regions
                .get(*b)
                .map(|d| d.carbon_intensity)
                .unwrap_or(f64::MAX);
            a_intensity
                .partial_cmp(&b_intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        regions
    }

    /// Check if data is stale (older than specified minutes)
    pub fn is_stale(&self, max_age_minutes: i64) -> bool {
        let now = Utc::now();
        let age = now.signed_duration_since(self.last_updated);
        age.num_minutes() > max_age_minutes
    }
}

/// Carbon-aware scheduling configuration
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CarbonAwareConfig {
    /// Enable carbon-aware scheduling
    pub enabled: bool,
    /// Carbon intensity API provider
    pub provider: CarbonProvider,
    /// Maximum data age in minutes
    pub max_data_age_minutes: i64,
    /// Weight for carbon intensity in scoring (0-1)
    pub carbon_weight: f64,
    /// Minimum carbon intensity difference to consider migration
    pub migration_threshold: f64,
}

impl Default for CarbonAwareConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: CarbonProvider::default(),
            max_data_age_minutes: 15,
            carbon_weight: 0.7,
            migration_threshold: 50.0, // gCO2/kWh
        }
    }
}

/// Carbon intensity data providers
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CarbonProvider {
    /// ElectricityMap API
    ElectricityMap {
        /// API base URL
        url: String,
        /// API token
        token: String,
    },
    /// Custom API endpoint
    Custom {
        /// API URL
        url: String,
        /// Authentication header
        auth_header: Option<String>,
    },
    /// Mock provider for testing
    Mock,
}

impl Default for CarbonProvider {
    fn default() -> Self {
        Self::ElectricityMap {
            url: "https://api.electricitymap.org".to_string(),
            token: "".to_string(),
        }
    }
}
