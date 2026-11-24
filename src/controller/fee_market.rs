use crate::controller::metrics::{
    FeeMarketLabels, FEE_MARKET_BURN_STROOPS_TOTAL, FEE_MARKET_INCLUSION_RATE, FEE_MARKET_LEDGER,
    FEE_MARKET_P95_STROOPS, FEE_MARKET_SPIKE_THRESHOLD_STROOPS,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

static LAST_RECORDED_FEE_LEDGER: once_cell::sync::Lazy<Mutex<HashMap<(String, String), u64>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeMarketSnapshot {
    pub latest_ledger: u64,
    pub classic_p95_stroops: Option<u64>,
    pub soroban_p95_stroops: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeLedgerObservation {
    pub ledger: u64,
    pub classic_fee_burn_stroops: u64,
    pub classic_submitted: u64,
    pub classic_included: u64,
    pub soroban_fee_burn_stroops: u64,
    pub soroban_submitted: u64,
    pub soroban_included: u64,
}

/// Record one finalized ledger's fee totals; callers must submit each ledger once.
pub fn record_ledger_observation(network: &str, observation: &FeeLedgerObservation) {
    for (fee_type, burn, submitted, included) in [
        (
            "classic",
            observation.classic_fee_burn_stroops,
            observation.classic_submitted,
            observation.classic_included,
        ),
        (
            "soroban",
            observation.soroban_fee_burn_stroops,
            observation.soroban_submitted,
            observation.soroban_included,
        ),
    ] {
        let labels = FeeMarketLabels {
            network: network.to_string(),
            fee_type: fee_type.to_string(),
        };
        let key = (labels.network.clone(), labels.fee_type.clone());
        let mut last_recorded = LAST_RECORDED_FEE_LEDGER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !is_new_ledger(last_recorded.get(&key).copied(), observation.ledger) {
            continue;
        }
        last_recorded.insert(key, observation.ledger);
        FEE_MARKET_LEDGER
            .get_or_create(&labels)
            .set(observation.ledger.min(i64::MAX as u64) as i64);
        FEE_MARKET_BURN_STROOPS_TOTAL
            .get_or_create(&labels)
            .inc_by(burn);
        if submitted > 0 {
            FEE_MARKET_INCLUSION_RATE
                .get_or_create(&labels)
                .set(included.min(submitted) as f64 / submitted as f64);
        }
    }
}

fn is_new_ledger(previous: Option<u64>, current: u64) -> bool {
    previous.is_none_or(|previous| current > previous)
}

impl FeeMarketSnapshot {
    pub fn from_rpc_response(response: &Value) -> Result<Self, String> {
        if let Some(error) = response.get("error") {
            return Err(format!("getFeeStats RPC error: {error}"));
        }
        let result = response
            .get("result")
            .ok_or_else(|| "getFeeStats response is missing result".to_string())?;
        let latest_ledger = result
            .get("latestLedger")
            .and_then(Value::as_u64)
            .ok_or_else(|| "getFeeStats result is missing latestLedger".to_string())?;
        Ok(Self {
            latest_ledger,
            classic_p95_stroops: p95(result.get("inclusionFee"))?,
            soroban_p95_stroops: p95(result.get("sorobanInclusionFee"))?,
        })
    }

    pub fn record(&self, network: &str, spike_threshold_stroops: u64) {
        for (fee_type, value) in [
            ("classic", self.classic_p95_stroops),
            ("soroban", self.soroban_p95_stroops),
        ] {
            let labels = FeeMarketLabels {
                network: network.to_string(),
                fee_type: fee_type.to_string(),
            };
            FEE_MARKET_LEDGER
                .get_or_create(&labels)
                .set(self.latest_ledger.min(i64::MAX as u64) as i64);
            FEE_MARKET_SPIKE_THRESHOLD_STROOPS
                .get_or_create(&labels)
                .set(spike_threshold_stroops.min(i64::MAX as u64) as i64);
            if let Some(value) = value {
                FEE_MARKET_P95_STROOPS
                    .get_or_create(&labels)
                    .set(value.min(i64::MAX as u64) as i64);
            }
        }
    }
}

fn p95(stats: Option<&Value>) -> Result<Option<u64>, String> {
    let Some(value) = stats.and_then(|stats| stats.get("p95")) else {
        return Ok(None);
    };
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .map(Some)
        .ok_or_else(|| "fee p95 value is not an unsigned integer".to_string())
}

pub async fn get_fee_stats(
    client: &reqwest::Client,
    rpc_url: &str,
) -> Result<FeeMarketSnapshot, String> {
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getFeeStats",
            "params": {}
        }))
        .send()
        .await
        .map_err(|error| format!("getFeeStats request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("getFeeStats HTTP error: {error}"))?
        .json::<Value>()
        .await
        .map_err(|error| format!("getFeeStats response decode failed: {error}"))?;
    FeeMarketSnapshot::from_rpc_response(&response)
}

pub async fn run_fee_market_collector(
    rpc_url: String,
    network: String,
    threshold: u64,
    is_leader: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let client = reqwest::Client::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        if !is_leader.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        match get_fee_stats(&client, &rpc_url).await {
            Ok(snapshot) => snapshot.record(&network, threshold),
            Err(error) => tracing::warn!(%error, %network, "fee market poll failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_new_ledger, FeeMarketSnapshot};
    use serde_json::json;

    #[test]
    fn parses_classic_and_soroban_fee_percentiles() {
        let snapshot = FeeMarketSnapshot::from_rpc_response(&json!({
            "result": {
                "latestLedger": 123,
                "inclusionFee": {"p95": "100"},
                "sorobanInclusionFee": {"p95": "2500"}
            }
        }))
        .unwrap();
        assert_eq!(snapshot.latest_ledger, 123);
        assert_eq!(snapshot.classic_p95_stroops, Some(100));
        assert_eq!(snapshot.soroban_p95_stroops, Some(2500));
    }

    #[test]
    fn rejects_rpc_errors_and_missing_ledger_sequence() {
        assert!(FeeMarketSnapshot::from_rpc_response(&json!({"error": "unavailable"})).is_err());
        assert!(FeeMarketSnapshot::from_rpc_response(&json!({"result": {}})).is_err());
    }

    #[test]
    fn ledger_observations_are_monotonic() {
        assert!(is_new_ledger(None, 10));
        assert!(is_new_ledger(Some(10), 11));
        assert!(!is_new_ledger(Some(10), 10));
        assert!(!is_new_ledger(Some(10), 9));
    }
}
