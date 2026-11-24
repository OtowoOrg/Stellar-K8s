use serde::Deserialize;
use std::collections::BTreeSet;

const RELEASE_MATRIX: &str = include_str!("../config/caps.toml");

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CoreRelease {
    pub version: String,
    pub protocol: u32,
    pub xdr_version: u32,
    #[serde(default)]
    pub introduced_caps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseMatrix {
    releases: Vec<CoreRelease>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub compatible: bool,
    pub protocol_compatible: bool,
    pub xdr_compatible: bool,
    pub unsupported_caps: Vec<String>,
    pub recommended_version: Option<String>,
}

pub fn supported_releases() -> Result<Vec<CoreRelease>, toml::de::Error> {
    Ok(toml::from_str::<ReleaseMatrix>(RELEASE_MATRIX)?.releases)
}

/// Compare required CAPs with the support recorded for a core release.
pub fn check_compatibility(
    current_version: &str,
    current_protocol: u32,
    required_xdr_version: u32,
    required_caps: &[String],
) -> Result<CompatibilityReport, toml::de::Error> {
    let releases = supported_releases()?;
    let selected = releases
        .iter()
        .find(|release| release.version == current_version);
    let supported_protocol = selected.map(|release| release.protocol);
    let xdr_compatible = selected
        .map(|release| release.xdr_version >= required_xdr_version)
        .unwrap_or(false);
    let protocol_compatible =
        supported_protocol.is_some_and(|supported| supported >= current_protocol);
    let supported_caps: BTreeSet<&str> = releases
        .iter()
        .filter(|release| supported_protocol.is_some_and(|protocol| protocol >= release.protocol))
        .flat_map(|release| release.introduced_caps.iter().map(String::as_str))
        .collect();

    let mut unsupported_caps: Vec<String> = required_caps
        .iter()
        .filter(|cap| {
            let cap_protocol = releases
                .iter()
                .find(|release| release.introduced_caps.contains(cap))
                .map(|release| release.protocol);
            match (supported_protocol, cap_protocol) {
                (Some(supported), Some(required)) => supported < required,
                (Some(_), None) => !supported_caps.contains(cap.as_str()),
                (None, _) => true,
            }
        })
        .cloned()
        .collect();
    unsupported_caps.sort();
    unsupported_caps.dedup();

    let required_protocol = required_caps
        .iter()
        .filter_map(|cap| {
            releases
                .iter()
                .find(|release| release.introduced_caps.contains(cap))
                .map(|release| release.protocol)
        })
        .max()
        .unwrap_or(0);
    let compatible = protocol_compatible
        && xdr_compatible
        && current_protocol >= required_protocol
        && unsupported_caps.is_empty();

    let recommended_version = if compatible {
        None
    } else {
        releases
            .iter()
            .filter(|release| {
                release.protocol >= required_protocol.max(current_protocol)
                    && release.xdr_version >= required_xdr_version
                    && required_caps.iter().all(|cap| {
                        releases
                            .iter()
                            .find(|candidate| candidate.introduced_caps.contains(cap))
                            .is_some_and(|introduced| release.protocol >= introduced.protocol)
                    })
            })
            .min_by_key(|release| release.protocol)
            .map(|release| release.version.clone())
    };

    Ok(CompatibilityReport {
        compatible,
        protocol_compatible,
        xdr_compatible,
        unsupported_caps,
        recommended_version,
    })
}

#[cfg(test)]
mod tests {
    use super::{check_compatibility, supported_releases};

    #[test]
    fn matrix_covers_ten_core_releases() {
        assert_eq!(supported_releases().unwrap().len(), 10);
    }

    #[test]
    fn older_core_is_warned_with_a_recommendation() {
        let report = check_compatibility("v27.0.0", 27, 28, &["CAP-0085".to_string()]).unwrap();
        assert!(!report.compatible);
        assert!(!report.xdr_compatible);
        assert_eq!(report.unsupported_caps, ["CAP-0085"]);
        assert_eq!(report.recommended_version.as_deref(), Some("v28.0.1"));
    }

    #[test]
    fn checks_network_xdr_version_separately_from_protocol() {
        let report = check_compatibility("v28.0.1", 28, 27, &[]).unwrap();
        assert!(report.protocol_compatible);
        assert!(report.xdr_compatible);
        assert!(report.compatible);
    }

    #[test]
    fn unknown_cap_does_not_receive_a_false_recommendation() {
        let report = check_compatibility("v28.0.1", 28, 28, &["CAP-9999".to_string()]).unwrap();
        assert!(!report.compatible);
        assert_eq!(report.unsupported_caps, ["CAP-9999"]);
        assert_eq!(report.recommended_version, None);
    }

    #[test]
    fn recommends_the_lowest_release_satisfying_a_known_cap() {
        let report = check_compatibility("v26.0.0", 26, 26, &["CAP-0071".to_string()]).unwrap();
        assert!(!report.compatible);
        assert_eq!(report.recommended_version.as_deref(), Some("v27.1.0"));
    }

    #[test]
    fn does_not_recommend_a_release_below_the_network_protocol() {
        let report = check_compatibility("v28.0.1", 30, 28, &[]).unwrap();
        assert!(!report.compatible);
        assert_eq!(report.recommended_version, None);
    }
}
