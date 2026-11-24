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
//! Schema checks for the seed-rotation observability status fields (#1557).
//!
//! `StellarNodeStatus` declares `observedSeedSecretVersion`,
//! `observedPassphraseSecretVersion` and `lastSecretRotationTime`, but the
//! generated and Helm-bundled OpenAPI schemas must declare them too. A
//! structural schema that omits a status key makes the API server prune it on
//! every write, so `secret_watcher` would read back an empty observed version
//! and roll the pods on every reconcile.

#[cfg(test)]
mod secret_rotation_crd_schema {
    use std::fs;
    use std::path::PathBuf;

    use crate::crd::StellarNodeStatus;

    /// Status keys the seed-rotation watcher writes.
    const ROTATION_STATUS_FIELDS: [&str; 3] = [
        "observedSeedSecretVersion",
        "observedPassphraseSecretVersion",
        "lastSecretRotationTime",
    ];

    fn read_repo_file(relative: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"))
    }

    #[test]
    fn status_serializes_rotation_fields_in_camel_case() {
        let status = StellarNodeStatus {
            observed_seed_secret_version: Some("1001".to_string()),
            observed_passphrase_secret_version: Some("2002".to_string()),
            last_secret_rotation_time: Some("2026-09-26T10:00:00Z".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_value(&status).expect("status serializes");

        assert_eq!(json["observedSeedSecretVersion"], "1001");
        assert_eq!(json["observedPassphraseSecretVersion"], "2002");
        assert_eq!(json["lastSecretRotationTime"], "2026-09-26T10:00:00Z");
    }

    #[test]
    fn status_deserializes_rotation_fields_from_api_server_json() {
        let status: StellarNodeStatus = serde_json::from_value(serde_json::json!({
            "phase": "Ready",
            "observedSeedSecretVersion": "1001",
            "observedPassphraseSecretVersion": "2002",
            "lastSecretRotationTime": "2026-09-26T10:00:00Z",
        }))
        .expect("status deserializes");

        assert_eq!(
            status.observed_seed_secret_version.as_deref(),
            Some("1001")
        );
        assert_eq!(
            status.observed_passphrase_secret_version.as_deref(),
            Some("2002")
        );
        assert_eq!(
            status.last_secret_rotation_time.as_deref(),
            Some("2026-09-26T10:00:00Z")
        );
    }

    #[test]
    fn rotation_fields_are_omitted_when_unset() {
        let json = serde_json::to_value(StellarNodeStatus::default()).expect("status serializes");

        for field in ROTATION_STATUS_FIELDS {
            assert!(
                json.get(field).is_none(),
                "{field} must be omitted until the watcher records a rotation, got: {json}"
            );
        }
    }

    #[test]
    fn generated_crd_schema_declares_rotation_status_fields() {
        let yaml = read_repo_file("config/crd/stellarnode-crd.yaml");

        for field in ROTATION_STATUS_FIELDS {
            assert!(
                yaml.contains(&format!("{field}:")),
                "config/crd/stellarnode-crd.yaml status schema must declare {field}"
            );
        }
    }

    #[test]
    fn helm_crd_template_declares_rotation_status_fields() {
        let yaml = read_repo_file("charts/stellar-operator/templates/crd.yaml");

        for field in ROTATION_STATUS_FIELDS {
            assert!(
                yaml.contains(&format!("{field}:")),
                "charts/stellar-operator/templates/crd.yaml status schema must declare {field}"
            );
        }
    }

    #[test]
    fn extracted_json_schema_declares_rotation_status_fields() {
        let json = read_repo_file("schemas/crd/StellarNode-stellar.org-v1alpha1.json");
        let schema: serde_json::Value = serde_json::from_str(&json).expect("valid JSON schema");

        let properties = &schema["properties"]["status"]["properties"];
        for field in ROTATION_STATUS_FIELDS {
            assert_eq!(
                properties[field]["type"], "string",
                "kubeconform schema must accept {field} as a string"
            );
        }
    }
}
