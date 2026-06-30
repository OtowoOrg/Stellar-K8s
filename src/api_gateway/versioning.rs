//! API versioning and deprecation management.

use crate::api_gateway::config::VersioningConfig;

/// Outcome of a version check.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionStatus {
    Current,
    Deprecated { sunset_date: Option<String> },
    Sunset,
}

pub fn check_version(version: &str, cfg: &VersioningConfig) -> VersionStatus {
    if cfg.sunset_versions.iter().any(|v| v == version) {
        return VersionStatus::Sunset;
    }
    if cfg.deprecated_versions.iter().any(|v| v == version) {
        return VersionStatus::Deprecated { sunset_date: None };
    }
    VersionStatus::Current
}

/// Build the `Deprecation` and `Sunset` response headers for deprecated routes.
pub fn deprecation_headers(sunset_date: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![("Deprecation".into(), "true".into())];
    if let Some(date) = sunset_date {
        headers.push(("Sunset".into(), date.into()));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sunset() {
        let cfg = VersioningConfig {
            current_version: "v2".into(),
            deprecated_versions: vec!["v1".into()],
            sunset_versions: vec!["v0".into()],
        };
        assert_eq!(check_version("v0", &cfg), VersionStatus::Sunset);
        assert_eq!(
            check_version("v1", &cfg),
            VersionStatus::Deprecated { sunset_date: None }
        );
        assert_eq!(check_version("v2", &cfg), VersionStatus::Current);
    }
}
