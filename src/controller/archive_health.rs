//! History Archive Health Check Module
//!
//! Provides async health checking for Stellar history archive URLs.
//! Used to verify archives are reachable before starting validator nodes.

use crate::error::{Error, Result};
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, warn};

/// Result of checking multiple history archive URLs
#[derive(Debug, Clone)]
pub struct ArchiveHealthResult {
    /// URLs that passed the health check
    pub healthy_urls: Vec<String>,
    /// URLs that failed with their error messages
    pub unhealthy_urls: Vec<(String, String)>,
    /// True if all URLs are healthy
    pub all_healthy: bool,
    /// True if at least one URL is healthy
    pub any_healthy: bool,
}

impl ArchiveHealthResult {
    /// Create a new result from check outcomes
    pub fn new(healthy: Vec<String>, unhealthy: Vec<(String, String)>) -> Self {
        let all_healthy = unhealthy.is_empty() && !healthy.is_empty();
        let any_healthy = !healthy.is_empty();

        Self {
            healthy_urls: healthy,
            unhealthy_urls: unhealthy,
            all_healthy,
            any_healthy,
        }
    }

    pub fn summary(&self) -> String {
        if self.healthy_urls.is_empty() && self.unhealthy_urls.is_empty() {
            "No archives configured".to_string()
        } else if self.all_healthy {
            format!("All {} archive(s) healthy", self.healthy_urls.len())
        } else if self.any_healthy {
            format!(
                "{} healthy, {} unhealthy archive(s)",
                self.healthy_urls.len(),
                self.unhealthy_urls.len()
            )
        } else {
            format!("All {} archive(s) unhealthy", self.unhealthy_urls.len())
        }
    }

    /// Get detailed error messages for unhealthy archives
    pub fn error_details(&self) -> String {
        self.unhealthy_urls
            .iter()
            .map(|(url, err)| format!("  - {}: {}", url, err))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Check health of a single history archive URL
///
/// Tries the following endpoints in order:
/// 1. HEAD request to `.well-known/stellar-history.json` (lightweight)
/// 2. GET request to root `/` (fallback)
async fn check_single_archive(client: &Client, url: &str, timeout: Duration) -> Result<()> {
    let base_url = url.trim_end_matches('/');

    // Try the standard Stellar history metadata endpoint first
    let metadata_url = format!("{}/.well-known/stellar-history.json", base_url);

    debug!("Checking archive health: {}", metadata_url);

    match client.head(&metadata_url).timeout(timeout).send().await {
        Ok(resp) if resp.status().is_success() => {
            debug!("Archive healthy (metadata endpoint): {}", url);
            return Ok(());
        }
        Ok(resp) => {
            debug!(
                "Metadata endpoint returned {}, trying root: {}",
                resp.status(),
                url
            );
        }
        Err(e) => {
            debug!("Metadata endpoint failed ({}), trying root: {}", e, url);
        }
    }

    // Fallback to root endpoint
    match client.head(base_url).timeout(timeout).send().await {
        Ok(resp) if resp.status().is_success() => {
            debug!("Archive healthy (root endpoint): {}", url);
            Ok(())
        }
        Ok(resp) => {
            let msg = format!("Archive returned HTTP {}", resp.status());
            warn!("{}: {}", url, msg);
            Err(Error::ArchiveHealthCheckError(msg))
        }
        Err(e) => {
            let msg = format!("Connection failed: {}", e);
            warn!("{}: {}", url, msg);
            Err(Error::HttpError(e))
        }
    }
}

/// Check health of multiple history archive URLs in parallel
///
/// # Arguments
/// * `urls` - List of archive URLs to check
/// * `timeout` - Timeout per URL check (default: 10 seconds)
///
/// # Returns
/// `ArchiveHealthResult` with details of healthy and unhealthy archives
pub async fn check_history_archive_health(
    urls: &[String],
    timeout: Option<Duration>,
) -> Result<ArchiveHealthResult> {
    if urls.is_empty() {
        debug!("No archive URLs to check, skipping health check");
        return Ok(ArchiveHealthResult::new(vec![], vec![]));
    }

    let timeout = timeout.unwrap_or(Duration::from_secs(10));

    // Create HTTP client with reasonable defaults
    let client = Client::builder()
        .timeout(timeout)
        .user_agent("stellar-k8s-operator/0.1.0")
        .build()
        .map_err(Error::HttpError)?;

    // Check all URLs in parallel
    let checks: Vec<_> = urls
        .iter()
        .map(|url| check_single_archive(&client, url, timeout))
        .collect();

    let results = futures::future::join_all(checks).await;

    // Categorize results
    let mut healthy = Vec::new();
    let mut unhealthy = Vec::new();

    for (url, result) in urls.iter().zip(results.into_iter()) {
        match result {
            Ok(()) => healthy.push(url.clone()),
            Err(e) => unhealthy.push((url.clone(), e.to_string())),
        }
    }

    let health_result = ArchiveHealthResult::new(healthy, unhealthy);

    debug!("Archive health check complete: {}", health_result.summary());

    Ok(health_result)
}

/// Calculate exponential backoff delay for retry attempts
///
/// # Arguments
/// * `attempt` - Current retry attempt number (0-indexed)
/// * `base_delay_secs` - Base delay in seconds (default: 15)
/// * `max_delay_secs` - Maximum delay cap in seconds (default: 300 = 5 minutes)
///
/// # Returns
/// Duration to wait before next retry
pub fn calculate_backoff(
    attempt: u32,
    base_delay_secs: Option<u64>,
    max_delay_secs: Option<u64>,
) -> Duration {
    let base = base_delay_secs.unwrap_or(15);
    let max = max_delay_secs.unwrap_or(300);

    // Exponential: base * 2^attempt, capped at max
    let delay_secs = base.saturating_mul(2_u64.saturating_pow(attempt.min(5)));
    let capped_delay = delay_secs.min(max);

    Duration::from_secs(capped_delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        // Attempt 0: 15 seconds
        assert_eq!(calculate_backoff(0, None, None), Duration::from_secs(15));

        // Attempt 1: 30 seconds
        assert_eq!(calculate_backoff(1, None, None), Duration::from_secs(30));

        // Attempt 2: 60 seconds
        assert_eq!(calculate_backoff(2, None, None), Duration::from_secs(60));

        // Attempt 3: 120 seconds
        assert_eq!(calculate_backoff(3, None, None), Duration::from_secs(120));

        // Attempt 4: 240 seconds
        assert_eq!(calculate_backoff(4, None, None), Duration::from_secs(240));

        // Attempt 5+: capped at 300 seconds (5 minutes)
        assert_eq!(calculate_backoff(5, None, None), Duration::from_secs(300));
        assert_eq!(calculate_backoff(10, None, None), Duration::from_secs(300));
    }

    #[test]
    fn test_health_result_summary() {
        let result = ArchiveHealthResult::new(vec!["http://archive1.com".to_string()], vec![]);
        assert!(result.all_healthy);
        assert!(result.any_healthy);
        assert_eq!(result.summary(), "All 1 archive(s) healthy");

        let result = ArchiveHealthResult::new(
            vec!["http://archive1.com".to_string()],
            vec![("http://archive2.com".to_string(), "timeout".to_string())],
        );
        assert!(!result.all_healthy);
        assert!(result.any_healthy);
        assert_eq!(result.summary(), "1 healthy, 1 unhealthy archive(s)");

        let result = ArchiveHealthResult::new(
            vec![],
            vec![("http://archive1.com".to_string(), "timeout".to_string())],
        );
        assert!(!result.all_healthy);
        assert!(!result.any_healthy);
        assert_eq!(result.summary(), "All 1 archive(s) unhealthy");
    }

    #[test]
    fn test_health_result_empty() {
        let result = ArchiveHealthResult::new(vec![], vec![]);
        assert!(!result.all_healthy);
        assert!(!result.any_healthy);
        assert_eq!(result.summary(), "No archives configured");
    }
}
