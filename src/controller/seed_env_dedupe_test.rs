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
//! Regression tests for duplicate container env var injection (#1556).
//!
//! A `StellarNode` can configure its validator seed twice: through the legacy
//! `spec.validatorConfig.seedSecretRef` and through
//! `spec.validatorConfig.seedSecretSource`. Both paths inject an env var
//! called `STELLAR_CORE_SEED`, and a container carrying the same env var name
//! twice is rejected by the API server. The pod builder therefore merges env
//! vars by name (`merge_env_overrides`) instead of appending, and these tests
//! pin that contract for every seed source style, every node type, and random
//! combinations of user overrides.

#[cfg(test)]
mod seed_env_dedupe {
    use std::collections::BTreeSet;

    use k8s_openapi::api::core::v1::{Container, EnvVar};
    use proptest::prelude::*;

    use super::super::kms_secret::SeedInjectionSpec;
    use super::super::resources::{build_deployment, build_statefulset};
    use crate::crd::seed_secret::{
        CsiSecretRef, LocalSecretRef, SeedSecretSource, VaultSecretRef, DEFAULT_SEED_KEY,
    };
    use crate::crd::types::ValidatorConfig;
    use crate::crd::{NodeType, StellarNetwork, StellarNode, StellarNodeSpec};

    /// Env var name both seed styles inject.
    const SEED_ENV_VAR: &str = "STELLAR_CORE_SEED";

    /// Name of the main container.
    const MAIN_CONTAINER: &str = "stellar-node";

    // ── Fixture helpers ────────────────────────────────────────────────────

    fn local_source(name: &str) -> SeedSecretSource {
        SeedSecretSource {
            local_ref: Some(LocalSecretRef {
                name: name.to_string(),
                key: None,
            }),
            external_ref: None,
            csi_ref: None,
            vault_ref: None,
        }
    }

    fn csi_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: None,
            external_ref: None,
            csi_ref: Some(CsiSecretRef {
                secret_provider_class_name: "stellar-seed".to_string(),
                mount_path: None,
                seed_file_name: None,
            }),
            vault_ref: None,
        }
    }

    fn vault_source() -> SeedSecretSource {
        SeedSecretSource {
            local_ref: None,
            external_ref: None,
            csi_ref: None,
            vault_ref: Some(VaultSecretRef {
                role: "stellar-validator".to_string(),
                secret_path: "secret/data/stellar/validator".to_string(),
                secret_key: None,
                secret_file_name: None,
                template: None,
                restart_on_secret_rotation: false,
                extra_pod_annotations: Vec::new(),
            }),
        }
    }

    /// The injection the reconciler would derive from a resolved seed source.
    fn injection_for(source: &SeedSecretSource) -> Option<SeedInjectionSpec> {
        if let Some(csi) = &source.csi_ref {
            return Some(SeedInjectionSpec::CsiMount { config: csi.clone() });
        }
        if let Some(vault) = &source.vault_ref {
            return Some(SeedInjectionSpec::VaultAgent {
                config: vault.clone(),
                pod_annotations: Default::default(),
            });
        }
        let local = source.local_ref.as_ref()?;
        Some(SeedInjectionSpec::EnvFromSecret {
            secret_name: local.name.clone(),
            secret_key: local.effective_key().to_string(),
        })
    }

    /// Build a node with the legacy seed ref, the new seed source, and any
    /// user-supplied container env overrides.
    fn seed_node(
        seed_secret_ref: &str,
        seed_secret_source: Option<SeedSecretSource>,
        overrides: Vec<EnvVar>,
    ) -> StellarNode {
        let mut node = StellarNode::new(
            "seed-env",
            StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                replicas: 1,
                stellar_core_env: overrides,
                validator_config: Some(ValidatorConfig {
                    seed_secret_ref: seed_secret_ref.to_string(),
                    seed_secret_source,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        node.metadata.namespace = Some("default".to_string());
        node
    }

    fn containers_for(node: &StellarNode, injection: Option<&SeedInjectionSpec>) -> Vec<Container> {
        let pod_spec = match node.spec.node_type {
            NodeType::Validator => build_statefulset(node, false, injection)
                .spec
                .expect("statefulset spec")
                .template
                .spec
                .expect("pod spec"),
            _ => build_deployment(node, false)
                .spec
                .expect("deployment spec")
                .template
                .spec
                .expect("pod spec"),
        };
        pod_spec.containers
    }

    fn main_env(node: &StellarNode, injection: Option<&SeedInjectionSpec>) -> Vec<EnvVar> {
        let containers = containers_for(node, injection);
        let main = containers
            .iter()
            .find(|c| c.name == MAIN_CONTAINER)
            .unwrap_or_else(|| panic!("{MAIN_CONTAINER} container must exist"));
        main.env.clone().unwrap_or_default()
    }

    fn seed_env_vars(env: &[EnvVar]) -> Vec<&EnvVar> {
        env.iter().filter(|e| e.name == SEED_ENV_VAR).collect()
    }

    /// The Secret a `STELLAR_CORE_SEED` env var reads from, if any.
    fn seed_secret_name(env: &[EnvVar]) -> Option<String> {
        seed_env_vars(env)
            .first()
            .and_then(|e| e.value_from.as_ref())
            .and_then(|source| source.secret_key_ref.as_ref())
            .and_then(|selector| selector.name.clone())
    }

    fn assert_unique_env_names(containers: &[Container]) {
        for container in containers {
            let names: Vec<&str> = container
                .env
                .iter()
                .flatten()
                .map(|e| e.name.as_str())
                .collect();
            let unique: BTreeSet<&str> = names.iter().copied().collect();
            assert_eq!(
                unique.len(),
                names.len(),
                "container {} has duplicate env var names: {names:?}",
                container.name
            );
        }
    }

    // ── The reported regression: both seed styles configured at once ───────

    /// With `seedSecretRef` and `seedSecretSource` both set, the rendered pod
    /// spec used to carry `STELLAR_CORE_SEED` twice, which the API server
    /// rejects.
    #[test]
    fn both_seed_styles_inject_a_single_seed_env_var() {
        let source = local_source("source-seed");
        let node = seed_node("legacy-seed", Some(source.clone()), Vec::new());
        let injection = injection_for(&source);

        let env = main_env(&node, injection.as_ref());

        assert_eq!(
            seed_env_vars(&env).len(),
            1,
            "{SEED_ENV_VAR} must be injected exactly once, got: {env:?}"
        );
    }

    /// `seedSecretSource` wins, matching `ValidatorConfig::resolve_seed_source`.
    #[test]
    fn seed_source_takes_precedence_over_the_legacy_ref() {
        let source = local_source("source-seed");
        let node = seed_node("legacy-seed", Some(source.clone()), Vec::new());
        let injection = injection_for(&source);

        let env = main_env(&node, injection.as_ref());

        assert_eq!(
            seed_secret_name(&env).as_deref(),
            Some("source-seed"),
            "seedSecretSource must win over seedSecretRef: {env:?}"
        );
    }

    // ── Each seed style on its own ─────────────────────────────────────────

    #[test]
    fn legacy_seed_ref_alone_injects_a_single_seed_env_var() {
        let node = seed_node("legacy-seed", None, Vec::new());

        let env = main_env(&node, None);

        assert_eq!(seed_env_vars(&env).len(), 1, "{env:?}");
        assert_eq!(seed_secret_name(&env).as_deref(), Some("legacy-seed"));
    }

    #[test]
    fn seed_source_local_ref_alone_injects_a_single_seed_env_var() {
        let source = local_source("source-seed");
        let node = seed_node("", Some(source.clone()), Vec::new());
        let injection = injection_for(&source);

        let env = main_env(&node, injection.as_ref());

        assert_eq!(seed_env_vars(&env).len(), 1, "{env:?}");
        assert_eq!(seed_secret_name(&env).as_deref(), Some("source-seed"));
    }

    /// CSI and Vault deliver the seed as a file, so `STELLAR_CORE_SEED` must be
    /// absent even when the legacy ref is also configured.
    #[test]
    fn file_backed_seed_sources_never_inject_the_seed_env_var() {
        for (label, source) in [("csiRef", csi_source()), ("vaultRef", vault_source())] {
            // Both styles configured: the file source must suppress the legacy env var.
            let node = seed_node("legacy-seed", Some(source.clone()), Vec::new());
            let injection = injection_for(&source);

            let env = main_env(&node, injection.as_ref());

            assert!(
                seed_env_vars(&env).is_empty(),
                "{label} delivers the seed as a file and must not also set {SEED_ENV_VAR}: {env:?}"
            );
            assert!(
                env.iter().any(|e| e.name == "STELLAR_SEED_FILE"),
                "{label} must set STELLAR_SEED_FILE: {env:?}"
            );
        }
    }

    // ── Audit: no other env var collisions across node types ───────────────

    /// Duplicate env var names are rejected by the API server, so every node
    /// type must render a container with unique env var names.
    #[test]
    fn env_var_names_are_unique_for_every_node_type() {
        for node_type in [NodeType::Validator, NodeType::Horizon, NodeType::SorobanRpc] {
            let mut node = StellarNode::new(
                "audit-node",
                StellarNodeSpec {
                    node_type: node_type.clone(),
                    network: StellarNetwork::Testnet,
                    version: "v21.0.0".to_string(),
                    replicas: 1,
                    ..Default::default()
                },
            );
            node.metadata.namespace = Some("default".to_string());
            node.spec.horizon_config = Some(crate::crd::types::HorizonConfig {
                stellar_core_url: "http://core:8000".to_string(),
                ..Default::default()
            });
            if node_type == NodeType::Validator {
                let source = local_source("source-seed");
                node.spec.validator_config = Some(ValidatorConfig {
                    seed_secret_ref: "legacy-seed".to_string(),
                    seed_secret_source: Some(source.clone()),
                    ..Default::default()
                });
                let injection = injection_for(&source);
                assert_unique_env_names(&containers_for(&node, injection.as_ref()));
            } else {
                assert_unique_env_names(&containers_for(&node, None));
            }
        }
    }

    // ── Property-based: random overrides never create a duplicate ──────────

    /// Env vars the operator always sets, and which a user override may
    /// therefore legitimately collide with.
    const OVERRIDABLE_ENV_VARS: [&str; 4] = [
        "NETWORK_PASSPHRASE",
        "STELLAR_CORE_WORKER_THREADS",
        "STELLAR_CORE_HTTP_QUERY_THREADS",
        SEED_ENV_VAR,
    ];

    fn env_override() -> impl Strategy<Value = EnvVar> {
        (proptest::sample::select(OVERRIDABLE_ENV_VARS.to_vec()), "[A-Za-z0-9_.-]{1,16}")
            .prop_map(|(name, value)| EnvVar {
                name: name.to_string(),
                value: Some(value),
                ..Default::default()
            })
    }

    fn seed_style() -> impl Strategy<Value = SeedSecretSource> {
        proptest::sample::select(vec![local_source("source-seed"), csi_source(), vault_source()])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Any combination of user env overrides and seed source styles must
        /// still render a container with unique env var names and at most one
        /// `STELLAR_CORE_SEED`.
        #[test]
        fn random_env_combinations_never_duplicate_env_var_names(
            overrides in proptest::collection::vec(env_override(), 0..8),
            legacy_ref in "[a-z0-9-]{0,12}",
            source in proptest::option::of(seed_style()),
        ) {
            let injection = source.as_ref().and_then(injection_for);
            let node = seed_node(&legacy_ref, source.clone(), overrides);

            let containers = containers_for(&node, injection.as_ref());
            for container in &containers {
                let names: Vec<&str> = container
                    .env
                    .iter()
                    .flatten()
                    .map(|e| e.name.as_str())
                    .collect();
                let unique: BTreeSet<&str> = names.iter().copied().collect();
                prop_assert_eq!(
                    unique.len(),
                    names.len(),
                    "duplicate env var names rendered in {}: {:?}",
                    container.name,
                    names
                );
            }

            let env = main_env(&node, injection.as_ref());
            prop_assert!(
                seed_env_vars(&env).len() <= 1,
                "at most one {} may be injected: {env:?}",
                SEED_ENV_VAR
            );
        }
    }
}
