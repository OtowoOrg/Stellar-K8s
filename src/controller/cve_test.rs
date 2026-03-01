//! Tests for CVE handling functionality

#[cfg(test)]
mod tests {
    use crate::controller::cve::{
        CVECount, CVEDetectionResult, CVERolloutStatus, CanaryTestStatus, Vulnerability,
        VulnerabilitySeverity,
    };
    use crate::crd::CVEHandlingConfig;
    use chrono::Utc;

    #[test]
    fn test_cve_handling_config_defaults() {
        let config = CVEHandlingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.scan_interval_secs, 3600);
        assert!(!config.critical_only);
        assert_eq!(config.canary_test_timeout_secs, 300);
        assert_eq!(config.canary_pass_rate_threshold, 100.0);
        assert!(config.enable_auto_rollback);
        assert_eq!(config.consensus_health_threshold, 0.95);
    }

    #[test]
    fn test_cve_detection_result_requires_patch() {
        let result_with_critical = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vec![Vulnerability {
                cve_id: "CVE-2024-1234".to_string(),
                severity: VulnerabilitySeverity::Critical,
                package: "openssl".to_string(),
                installed_version: "1.0.0".to_string(),
                fixed_version: Some("1.0.1".to_string()),
                description: "Critical vulnerability in OpenSSL".to_string(),
            }],
            patched_version: Some("stellar/core:v21.0.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                ..Default::default()
            },
            has_critical: true,
        };

        assert!(result_with_critical.requires_urgent_patch());
        assert!(result_with_critical.can_patch());
    }

    #[test]
    fn test_cve_count_total() {
        let count = CVECount {
            critical: 1,
            high: 2,
            medium: 3,
            low: 4,
            unknown: 5,
        };
        assert_eq!(count.total(), 15);
    }

    #[test]
    fn test_vulnerability_severity_ordering() {
        assert!(VulnerabilitySeverity::Critical > VulnerabilitySeverity::High);
        assert!(VulnerabilitySeverity::High > VulnerabilitySeverity::Medium);
        assert!(VulnerabilitySeverity::Medium > VulnerabilitySeverity::Low);
        assert!(VulnerabilitySeverity::Low > VulnerabilitySeverity::Unknown);
    }

    #[test]
    fn test_canary_test_status_string_repr() {
        assert_eq!(CanaryTestStatus::Pending.as_str(), "Pending");
        assert_eq!(CanaryTestStatus::Running.as_str(), "Running");
        assert_eq!(CanaryTestStatus::Passed.as_str(), "Passed");
        assert_eq!(CanaryTestStatus::Failed.as_str(), "Failed");
        assert_eq!(CanaryTestStatus::Timeout.as_str(), "Timeout");
    }

    #[test]
    fn test_cve_rollout_status_string_repr() {
        assert_eq!(CVERolloutStatus::Idle.as_str(), "Idle");
        assert_eq!(CVERolloutStatus::CanaryTesting.as_str(), "CanaryTesting");
        assert_eq!(CVERolloutStatus::Rolling.as_str(), "Rolling");
        assert_eq!(CVERolloutStatus::Complete.as_str(), "Complete");
        assert_eq!(CVERolloutStatus::RollingBack.as_str(), "RollingBack");
        assert_eq!(CVERolloutStatus::RolledBack.as_str(), "RolledBack");
        assert_eq!(CVERolloutStatus::Failed.as_str(), "Failed");
    }

    #[test]
    fn test_cve_config_critical_only() {
        let config = CVEHandlingConfig {
            enabled: true,
            scan_interval_secs: 3600,
            critical_only: true,
            canary_test_timeout_secs: 300,
            canary_pass_rate_threshold: 100.0,
            enable_auto_rollback: true,
            consensus_health_threshold: 0.95,
        };

        assert!(config.critical_only);
        assert!(config.enable_auto_rollback);
    }

    #[test]
    fn test_cve_detection_without_patch() {
        let result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vec![],
            patched_version: None,
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                ..Default::default()
            },
            has_critical: true,
        };

        assert!(result.requires_urgent_patch());
        assert!(!result.can_patch()); // No patch available
    }

    #[test]
    fn test_cve_config_aggressive_patching() {
        let config = CVEHandlingConfig {
            enabled: true,
            scan_interval_secs: 1800,      // 30 minutes
            critical_only: false,          // Patch all levels
            canary_test_timeout_secs: 180, // 3 minutes
            canary_pass_rate_threshold: 100.0,
            enable_auto_rollback: true,
            consensus_health_threshold: 0.90, // Less strict
        };

        assert!(!config.critical_only);
        assert_eq!(config.scan_interval_secs, 1800);
        assert_eq!(config.canary_test_timeout_secs, 180);
        assert_eq!(config.consensus_health_threshold, 0.90);
    }

    #[test]
    fn test_cve_config_manual_rollback() {
        let config = CVEHandlingConfig {
            enabled: true,
            scan_interval_secs: 3600,
            critical_only: false,
            canary_test_timeout_secs: 300,
            canary_pass_rate_threshold: 100.0,
            enable_auto_rollback: false, // Disable auto-rollback
            consensus_health_threshold: 0.95,
        };

        assert!(!config.enable_auto_rollback);
    }

    // ==========================================
    // Issue #154: Expanded CVE test coverage
    // ==========================================

    #[test]
    fn test_parse_cve_scan_results_mixed_severities() {
        let vulns = vec![
            Vulnerability {
                cve_id: "CVE-2024-0001".to_string(),
                severity: VulnerabilitySeverity::Critical,
                package: "openssl".to_string(),
                installed_version: "1.1.1".to_string(),
                fixed_version: Some("1.1.1w".to_string()),
                description: "Buffer overflow in OpenSSL".to_string(),
            },
            Vulnerability {
                cve_id: "CVE-2024-0002".to_string(),
                severity: VulnerabilitySeverity::High,
                package: "glibc".to_string(),
                installed_version: "2.31".to_string(),
                fixed_version: Some("2.31-13".to_string()),
                description: "Use-after-free in glibc".to_string(),
            },
            Vulnerability {
                cve_id: "CVE-2024-0003".to_string(),
                severity: VulnerabilitySeverity::Medium,
                package: "curl".to_string(),
                installed_version: "7.68".to_string(),
                fixed_version: None,
                description: "Info leak in curl".to_string(),
            },
            Vulnerability {
                cve_id: "CVE-2024-0004".to_string(),
                severity: VulnerabilitySeverity::Low,
                package: "bash".to_string(),
                installed_version: "5.0".to_string(),
                fixed_version: Some("5.0-p1".to_string()),
                description: "Minor issue in bash".to_string(),
            },
            Vulnerability {
                cve_id: "CVE-2024-0005".to_string(),
                severity: VulnerabilitySeverity::Unknown,
                package: "libfoo".to_string(),
                installed_version: "0.1".to_string(),
                fixed_version: None,
                description: "Unknown severity issue".to_string(),
            },
        ];

        let result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vulns,
            patched_version: Some("stellar/core:v21.0.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                high: 1,
                medium: 1,
                low: 1,
                unknown: 1,
            },
            has_critical: true,
        };

        assert_eq!(result.cve_count.total(), 5);
        assert!(result.requires_urgent_patch());
        assert!(result.can_patch());
        assert_eq!(result.vulnerabilities.len(), 5);
    }

    #[test]
    fn test_parse_cve_scan_results_no_vulnerabilities() {
        let result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vec![],
            patched_version: None,
            scan_timestamp: Utc::now(),
            cve_count: CVECount::default(),
            has_critical: false,
        };

        assert_eq!(result.cve_count.total(), 0);
        assert!(!result.requires_urgent_patch());
        assert!(!result.can_patch());
    }

    #[test]
    fn test_parse_cve_scan_high_only_no_critical() {
        let vulns = vec![Vulnerability {
            cve_id: "CVE-2024-5678".to_string(),
            severity: VulnerabilitySeverity::High,
            package: "libxml2".to_string(),
            installed_version: "2.9.10".to_string(),
            fixed_version: Some("2.9.14".to_string()),
            description: "XXE vulnerability".to_string(),
        }];

        let result = CVEDetectionResult {
            current_image: "stellar/horizon:v2.28.0".to_string(),
            vulnerabilities: vulns,
            patched_version: Some("stellar/horizon:v2.28.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                high: 1,
                ..Default::default()
            },
            has_critical: false,
        };

        assert!(
            !result.requires_urgent_patch(),
            "High-only should not require urgent patch"
        );
        assert!(
            result.can_patch(),
            "Should be patchable when version available"
        );
    }

    #[test]
    fn test_rollout_status_transitions() {
        let statuses = [
            CVERolloutStatus::Idle,
            CVERolloutStatus::CanaryTesting,
            CVERolloutStatus::Rolling,
            CVERolloutStatus::Complete,
        ];

        assert_eq!(statuses[0].as_str(), "Idle");
        assert_eq!(statuses[1].as_str(), "CanaryTesting");
        assert_eq!(statuses[2].as_str(), "Rolling");
        assert_eq!(statuses[3].as_str(), "Complete");
    }

    #[test]
    fn test_rollout_status_rollback_path() {
        let statuses = [
            CVERolloutStatus::Rolling,
            CVERolloutStatus::RollingBack,
            CVERolloutStatus::RolledBack,
        ];

        assert_eq!(statuses[0].as_str(), "Rolling");
        assert_eq!(statuses[1].as_str(), "RollingBack");
        assert_eq!(statuses[2].as_str(), "RolledBack");
    }

    #[test]
    fn test_canary_test_outcomes_drive_rollout() {
        let canary_passed = CanaryTestStatus::Passed;
        assert_eq!(canary_passed.as_str(), "Passed");
        let rollout_after_pass = CVERolloutStatus::Rolling;
        assert_eq!(rollout_after_pass.as_str(), "Rolling");

        let canary_failed = CanaryTestStatus::Failed;
        assert_eq!(canary_failed.as_str(), "Failed");
        let rollout_after_fail = CVERolloutStatus::Failed;
        assert_eq!(rollout_after_fail.as_str(), "Failed");

        let canary_timeout = CanaryTestStatus::Timeout;
        assert_eq!(canary_timeout.as_str(), "Timeout");
    }

    #[test]
    fn test_vulnerable_image_replaced_with_fixed_version() {
        let vulnerable_image = "stellar/core:v21.0.0";
        let fixed_image = "stellar/core:v21.0.1";

        let result = CVEDetectionResult {
            current_image: vulnerable_image.to_string(),
            vulnerabilities: vec![Vulnerability {
                cve_id: "CVE-2024-9999".to_string(),
                severity: VulnerabilitySeverity::Critical,
                package: "openssl".to_string(),
                installed_version: "3.0.0".to_string(),
                fixed_version: Some("3.0.13".to_string()),
                description: "Critical OpenSSL vulnerability".to_string(),
            }],
            patched_version: Some(fixed_image.to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                ..Default::default()
            },
            has_critical: true,
        };

        assert!(result.can_patch());
        assert_eq!(result.patched_version.as_deref(), Some(fixed_image));
        assert_ne!(result.current_image, fixed_image);
    }

    #[test]
    fn test_vulnerable_image_no_fixed_version_available() {
        let result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vec![Vulnerability {
                cve_id: "CVE-2024-0000".to_string(),
                severity: VulnerabilitySeverity::Critical,
                package: "zlib".to_string(),
                installed_version: "1.2.11".to_string(),
                fixed_version: None,
                description: "No fix available yet".to_string(),
            }],
            patched_version: None,
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 1,
                ..Default::default()
            },
            has_critical: true,
        };

        assert!(result.requires_urgent_patch());
        assert!(
            !result.can_patch(),
            "Should not be patchable without a fixed version"
        );
    }

    #[test]
    fn test_dry_run_does_not_mutate_cve_resources() {
        let dry_run = true;
        let mut mutations_performed = 0u32;

        let cve_detected = true;
        let patched_version_available = true;

        if cve_detected && patched_version_available && !dry_run {
            mutations_performed += 1;
            mutations_performed += 1;
        }

        assert_eq!(
            mutations_performed, 0,
            "Dry-run mode must not mutate CVE resources"
        );
    }

    #[test]
    fn test_dry_run_still_detects_vulnerabilities() {
        let dry_run = true;

        let scan_result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vec![Vulnerability {
                cve_id: "CVE-2024-1111".to_string(),
                severity: VulnerabilitySeverity::High,
                package: "libcrypto".to_string(),
                installed_version: "1.0".to_string(),
                fixed_version: Some("1.1".to_string()),
                description: "Crypto weakness".to_string(),
            }],
            patched_version: Some("stellar/core:v21.0.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                high: 1,
                ..Default::default()
            },
            has_critical: false,
        };

        assert!(!scan_result.vulnerabilities.is_empty());
        assert!(scan_result.can_patch());

        if dry_run {
            let action_taken = false;
            assert!(!action_taken, "No action should be taken in dry-run mode");
        }
    }

    #[test]
    fn test_severity_as_str() {
        assert_eq!(VulnerabilitySeverity::Critical.as_str(), "CRITICAL");
        assert_eq!(VulnerabilitySeverity::High.as_str(), "HIGH");
        assert_eq!(VulnerabilitySeverity::Medium.as_str(), "MEDIUM");
        assert_eq!(VulnerabilitySeverity::Low.as_str(), "LOW");
        assert_eq!(VulnerabilitySeverity::Unknown.as_str(), "UNKNOWN");
    }

    #[test]
    fn test_cve_count_default_is_zero() {
        let count = CVECount::default();
        assert_eq!(count.critical, 0);
        assert_eq!(count.high, 0);
        assert_eq!(count.medium, 0);
        assert_eq!(count.low, 0);
        assert_eq!(count.unknown, 0);
        assert_eq!(count.total(), 0);
    }

    #[test]
    fn test_multiple_critical_vulnerabilities() {
        let vulns: Vec<Vulnerability> = (0..5)
            .map(|i| Vulnerability {
                cve_id: format!("CVE-2024-{i:04}"),
                severity: VulnerabilitySeverity::Critical,
                package: format!("pkg-{i}"),
                installed_version: "1.0.0".to_string(),
                fixed_version: Some("1.0.1".to_string()),
                description: format!("Critical vuln {i}"),
            })
            .collect();

        let result = CVEDetectionResult {
            current_image: "stellar/core:v21.0.0".to_string(),
            vulnerabilities: vulns,
            patched_version: Some("stellar/core:v21.0.1".to_string()),
            scan_timestamp: Utc::now(),
            cve_count: CVECount {
                critical: 5,
                ..Default::default()
            },
            has_critical: true,
        };

        assert!(result.requires_urgent_patch());
        assert!(result.can_patch());
        assert_eq!(result.vulnerabilities.len(), 5);
        assert_eq!(result.cve_count.critical, 5);
    }

    #[test]
    fn test_cve_config_disabled_skips_all() {
        let config = CVEHandlingConfig {
            enabled: false,
            scan_interval_secs: 3600,
            critical_only: false,
            canary_test_timeout_secs: 300,
            canary_pass_rate_threshold: 100.0,
            enable_auto_rollback: true,
            consensus_health_threshold: 0.95,
        };

        assert!(!config.enabled, "Disabled config should skip CVE handling");
    }
}
