// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
/// tests/common/fixtures.rs
///
/// Isolated, deterministic test fixtures for integration and unit test suites.
///
/// # Design (issue #1140, consolidated from tests/fixtures/mod.rs per #1196)
///
/// Every fixture function returns a fully-constructed value with sensible
/// defaults. Tests can customise via builder-style overrides. No fixture
/// function allocates cluster resources — that is the responsibility of the
/// test guards in `common/mod.rs`.
///
/// Fixture categories:
/// - `backup_*`       — `BackupVerificationConfig` and `BackupSource`
/// - `rotation_*`     — `SecretRotationConfig`
/// - `k8s_*`          — Kubernetes API objects (Pods, Containers, VolumeMounts)
///
/// Obsolete StellarNode YAML / deterministic helpers for deprecated
/// reconciliation integration paths were removed in issue #1218 (unused after
/// reconciler tests moved to typed `create_test_*` constructors).
use k8s_openapi::api::core::v1::{Container, VolumeMount};

// ---------------------------------------------------------------------------
// Kubernetes API object fixtures
// ---------------------------------------------------------------------------

/// A minimal init container with a name, image, and command.
pub fn init_container(name: &str, image: &str, command: Vec<&str>) -> Container {
    Container {
        name: name.to_string(),
        image: Some(image.to_string()),
        command: Some(command.into_iter().map(String::from).collect()),
        ..Default::default()
    }
}

/// An init container that mounts a named volume at the given path.
pub fn init_container_with_volume(
    name: &str,
    image: &str,
    volume_name: &str,
    mount_path: &str,
) -> Container {
    Container {
        name: name.to_string(),
        image: Some(image.to_string()),
        volume_mounts: Some(vec![VolumeMount {
            name: volume_name.to_string(),
            mount_path: mount_path.to_string(),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

/// A volume mount referencing the given volume at a path.
pub fn volume_mount(volume_name: &str, mount_path: &str) -> VolumeMount {
    VolumeMount {
        name: volume_name.to_string(),
        mount_path: mount_path.to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Backup / rotation fixtures
// ---------------------------------------------------------------------------

/// Returns a `BackupVerificationConfig` with all fields at documented defaults.
///
/// Use this instead of `BackupVerificationConfig::default()` directly so tests
/// are isolated from any future changes to the `Default` impl.
pub fn backup_verification_defaults() -> stellar_k8s::backup::BackupVerificationConfig {
    stellar_k8s::backup::BackupVerificationConfig {
        enabled: false,
        schedule: "0 2 * * 0".to_string(),
        timeout_minutes: 60,
        benchmark_enabled: false,
        strategy: stellar_k8s::backup::VerificationStrategy::Standard,
        ..Default::default()
    }
}

/// Returns a `BackupVerificationConfig` configured for a quick CI run.
pub fn backup_verification_quick() -> stellar_k8s::backup::BackupVerificationConfig {
    stellar_k8s::backup::BackupVerificationConfig {
        enabled: true,
        schedule: "*/5 * * * *".to_string(),
        timeout_minutes: 5,
        benchmark_enabled: false,
        strategy: stellar_k8s::backup::VerificationStrategy::Quick,
        ..Default::default()
    }
}

/// A `BackupSource::S3` pointing at a test bucket.
pub fn s3_backup_source() -> stellar_k8s::backup::BackupSource {
    stellar_k8s::backup::BackupSource::S3 {
        bucket: "stellar-it-test-bucket".to_string(),
        region: "us-east-1".to_string(),
        prefix: "integration-tests/".to_string(),
        credentials_secret: "aws-test-creds".to_string(),
    }
}

/// A `BackupSource::VolumeSnapshot` referencing a test snapshot.
pub fn volume_snapshot_backup_source() -> stellar_k8s::backup::BackupSource {
    stellar_k8s::backup::BackupSource::VolumeSnapshot {
        snapshot_name: "stellar-it-snapshot".to_string(),
        storage_class: "standard".to_string(),
    }
}

/// Returns a `SecretRotationConfig` with all fields at documented defaults.
pub fn secret_rotation_defaults() -> stellar_k8s::backup::SecretRotationConfig {
    stellar_k8s::backup::SecretRotationConfig {
        enabled: false,
        schedule: "0 0 1 * *".to_string(),
        password_length: 32,
        db_timeout_seconds: 30,
        max_retries: 3,
        audit_logging_enabled: false,
        audit_log_destination: None,
        notification_webhook: None,
    }
}

/// Returns a `SecretRotationConfig` with all features enabled, suitable for
/// testing the serialisation round-trip.
pub fn secret_rotation_full() -> stellar_k8s::backup::SecretRotationConfig {
    stellar_k8s::backup::SecretRotationConfig {
        enabled: true,
        schedule: "0 0 1 * *".to_string(),
        password_length: 40,
        db_timeout_seconds: 60,
        max_retries: 5,
        audit_logging_enabled: true,
        audit_log_destination: Some("https://audit.example.com".to_string()),
        notification_webhook: Some("https://webhook.example.com".to_string()),
    }
}
