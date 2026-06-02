//! Update check: fetches the latest release from GitHub and compares with the local version.
//!
//! Used by both `stellar-operator update-check` and `kubectl stellar update-check`.

use std::time::Duration;

use serde::Deserialize;

/// GitHub repository for this project (from Cargo.toml `repository` field).
const GITHUB_REPO: &str = "stellar/stellar-k8s";

/// GitHub Releases API endpoint for the latest release.
const GITHUB_API_URL: &str =
    "https://api.github.com/repos/stellar/stellar-k8s/releases/latest";

/// Human-readable URL for the releases page.
const RELEASES_URL: &str = "https://github.com/stellar/stellar-k8s/releases";

/// Minimal subset of the GitHub releases API response we care about.
#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    /// Tag name, e.g. `"v0.2.0"`.
    pub tag_name: String,
    /// Release name / title.
    pub name: Option<String>,
    /// URL to the release page on GitHub.
    pub html_url: String,
    /// Whether this is a pre-release.
    pub prerelease: bool,
    /// Publication timestamp (ISO 8601).
    pub published_at: Option<String>,
}

/// Result of a version comparison.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionStatus {
    /// The local version is up-to-date.
    UpToDate,
    /// A newer version is available on GitHub.
    UpdateAvailable {
        latest: String,
        release_url: String,
    },
    /// The local version is newer than the latest GitHub release (dev / pre-release build).
    AheadOfRelease,
}

/// Fetch the latest release from GitHub and compare it with `local_version`.
///
/// `local_version` should be a semver string without a leading `v`, e.g. `"0.1.0"`.
pub async fn check_for_update(local_version: &str) -> Result<(GitHubRelease, VersionStatus), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!(
            "stellar-k8s/{} (update-check; +{})",
            local_version, RELEASES_URL
        ))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .get(GITHUB_API_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub API: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "GitHub API returned HTTP {status}. Check your network connection or try again later."
        ));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub API response: {e}"))?;

    let status = compare_versions(local_version, &release.tag_name, &release.html_url);
    Ok((release, status))
}

/// Compare `local` (e.g. `"0.1.0"`) with `remote_tag` (e.g. `"v0.2.0"`).
fn compare_versions(local: &str, remote_tag: &str, release_url: &str) -> VersionStatus {
    // Strip a leading 'v' from the tag if present.
    let remote = remote_tag.trim_start_matches('v');

    // Parse both as semver tuples (major, minor, patch).  Fall back to string
    // comparison if parsing fails so we never panic on unexpected tag formats.
    match (parse_semver(local), parse_semver(remote)) {
        (Some(l), Some(r)) => {
            use std::cmp::Ordering;
            match l.cmp(&r) {
                Ordering::Less => VersionStatus::UpdateAvailable {
                    latest: remote_tag.to_string(),
                    release_url: release_url.to_string(),
                },
                Ordering::Equal => VersionStatus::UpToDate,
                Ordering::Greater => VersionStatus::AheadOfRelease,
            }
        }
        // If we can't parse, fall back to a simple string comparison.
        _ => {
            if local == remote {
                VersionStatus::UpToDate
            } else {
                VersionStatus::UpdateAvailable {
                    latest: remote_tag.to_string(),
                    release_url: release_url.to_string(),
                }
            }
        }
    }
}

/// Parse a semver string `"major.minor.patch"` into a comparable tuple.
fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    // Strip any pre-release suffix from the patch component (e.g. "0-alpha.1").
    let patch_str = parts[2].split('-').next().unwrap_or(parts[2]);
    let patch = patch_str.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Print the update-check result to stdout in a human-friendly format.
pub fn print_update_result(
    local_version: &str,
    release: &GitHubRelease,
    status: &VersionStatus,
) {
    let release_name = release
        .name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or(&release.tag_name);

    let published = release
        .published_at
        .as_deref()
        .map(|d| format!(" (published {})", &d[..10])) // trim to YYYY-MM-DD
        .unwrap_or_default();

    println!("Stellar-K8s Update Check");
    println!("{}", "─".repeat(40));
    println!("  Local version  : v{local_version}");
    println!("  Latest release : {}{}", release_name, published);

    if release.prerelease {
        println!("  ⚠  Latest is a pre-release");
    }

    match status {
        VersionStatus::UpToDate => {
            println!();
            println!("✅ You are running the latest release.");
        }
        VersionStatus::UpdateAvailable { latest, release_url } => {
            println!();
            println!("🚀 A new version is available: {latest}");
            println!();
            println!("How to upgrade:");
            println!();
            println!("  Helm (recommended):");
            println!("    helm repo update");
            println!("    helm upgrade stellar-operator stellar/stellar-operator \\");
            println!("      --namespace stellar-system");
            println!();
            println!("  kubectl (manual):");
            println!("    kubectl set image deployment/stellar-operator \\");
            println!("      operator=ghcr.io/{GITHUB_REPO}:{latest} \\");
            println!("      -n stellar-system");
            println!();
            println!("  Release notes:");
            println!("    {release_url}");
            println!();
            println!("  All releases:");
            println!("    {RELEASES_URL}");
        }
        VersionStatus::AheadOfRelease => {
            println!();
            println!("ℹ  Your local build (v{local_version}) is newer than the latest");
            println!("   published release ({}).  You may be running a", release.tag_name);
            println!("   development or pre-release build.");
            println!();
            println!("  All releases: {RELEASES_URL}");
        }
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_semver_valid() {
        assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("10.0.0"), Some((10, 0, 0)));
    }

    #[test]
    fn test_parse_semver_with_prerelease_suffix() {
        assert_eq!(parse_semver("0.2.0-alpha.1"), Some((0, 2, 0)));
        assert_eq!(parse_semver("1.0.0-rc.2"), Some((1, 0, 0)));
    }

    #[test]
    fn test_parse_semver_invalid() {
        assert_eq!(parse_semver("not-a-version"), None);
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn test_compare_versions_up_to_date() {
        let status = compare_versions("0.1.0", "v0.1.0", "https://example.com");
        assert_eq!(status, VersionStatus::UpToDate);
    }

    #[test]
    fn test_compare_versions_update_available() {
        let status = compare_versions("0.1.0", "v0.2.0", "https://example.com/release");
        assert_eq!(
            status,
            VersionStatus::UpdateAvailable {
                latest: "v0.2.0".to_string(),
                release_url: "https://example.com/release".to_string(),
            }
        );
    }

    #[test]
    fn test_compare_versions_ahead_of_release() {
        let status = compare_versions("0.3.0", "v0.2.0", "https://example.com");
        assert_eq!(status, VersionStatus::AheadOfRelease);
    }

    #[test]
    fn test_compare_versions_no_leading_v() {
        // Remote tag without 'v' prefix should still work.
        let status = compare_versions("0.1.0", "0.1.0", "https://example.com");
        assert_eq!(status, VersionStatus::UpToDate);
    }

    #[test]
    fn test_compare_versions_patch_update() {
        let status = compare_versions("0.1.0", "v0.1.1", "https://example.com/patch");
        assert_eq!(
            status,
            VersionStatus::UpdateAvailable {
                latest: "v0.1.1".to_string(),
                release_url: "https://example.com/patch".to_string(),
            }
        );
    }

    #[test]
    fn test_compare_versions_major_update() {
        let status = compare_versions("0.9.9", "v1.0.0", "https://example.com/major");
        assert_eq!(
            status,
            VersionStatus::UpdateAvailable {
                latest: "v1.0.0".to_string(),
                release_url: "https://example.com/major".to_string(),
            }
        );
    }
}
