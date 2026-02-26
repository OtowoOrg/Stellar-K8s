//! Carbon intensity API integration

use crate::carbon_aware::types::{CarbonIntensityData, CarbonProvider, RegionCarbonData};
use crate::error::Result;
use chrono::Utc;
use reqwest::Client;
use serde_json::Value;
use tracing::{info, warn};

/// Carbon intensity API client
#[derive(Clone)]
pub struct CarbonIntensityAPI {
    client: Client,
    provider: CarbonProvider,
}

impl CarbonIntensityAPI {
    /// Create new carbon intensity API client
    pub fn new(provider: CarbonProvider) -> Self {
        Self {
            client: Client::new(),
            provider,
        }
    }

    /// Fetch current carbon intensity data for all regions
    pub async fn fetch_all_regions(&self) -> Result<RegionCarbonData> {
        match &self.provider {
            CarbonProvider::ElectricityMap { url, token } => {
                self.fetch_electricitymap_data(url, token).await
            }
            CarbonProvider::Custom { url, auth_header } => {
                self.fetch_custom_data(url, auth_header).await
            }
            CarbonProvider::Mock => self.fetch_mock_data().await,
        }
    }

    /// Fetch carbon intensity for a specific region
    pub async fn fetch_region(&self, region: &str) -> Result<Option<CarbonIntensityData>> {
        let all_data = self.fetch_all_regions().await?;
        Ok(all_data.get_region(region).cloned())
    }

    /// Fetch data from ElectricityMap API
    async fn fetch_electricitymap_data(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<RegionCarbonData> {
        let url = format!("{}/v3/carbon-intensity/latest", base_url);

        let mut request = self.client.get(&url);

        if !token.is_empty() {
            request = request.header("auth-token", token);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(crate::error::Error::NetworkError(format!(
                "ElectricityMap API error: {}",
                response.status()
            )));
        }

        let json: Value = response.json().await?;
        let mut region_data = RegionCarbonData::new();

        if let Some(data) = json.as_array() {
            for item in data {
                if let (Some(region), Some(intensity), Some(datetime)) = (
                    item.get("zone").and_then(|z| z.as_str()),
                    item.get("carbonIntensity").and_then(|ci| ci.as_f64()),
                    item.get("datetime").and_then(|dt| dt.as_str()),
                ) {
                    // Parse datetime
                    let timestamp = match chrono::DateTime::parse_from_rfc3339(datetime) {
                        Ok(dt) => dt.with_timezone(&Utc),
                        Err(e) => {
                            warn!("Failed to parse datetime '{}': {}", datetime, e);
                            Utc::now()
                        }
                    };

                    // Extract renewable percentage if available
                    let renewable_percentage =
                        item.get("renewablePercentage").and_then(|rp| rp.as_f64());

                    let carbon_data = CarbonIntensityData {
                        region: region.to_string(),
                        carbon_intensity: intensity,
                        timestamp,
                        source: "ElectricityMap".to_string(),
                        renewable_percentage,
                        forecast: None, // Could be extended to fetch forecast data
                    };

                    region_data.update_region(carbon_data);
                }
            }
        }

        info!(
            "Fetched carbon intensity data for {} regions from ElectricityMap",
            region_data.regions.len()
        );

        Ok(region_data)
    }

    /// Fetch data from custom API
    async fn fetch_custom_data(
        &self,
        url: &str,
        auth_header: &Option<String>,
    ) -> Result<RegionCarbonData> {
        let mut request = self.client.get(url);

        if let Some(auth) = auth_header {
            request = request.header("Authorization", auth);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(crate::error::Error::NetworkError(format!(
                "Custom API error: {}",
                response.status()
            )));
        }

        let json: Value = response.json().await?;
        let mut region_data = RegionCarbonData::new();

        // Expected format: {"regions": [{"region": "US-CA", "carbonIntensity": 100, ...}]}
        if let Some(regions) = json.get("regions").and_then(|r| r.as_array()) {
            for item in regions {
                if let Some(data) = self.parse_custom_region_data(item) {
                    region_data.update_region(data);
                }
            }
        }

        info!(
            "Fetched carbon intensity data for {} regions from custom API",
            region_data.regions.len()
        );

        Ok(region_data)
    }

    /// Parse region data from custom API response
    fn parse_custom_region_data(&self, item: &Value) -> Option<CarbonIntensityData> {
        let region = item.get("region").and_then(|r| r.as_str())?;
        let intensity = item.get("carbonIntensity").and_then(|ci| ci.as_f64())?;

        let timestamp = item
            .get("timestamp")
            .and_then(|ts| ts.as_str())
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let renewable_percentage = item.get("renewablePercentage").and_then(|rp| rp.as_f64());

        Some(CarbonIntensityData {
            region: region.to_string(),
            carbon_intensity: intensity,
            timestamp,
            source: "Custom API".to_string(),
            renewable_percentage,
            forecast: None,
        })
    }

    /// Generate mock data for testing
    async fn fetch_mock_data(&self) -> Result<RegionCarbonData> {
        let mut region_data = RegionCarbonData::new();
        let now = Utc::now();

        // Mock data for various regions with realistic carbon intensity values
        let mock_regions = vec![
            ("US-CA", 150.0, Some(45.0)),  // California - moderate
            ("US-WA", 80.0, Some(65.0)),   // Washington - low (hydro)
            ("DE", 300.0, Some(25.0)),     // Germany - high
            ("FR", 60.0, Some(70.0)),      // France - very low (nuclear)
            ("GB", 200.0, Some(35.0)),     // Great Britain - moderate
            ("NO", 30.0, Some(95.0)),      // Norway - very low (hydro)
            ("SE", 50.0, Some(80.0)),      // Sweden - very low (hydro/nuclear)
            ("AU-NSW", 600.0, Some(15.0)), // Australia NSW - high (coal)
        ];

        for (region, intensity, renewable) in mock_regions {
            let carbon_data = CarbonIntensityData {
                region: region.to_string(),
                carbon_intensity: intensity,
                timestamp: now,
                source: "Mock".to_string(),
                renewable_percentage: renewable,
                forecast: None,
            };
            region_data.update_region(carbon_data);
        }

        info!(
            "Generated mock carbon intensity data for {} regions",
            region_data.regions.len()
        );
        Ok(region_data)
    }

    /// Validate API connectivity
    pub async fn health_check(&self) -> Result<bool> {
        match &self.provider {
            CarbonProvider::ElectricityMap { url, .. } => {
                let health_url = format!("{}/health", url);
                let response = self.client.get(&health_url).send().await?;
                Ok(response.status().is_success())
            }
            CarbonProvider::Custom { url, .. } => {
                let response = self.client.get(url).send().await?;
                Ok(response.status().is_success())
            }
            CarbonProvider::Mock => Ok(true),
        }
    }
}
