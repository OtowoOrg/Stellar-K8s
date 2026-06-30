//! Real-time cost calculation for AWS, GCP, Azure resource types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::model::{CloudProvider, CostRecord, ResourceType};

/// Unit cost for a resource (per hour unless noted)
#[derive(Clone, Debug)]
pub struct PricingRule {
    pub provider: CloudProvider,
    pub resource_type: ResourceType,
    pub instance_type: String,
    pub region: String,
    /// On-demand price per hour (USD)
    pub on_demand_usd: f64,
    /// Reserved 1-year price per hour (USD)
    pub reserved_1yr_usd: f64,
    /// Spot price per hour (USD, approximate)
    pub spot_usd: f64,
}

/// Cost calculated for a resource
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceCost {
    pub resource_id: String,
    pub provider: CloudProvider,
    pub instance_type: String,
    pub hourly_rate: f64,
    pub monthly_estimate_usd: f64,
    pub annual_estimate_usd: f64,
    pub potential_savings_reserved: f64,
    pub potential_savings_spot: f64,
}

const HOURS_PER_MONTH: f64 = 730.0;
const HOURS_PER_YEAR: f64 = 8_760.0;

/// Multi-cloud cost calculator
pub struct CloudCostCalculator {
    pricing: HashMap<String, PricingRule>,
}

impl CloudCostCalculator {
    pub fn new() -> Self {
        let mut calc = Self { pricing: HashMap::new() };
        calc.load_default_pricing();
        calc
    }

    fn pricing_key(provider: &CloudProvider, instance_type: &str) -> String {
        format!("{provider}::{instance_type}")
    }

    fn load_default_pricing(&mut self) {
        // Sample pricing (USD/hr) — replace with live pricing API in production
        let rules = vec![
            // AWS
            ("m5.large",    CloudProvider::Aws,   0.096,  0.058, 0.035),
            ("m5.xlarge",   CloudProvider::Aws,   0.192,  0.116, 0.070),
            ("c5.2xlarge",  CloudProvider::Aws,   0.340,  0.204, 0.120),
            ("r5.4xlarge",  CloudProvider::Aws,   1.008,  0.604, 0.350),
            // GCP
            ("n2-standard-4",  CloudProvider::Gcp, 0.190, 0.128, 0.065),
            ("n2-standard-8",  CloudProvider::Gcp, 0.380, 0.256, 0.130),
            ("n2-highcpu-8",   CloudProvider::Gcp, 0.312, 0.210, 0.110),
            // Azure
            ("Standard_D4s_v3",  CloudProvider::Azure, 0.192, 0.115, 0.070),
            ("Standard_D8s_v3",  CloudProvider::Azure, 0.384, 0.230, 0.140),
        ];

        for (instance, provider, on_demand, reserved, spot) in rules {
            let key = Self::pricing_key(&provider, instance);
            self.pricing.insert(key, PricingRule {
                provider,
                resource_type: ResourceType::ComputeInstance,
                instance_type: instance.into(),
                region: "us-east-1".into(),
                on_demand_usd: on_demand,
                reserved_1yr_usd: reserved,
                spot_usd: spot,
            });
        }
    }

    pub fn calculate(&self, record: &CostRecord) -> ResourceCost {
        let key = Self::pricing_key(&record.provider, &record.instance_type);
        let pricing = self.pricing.get(&key);

        let hourly_rate = if let Some(p) = pricing {
            if record.is_reserved { p.reserved_1yr_usd }
            else if record.is_spot { p.spot_usd }
            else { p.on_demand_usd }
        } else {
            record.hourly_rate()
        };

        let monthly = hourly_rate * HOURS_PER_MONTH;
        let annual = hourly_rate * HOURS_PER_YEAR;

        let (savings_reserved, savings_spot) = if let Some(p) = pricing {
            let base = p.on_demand_usd * HOURS_PER_MONTH;
            ((base - p.reserved_1yr_usd * HOURS_PER_MONTH).max(0.0),
             (base - p.spot_usd * HOURS_PER_MONTH).max(0.0))
        } else {
            (0.0, 0.0)
        };

        ResourceCost {
            resource_id: record.resource_id.clone(),
            provider: record.provider.clone(),
            instance_type: record.instance_type.clone(),
            hourly_rate,
            monthly_estimate_usd: monthly,
            annual_estimate_usd: annual,
            potential_savings_reserved: savings_reserved,
            potential_savings_spot: savings_spot,
        }
    }
}

impl Default for CloudCostCalculator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn aws_record(instance: &str) -> CostRecord {
        CostRecord {
            id: "r1".into(),
            provider: CloudProvider::Aws,
            resource_type: ResourceType::ComputeInstance,
            resource_id: "i-1234".into(),
            namespace: "default".into(),
            team: "infra".into(),
            region: "us-east-1".into(),
            instance_type: instance.into(),
            cost_usd: 10.0,
            usage_hours: 24.0,
            period_start: Utc::now(),
            period_end: Utc::now(),
            tags: HashMap::new(),
            is_spot: false,
            is_reserved: false,
        }
    }

    #[test]
    fn test_calculate_known_instance() {
        let calc = CloudCostCalculator::new();
        let cost = calc.calculate(&aws_record("m5.large"));
        assert!((cost.hourly_rate - 0.096).abs() < 0.001);
        assert!(cost.monthly_estimate_usd > 0.0);
        assert!(cost.potential_savings_reserved > 0.0);
    }

    #[test]
    fn test_reserved_rate_lower_than_on_demand() {
        let calc = CloudCostCalculator::new();
        let mut r = aws_record("m5.xlarge");
        r.is_reserved = true;
        let reserved = calc.calculate(&r);
        r.is_reserved = false;
        let on_demand = calc.calculate(&r);
        assert!(reserved.hourly_rate < on_demand.hourly_rate);
    }
}
