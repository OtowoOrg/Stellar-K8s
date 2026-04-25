//! CLI version check: fetches the latest GitHub release and notifies the user
//! if a newer version is available. Results are cached for 24 hours.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const GITHUB_API_URL: &str = "https://api.github.com/repos/stellar/stellar-k8s/releases/latest";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Serialize, Deserialize)]
struct Cache {
    /// Unix timestamp when the cache was written.
    fetched_at: u64,
    /// Latest version tag (e.g. "v0.2.0").
    latest_version: String,
}

fn cache_path() -> PathBuf {
    std::env::temp_dir().join("stellar-k8s-version-cache.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache() -> Option<String> {
    let data = fs::read_to_string(cache_path()).ok()?;
    let cache: Cache = serde_json::from_str(&data).ok()?;
    if now_secs().saturating_sub(cache.fetched_at) < CACHE_TTL.as_secs() {
        Some(cache.latest_version)
    } else {
        None
    }
}

fn write_cache(version: &str) {
    let cache = Cache {
        fetched_at: now_secs(),
        latest_version: version.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = fs::write(cache_path(), json);
    }
}

/// Fetch the latest release tag from GitHub (blocking via a one-shot tokio task).
async fn fetch_latest_version() -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("stellar-k8s/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;

    let release: Release = client
        .get(GITHUB_API_URL)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(release.tag_name)
}

/// Strip a leading `v` from a version string for comparison.
fn strip_v(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

/// Check for a newer version and print a notification to stderr if one exists.
///
/// Pass `offline = true` to skip the network request entirely.
pub async fn check_and_notify(offline: bool) {
    if offline {
        return;
    }

    let latest = if let Some(cached) = read_cache() {
        cached
    } else {
        match fetch_latest_version().await {
            Some(v) => {
                write_cache(&v);
                v
            }
            None => return, // network unavailable — fail silently
        }
    };

    let current = env!("CARGO_PKG_VERSION");
    if strip_v(&latest) != strip_v(current) {
        eprintln!(
            "\n\x1b[33m[stellar-k8s] A new version is available: {} (you have v{})\x1b[0m",
            latest, current
        );
        eprintln!("\x1b[33m  → https://github.com/stellar/stellar-k8s/releases/latest\x1b[0m\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_v_removes_prefix() {
        assert_eq!(strip_v("v0.2.0"), "0.2.0");
        assert_eq!(strip_v("0.2.0"), "0.2.0");
    }

    #[test]
    fn strip_v_same_version_no_notification() {
        let current = env!("CARGO_PKG_VERSION");
        let latest = format!("v{current}");
        assert_eq!(strip_v(&latest), strip_v(current));
    }

    #[test]
    fn cache_roundtrip() {
        // Write a cache entry and read it back.
        let tmp = std::env::temp_dir().join("stellar-k8s-version-cache-test.json");
        let cache = Cache {
            fetched_at: now_secs(),
            latest_version: "v9.9.9".to_string(),
        };
        let json = serde_json::to_string(&cache).unwrap();
        fs::write(&tmp, &json).unwrap();

        let data = fs::read_to_string(&tmp).unwrap();
        let parsed: Cache = serde_json::from_str(&data).unwrap();
        assert_eq!(parsed.latest_version, "v9.9.9");
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn expired_cache_returns_none() {
        let cache = Cache {
            // 25 hours ago
            fetched_at: now_secs().saturating_sub(25 * 60 * 60),
            latest_version: "v0.0.1".to_string(),
        };
        let json = serde_json::to_string(&cache).unwrap();
        fs::write(cache_path(), &json).unwrap();
        // read_cache should return None because TTL has elapsed
        assert!(read_cache().is_none());
    }
}
