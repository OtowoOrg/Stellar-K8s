//! Pod template builders.

use super::prelude::*;
use super::helpers::*;

// ============================================================================
// Pod Template Builder
// ============================================================================

/// Build the pod template.
///
/// `seed_injection` is `Some` only for Validator StatefulSets; it adds the
/// env vars / volumes / mounts required to deliver the seed from KMS/ESO/CSI.
pub(crate) fn build_pod_template(
    node: &StellarNode,
    labels: &BTreeMap<String, String>,
    enable_mtls: bool,
    // *** NEW PARAMETER ***
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
) -> PodTemplateSpec {
    let mut pod_spec = PodSpec {
        containers: vec![build_container(node, enable_mtls)],
        volumes: Some(vec![
            Volume {
                name: "data".to_string(),
                persistent_volume_claim: Some(
                    k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                        claim_name: resource_name(node, "data"),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
            Volume {
                name: "config".to_string(),
                config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                    name: Some(resource_name(node, "config")),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        topology_spread_constraints: Some(build_topology_spread_constraints(
            &node.spec,
            &node.name_any(),
        )),
        affinity: merge_workload_affinity(node),
        tolerations: build_workload_tolerations(node),
        security_context: Some(PodSecurityContext {
            run_as_non_root: Some(true),
            run_as_user: Some(10000),
            run_as_group: Some(10000),
            fs_group: Some(10000),
            seccomp_profile: Some(SeccompProfile {
                localhost_profile: None,
                type_: "RuntimeDefault".to_string(),
            }),
            ..Default::default()
        }),
        priority_class_name: node.spec.priority_class_name.clone(),
        ..Default::default()
    };

    if let Some(custom_volumes) = &node.spec.volumes {
        let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
        volumes.extend(custom_volumes.clone());
    }

    if node.spec.node_type == NodeType::Validator {
        if let Some(fs) = &node.spec.forensic_snapshot {
            if fs.enable_share_process_namespace {
                pod_spec.share_process_namespace = Some(true);
            }
        }
    }

    // Add Horizon database migration init container
    if let NodeType::Horizon = node.spec.node_type {
        if let Some(horizon_config) = &node.spec.horizon_config {
            let blue_green_migration =
                node.spec.strategy.strategy_type == RolloutStrategyType::BlueGreen;
            if horizon_config.auto_migration && !blue_green_migration {
                let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                init_containers.push(build_horizon_migration_container(node));
            }
        }
    }

    // -------------------------------------------------------------------------
    // Snapshot / compressed-backup restore init container
    //
    // Injected when `spec.storage.snapshotRef.backupUrl` is set.  The init
    // container downloads and extracts the archive into /data before Stellar
    // Core starts, enabling near-instant bootstrap from a compressed DB backup.
    // CSI VolumeSnapshot restores are handled at the PVC level (dataSource) and
    // do NOT need an init container.
    // -------------------------------------------------------------------------
    if let Some(snapshot_ref) = &node.spec.storage.snapshot_ref {
        if let Some(backup_url) = &snapshot_ref.backup_url {
            let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
            init_containers.push(build_snapshot_restore_container(
                node,
                backup_url,
                snapshot_ref.credentials_secret_ref.as_deref(),
                snapshot_ref.restore_image.as_deref(),
            ));
        }
    }

    // Add KMS init container if needed (Validator nodes only)
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.key_source == KeySource::KMS {
                if let Some(kms_config) = &validator_config.kms_config {
                    let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
                    volumes.push(Volume {
                        name: "keys".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                    init_containers.push(Container {
                        name: "kms-fetcher".to_string(),
                        image: Some(
                            kms_config
                                .fetcher_image
                                .clone()
                                .unwrap_or_else(|| "stellar/kms-fetcher:latest".to_string()),
                        ),
                        env: Some(vec![
                            EnvVar {
                                name: "KMS_KEY_ID".to_string(),
                                value: Some(kms_config.key_id.clone()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KMS_PROVIDER".to_string(),
                                value: Some(kms_config.provider.clone()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KMS_REGION".to_string(),
                                value: kms_config.region.clone(),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KEY_OUTPUT_PATH".to_string(),
                                value: Some("/keys/validator-seed".to_string()),
                                ..Default::default()
                            },
                        ]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "keys".to_string(),
                            mount_path: "/keys".to_string(),
                            ..Default::default()
                        }]),
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add state-sync sidecar if enabled
    if let Some(dr_config) = &node.spec.dr_config {
        if dr_config.enabled
            && dr_config.sync_strategy == crate::crd::DRSyncStrategy::StreamingLedger
        {
            let sidecar = super::state_sync::build_state_sync_sidecar(node);
            pod_spec.containers.push(sidecar);
        }
    }

    // Add mTLS certificate volume
    let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
    volumes.push(Volume {
        name: "tls".to_string(),
        secret: Some(k8s_openapi::api::core::v1::SecretVolumeSource {
            secret_name: Some(format!("{}-client-cert", node.name_any())),
            ..Default::default()
        }),
        ..Default::default()
    });

    // Add Cloud HSM sidecar and volumes
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if let Some(hsm_config) = &validator_config.hsm_config {
                if hsm_config.provider == HsmProvider::AWS {
                    volumes.push(Volume {
                        name: "cloudhsm-socket".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    let containers = &mut pod_spec.containers;
                    containers.push(Container {
                        name: "cloudhsm-client".to_string(),
                        image: Some("amazon/cloudhsm-client:latest".to_string()),
                        command: Some(vec!["/opt/cloudhsm/bin/cloudhsm_client".to_string()]),
                        args: Some(vec!["--foreground".to_string()]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "cloudhsm-socket".to_string(),
                            mount_path: "/var/run/cloudhsm".to_string(),
                            ..Default::default()
                        }]),
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                } else if hsm_config.provider == HsmProvider::Azure {
                    volumes.push(Volume {
                        name: "dedicatedhsm-socket".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    let containers = &mut pod_spec.containers;
                    containers.push(Container {
                        name: "dedicatedhsm-client".to_string(),
                        image: Some("azure/dedicated-hsm-client:latest".to_string()),
                        command: Some(
                            vec!["/opt/dedicatedhsm/bin/dedicatedhsm_client".to_string()],
                        ),
                        args: Some(vec!["--foreground".to_string()]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "dedicatedhsm-socket".to_string(),
                            mount_path: "/var/run/dedicatedhsm".to_string(),
                            ..Default::default()
                        }]),
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add NAT traversal sidecar
    if let Some(nat_cfg) = &node.spec.nat_traversal {
        if nat_cfg.enabled {
            let mut env = vec![EnvVar {
                name: "ENABLE_ICE".to_string(),
                value: Some(nat_cfg.enable_ice.to_string()),
                ..Default::default()
            }];

            if let Some(stun) = &nat_cfg.stun_server {
                env.push(EnvVar {
                    name: "STUN_SERVER".to_string(),
                    value: Some(stun.clone()),
                    ..Default::default()
                });
            }

            if let Some(turn) = &nat_cfg.turn_server {
                env.push(EnvVar {
                    name: "TURN_SERVER".to_string(),
                    value: Some(turn.clone()),
                    ..Default::default()
                });
            }

            if let Some(secret_ref) = &nat_cfg.turn_credentials_secret_ref {
                env.push(EnvVar {
                    name: "TURN_USERNAME".to_string(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: Some(secret_ref.clone()),
                            key: "username".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                });
                env.push(EnvVar {
                    name: "TURN_PASSWORD".to_string(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: Some(secret_ref.clone()),
                            key: "password".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                });
            }

            let sidecar_image = nat_cfg
                .sidecar_image
                .clone()
                .unwrap_or_else(|| "stellar/nat-traversal:latest".to_string());

            let containers = &mut pod_spec.containers;
            containers.push(Container {
                name: "nat-traversal".to_string(),
                image: Some(sidecar_image),
                env: Some(env),
                security_context: Some(SecurityContext {
                    allow_privilege_escalation: Some(false),
                    capabilities: Some(Capabilities {
                        drop: Some(vec!["ALL".to_string()]),
                        add: None,
                    }),
                    run_as_non_root: Some(true),
                    privileged: Some(false),
                    seccomp_profile: Some(SeccompProfile {
                        type_: "RuntimeDefault".to_string(),
                        localhost_profile: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }

    // ==========================================================================
    // Merge user-defined sidecar containers into the pod spec
    // ==========================================================================
    if let Some(sidecars) = &node.spec.sidecars {
        pod_spec.containers.extend(sidecars.iter().cloned());
    }

    // ==========================================================================
    // Append user-defined init containers after all operator-managed ones
    // ==========================================================================
    if let Some(user_init_containers) = &node.spec.init_containers {
        pod_spec
            .init_containers
            .get_or_insert_with(Vec::new)
            .extend(user_init_containers.iter().cloned());
    }

    // ==========================================================================
    // Inject hitless-upgrade handoff sidecar (Validators only, when enabled)
    // ==========================================================================
    if let Some(hu_config) = &node.spec.hitless_upgrade {
        if hu_config.enabled && node.spec.node_type == NodeType::Validator {
            let sidecar_image = hu_config
                .sidecar_image
                .clone()
                .unwrap_or_else(|| "stellar-k8s/handoff-sidecar:latest".to_string());

            // Shared emptyDir volume for the Unix domain socket
            let handoff_vol = k8s_openapi::api::core::v1::Volume {
                name: "handoff-socket".to_string(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            };
            pod_spec
                .volumes
                .get_or_insert_with(Vec::new)
                .push(handoff_vol);

            let handoff_mount = k8s_openapi::api::core::v1::VolumeMount {
                name: "handoff-socket".to_string(),
                mount_path: "/handoff".to_string(),
                ..Default::default()
            };

            // Mount the handoff volume into the main container as well
            if let Some(main_container) = pod_spec.containers.first_mut() {
                main_container
                    .volume_mounts
                    .get_or_insert_with(Vec::new)
                    .push(handoff_mount.clone());
            }

            let handoff_sidecar = k8s_openapi::api::core::v1::Container {
                name: "stellar-handoff".to_string(),
                image: Some(sidecar_image),
                args: Some(vec![
                    "handoff".to_string(),
                    "--socket".to_string(),
                    "/handoff/sock".to_string(),
                    "--timeout".to_string(),
                    hu_config.handoff_timeout_seconds.to_string(),
                ]),
                volume_mounts: Some(vec![handoff_mount]),
                liveness_probe: Some(k8s_openapi::api::core::v1::Probe {
                    http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
                        path: Some("/healthz".to_string()),
                        port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8080),
                        ..Default::default()
                    }),
                    initial_delay_seconds: Some(5),
                    period_seconds: Some(10),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pod_spec.containers.push(handoff_sidecar);
        }
    }

    // ==========================================================================
    // Inject health check sidecar for advanced liveness/readiness probes
    // ==========================================================================
    let health_check_sidecar = k8s_openapi::api::core::v1::Container {
        name: "stellar-health-check".to_string(),
        image: Some(
            node.spec
                .container_image()
                .replace("stellar-core", "stellar-k8s")
                .replace("horizon", "stellar-k8s"),
        ),
        command: Some(vec!["/stellar-health-sidecar".to_string()]),
        ports: Some(vec![k8s_openapi::api::core::v1::ContainerPort {
            name: Some("health".to_string()),
            container_port: 8081,
            protocol: Some("TCP".to_string()),
            ..Default::default()
        }]),
        env: Some(vec![
            EnvVar {
                name: "CORE_URL".to_string(),
                value: Some(match node.spec.node_type {
                    NodeType::Validator => "http://localhost:11626".to_string(),
                    NodeType::Horizon => "http://localhost:8000".to_string(),
                    NodeType::SorobanRpc => "http://localhost:8000".to_string(),
                }),
                ..Default::default()
            },
            EnvVar {
                name: "RUST_LOG".to_string(),
                value: Some("info".to_string()),
                ..Default::default()
            },
        ]),
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                add: None,
            }),
            run_as_non_root: Some(true),
            privileged: Some(false),
            read_only_root_filesystem: Some(true),
            seccomp_profile: Some(SeccompProfile {
                type_: "RuntimeDefault".to_string(),
                localhost_profile: None,
            }),
            ..Default::default()
        }),
        resources: Some(build_diagnostic_sidecar_resources(
            node.spec.diagnostic_sidecar_resources.as_ref(),
        )),
        ..Default::default()
    };
    pod_spec.containers.push(health_check_sidecar);

    // ==========================================================================
    // NEW: Inject KMS/ESO/CSI seed env vars, volumes, and volume mounts
    // ==========================================================================
    if let Some(inj) = seed_injection {
        // Extend the main container (index 0) with seed env vars and volume mounts
        if let Some(container) = pod_spec.containers.first_mut() {
            if let Some(ref mut env) = container.env {
                env.extend(inj.env_vars());
            } else {
                container.env = Some(inj.env_vars());
            }
            if let Some(ref mut mounts) = container.volume_mounts {
                mounts.extend(inj.volume_mounts());
            } else {
                let vm = inj.volume_mounts();
                if !vm.is_empty() {
                    container.volume_mounts = Some(vm);
                }
            }
        }
        // Extend pod volumes with any CSI volume
        if let Some(ref mut vols) = pod_spec.volumes {
            vols.extend(inj.volumes());
        }
    }
    // ==========================================================================

    // ==========================================================================
    // Inject log-shipper sidecar when spec.logShipper.enabled == true
    // ==========================================================================
    if let Some(ls) = &node.spec.log_shipper {
        if ls.enabled {
            // Shared emptyDir volume for log files written by the main container.
            pod_spec.volumes.get_or_insert_with(Vec::new).push(Volume {
                name: "stellar-logs".to_string(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            });

            // Mount the shared log volume into the main container.
            if let Some(main) = pod_spec.containers.first_mut() {
                main.volume_mounts
                    .get_or_insert_with(Vec::new)
                    .push(VolumeMount {
                        name: "stellar-logs".to_string(),
                        mount_path: "/var/log/stellar".to_string(),
                        ..Default::default()
                    });
            }

            // Build env vars for the sidecar.
            let mut env = vec![
                EnvVar {
                    name: "S3_BUCKET".to_string(),
                    value: Some(ls.s3_bucket.clone()),
                    ..Default::default()
                },
                EnvVar {
                    name: "S3_PREFIX".to_string(),
                    value: Some(
                        ls.s3_prefix
                            .clone()
                            .unwrap_or_else(|| "stellar-logs".to_string()),
                    ),
                    ..Default::default()
                },
                EnvVar {
                    name: "S3_REGION".to_string(),
                    value: Some(
                        ls.s3_region
                            .clone()
                            .unwrap_or_else(|| "us-east-1".to_string()),
                    ),
                    ..Default::default()
                },
                EnvVar {
                    name: "BATCH_SIZE_LINES".to_string(),
                    value: Some(ls.batch_size_lines.to_string()),
                    ..Default::default()
                },
                EnvVar {
                    name: "FLUSH_INTERVAL_SECS".to_string(),
                    value: Some(ls.flush_interval_secs.to_string()),
                    ..Default::default()
                },
                // Kubernetes downward API: inject the pod name as NODE_NAME.
                EnvVar {
                    name: "NODE_NAME".to_string(),
                    value_from: Some(EnvVarSource {
                        field_ref: Some(k8s_openapi::api::core::v1::ObjectFieldSelector {
                            field_path: "metadata.name".to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ];

            // Inject AWS credentials from a Secret if specified.
            if let Some(secret_ref) = &ls.credentials_secret_ref {
                for key in &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"] {
                    env.push(EnvVar {
                        name: key.to_string(),
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: key.to_string(),
                                optional: Some(false),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }

            let sidecar_image = ls.image.clone().unwrap_or_else(|| {
                format!("ghcr.io/stellar/stellar-k8s:{}", env!("CARGO_PKG_VERSION"))
            });

            pod_spec.containers.push(Container {
                name: "stellar-log-shipper".to_string(),
                image: Some(sidecar_image),
                command: Some(vec!["/stellar-log-shipper".to_string()]),
                env: Some(env),
                volume_mounts: Some(vec![VolumeMount {
                    name: "stellar-logs".to_string(),
                    mount_path: "/var/log/stellar".to_string(),
                    read_only: Some(true),
                    ..Default::default()
                }]),
                resources: Some(K8sResources {
                    requests: Some(
                        [
                            ("cpu".to_string(), Quantity("50m".to_string())),
                            ("memory".to_string(), Quantity("32Mi".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    limits: Some(
                        [
                            ("cpu".to_string(), Quantity("200m".to_string())),
                            ("memory".to_string(), Quantity("128Mi".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                }),
                security_context: Some(SecurityContext {
                    allow_privilege_escalation: Some(false),
                    read_only_root_filesystem: Some(true),
                    run_as_non_root: Some(true),
                    capabilities: Some(Capabilities {
                        drop: Some(vec!["ALL".to_string()]),
                        add: None,
                    }),
                    seccomp_profile: Some(SeccompProfile {
                        type_: "RuntimeDefault".to_string(),
                        localhost_profile: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }
    // NEW: Inject ebpf-exporter sidecar (Validators only, when enabled)
    // ==========================================================================
    if let Some(ebpf_cfg) = &node.spec.ebpf_config {
        if ebpf_cfg.enabled && node.spec.node_type == NodeType::Validator {
            let exporter_args = vec!["--config.file=/ebpf/ebpf-exporter.yaml".to_string()];

            let sidecar_image = "cloudflare/ebpf_exporter:latest".to_string();

            let ebpf_container = k8s_openapi::api::core::v1::Container {
                name: "ebpf-exporter".to_string(),
                image: Some(sidecar_image),
                args: Some(exporter_args),
                ports: Some(vec![k8s_openapi::api::core::v1::ContainerPort {
                    name: Some("metrics".to_string()),
                    container_port: 9435,
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
                volume_mounts: Some(vec![
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "config".to_string(),
                        mount_path: "/ebpf".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "sys-kernel-debug".to_string(),
                        mount_path: "/sys/kernel/debug".to_string(),
                        read_only: Some(false),
                        ..Default::default()
                    },
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "lib-modules".to_string(),
                        mount_path: "/lib/modules".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                ]),
                security_context: Some(SecurityContext {
                    privileged: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pod_spec.containers.push(ebpf_container);

            let vols = pod_spec.volumes.get_or_insert_with(Vec::new);
            vols.push(k8s_openapi::api::core::v1::Volume {
                name: "sys-kernel-debug".to_string(),
                host_path: Some(k8s_openapi::api::core::v1::HostPathVolumeSource {
                    path: "/sys/kernel/debug".to_string(),
                    type_: Some("DirectoryOrCreate".to_string()),
                }),
                ..Default::default()
            });
            vols.push(k8s_openapi::api::core::v1::Volume {
                name: "lib-modules".to_string(),
                host_path: Some(k8s_openapi::api::core::v1::HostPathVolumeSource {
                    path: "/lib/modules".to_string(),
                    type_: Some("Directory".to_string()),
                }),
                ..Default::default()
            });
        }
    }
    // ==========================================================================

    let mut apparmor_annotations = BTreeMap::new();
    if let Some(containers) = &pod_spec.init_containers {
        for container in containers {
            apparmor_annotations.insert(
                format!(
                    "container.apparmor.security.beta.kubernetes.io/{}",
                    container.name
                ),
                "runtime/default".to_string(),
            );
        }
    }
    for container in &pod_spec.containers {
        apparmor_annotations.insert(
            format!(
                "container.apparmor.security.beta.kubernetes.io/{}",
                container.name
            ),
            "runtime/default".to_string(),
        );
    }

    let mut pod_object_meta = ObjectMeta {
        labels: Some(labels.clone()),
        annotations: if apparmor_annotations.is_empty() {
            None
        } else {
            Some(apparmor_annotations)
        },
        ..Default::default()
    };
    if let Some(inj) = seed_injection {
        if let Some(ann) = inj.pod_annotations() {
            let mut merged = pod_object_meta.annotations.unwrap_or_default();
            merged.extend(ann.iter().map(|(k, v)| (k.clone(), v.clone())));
            pod_object_meta.annotations = Some(merged);
        }
    }

    // ── Soroban RPC multi-layer cache ─────────────────────────────────────────
    // When cache_config is set, provision an emptyDir volume backed by the
    // node's local SSD and inject cache path / size env vars into the main
    // container so the Soroban RPC process can locate the cache directory.
    if node.spec.node_type == NodeType::SorobanRpc {
        if let Some(soroban_cfg) = &node.spec.soroban_config {
            if let Some(cache_cfg) = &soroban_cfg.cache_config {
                // Add emptyDir volume (uses node-local ephemeral storage).
                let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
                volumes.push(Volume {
                    name: "soroban-cache".to_string(),
                    empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                        size_limit: Some(Quantity(format!("{}", cache_cfg.l2_max_bytes))),
                        ..Default::default()
                    }),
                    ..Default::default()
                });

                // Mount the volume and inject env vars into the main container.
                if let Some(container) = pod_spec.containers.first_mut() {
                    let mounts = container.volume_mounts.get_or_insert_with(Vec::new);
                    mounts.push(VolumeMount {
                        name: "soroban-cache".to_string(),
                        mount_path: cache_cfg.l2_path.clone(),
                        ..Default::default()
                    });

                    let env = container.env.get_or_insert_with(Vec::new);
                    env.push(EnvVar {
                        name: "SOROBAN_CACHE_PATH".to_string(),
                        value: Some(cache_cfg.l2_path.clone()),
                        ..Default::default()
                    });
                    env.push(EnvVar {
                        name: "SOROBAN_CACHE_MAX_BYTES".to_string(),
                        value: Some(cache_cfg.l2_max_bytes.to_string()),
                        ..Default::default()
                    });
                    env.push(EnvVar {
                        name: "SOROBAN_CACHE_L1_CAPACITY".to_string(),
                        value: Some(cache_cfg.l1_capacity.to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    PodTemplateSpec {
        metadata: Some(merge_resource_meta(
            pod_object_meta,
            &node.spec.resource_meta,
        )),
        spec: Some(pod_spec),
    }
}

fn parse_cpu_millicores(cpu: &str) -> Option<u32> {
    let trimmed = cpu.trim();
    if let Some(milli) = trimmed.strip_suffix('m') {
        return milli.parse::<u32>().ok();
    }

    let cores = trimmed.parse::<f64>().ok()?;
    if cores.is_sign_negative() {
        return None;
    }

    Some((cores * 1000.0).round() as u32)
}

fn derive_worker_threads(node: &StellarNode) -> u32 {
    let millicores = parse_cpu_millicores(&node.spec.resources.limits.cpu)
        .or_else(|| parse_cpu_millicores(&node.spec.resources.requests.cpu))
        .unwrap_or(1000);

    let cores = millicores.div_ceil(1000).clamp(1, 32);
    cores.max(1)
}

fn network_spread_label_selector(spec: &StellarNodeSpec) -> LabelSelector {
    LabelSelector {
        match_labels: Some(BTreeMap::from([
            (
                "app.kubernetes.io/name".to_string(),
                "stellar-node".to_string(),
            ),
            (
                "stellar-network".to_string(),
                spec.network
                    .scheduling_label_value(&spec.custom_network_passphrase),
            ),
            (
                "app.kubernetes.io/component".to_string(),
                spec.node_type.to_string().to_lowercase(),
            ),
        ])),
        ..Default::default()
    }
}

pub(crate) fn merge_workload_affinity(node: &StellarNode) -> Option<Affinity> {
    let mut aff = Affinity::default();
    if let Some(na) = node.spec.storage.node_affinity.clone() {
        aff.node_affinity = Some(na);
    }

    if let Some(na) = node.spec.node_affinity.clone() {
        aff.node_affinity = Some(na);
    }

    // Inject jurisdiction nodeAffinity (overrides storage node_affinity if both set)
    if let Some(jurisdiction) = node.spec.placement.jurisdiction.as_ref() {
        if let Some(jur_affinity) =
            crate::controller::jurisdiction::build_jurisdiction_node_affinity(jurisdiction)
        {
            aff.node_affinity = Some(jur_affinity);
        }
    }

    let mut req_terms = Vec::new();
    let mut pref_terms = Vec::new();

    // 1. Default network-level separation
    if let Some(pa) = build_network_pod_anti_affinity(node) {
        if let Some(mut req) = pa.required_during_scheduling_ignored_during_execution {
            req_terms.append(&mut req);
        }
        if let Some(mut pref) = pa.preferred_during_scheduling_ignored_during_execution {
            pref_terms.append(&mut pref);
        }
    }

    // 2. SCP-aware separation (Validators only)
    if let Some(pa) = build_scp_aware_pod_anti_affinity(node) {
        if let Some(mut req) = pa.required_during_scheduling_ignored_during_execution {
            req_terms.append(&mut req);
        }
        if let Some(mut pref) = pa.preferred_during_scheduling_ignored_during_execution {
            pref_terms.append(&mut pref);
        }
    }

    if !req_terms.is_empty() || !pref_terms.is_empty() {
        aff.pod_anti_affinity = Some(PodAntiAffinity {
            required_during_scheduling_ignored_during_execution: if req_terms.is_empty() {
                None
            } else {
                Some(req_terms)
            },
            preferred_during_scheduling_ignored_during_execution: if pref_terms.is_empty() {
                None
            } else {
                Some(pref_terms)
            },
        });
    }

    if aff.node_affinity.is_none() && aff.pod_anti_affinity.is_none() {
        None
    } else {
        Some(aff)
    }
}

pub(crate) fn build_scp_aware_pod_anti_affinity(node: &StellarNode) -> Option<PodAntiAffinity> {
    // Only applies to Validators when SCP-aware placement is enabled
    if node.spec.node_type != NodeType::Validator || !node.spec.placement.scp_aware_anti_affinity {
        return None;
    }

    let qset = node
        .spec
        .validator_config
        .as_ref()
        .and_then(|c| c.quorum_set.as_ref())?;

    let peer_names = extract_peer_names_from_toml(qset);
    if peer_names.is_empty() {
        return None;
    }

    let mut terms = Vec::new();

    for peer_name in peer_names {
        // We discourage placing this validator on the same node as its quorum set members.
        // Each peer is identified by its instance name label.
        let mut match_labels = BTreeMap::new();
        match_labels.insert("app.kubernetes.io/instance".to_string(), peer_name);

        terms.push(WeightedPodAffinityTerm {
            weight: 100,
            pod_affinity_term: PodAffinityTerm {
                label_selector: Some(LabelSelector {
                    match_labels: Some(match_labels),
                    ..Default::default()
                }),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            },
        });
    }

    Some(PodAntiAffinity {
        preferred_during_scheduling_ignored_during_execution: Some(terms),
        ..Default::default()
    })
}

pub(crate) fn build_network_pod_anti_affinity(node: &StellarNode) -> Option<PodAntiAffinity> {
    match node.spec.pod_anti_affinity {
        PodAntiAffinityStrength::Disabled => None,
        PodAntiAffinityStrength::Hard => {
            let term = PodAffinityTerm {
                label_selector: Some(network_spread_label_selector(&node.spec)),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            };
            Some(PodAntiAffinity {
                required_during_scheduling_ignored_during_execution: Some(vec![term]),
                ..Default::default()
            })
        }
        PodAntiAffinityStrength::Soft => {
            let term = PodAffinityTerm {
                label_selector: Some(network_spread_label_selector(&node.spec)),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            };
            Some(PodAntiAffinity {
                preferred_during_scheduling_ignored_during_execution: Some(vec![
                    WeightedPodAffinityTerm {
                        weight: 100,
                        pod_affinity_term: term,
                    },
                ]),
                ..Default::default()
            })
        }
    }
}

/// Build `TopologySpreadConstraints` for a pod spec.
pub fn build_topology_spread_constraints(
    spec: &crate::crd::StellarNodeSpec,
    _node_name: &str,
) -> Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint> {
    use k8s_openapi::api::core::v1::TopologySpreadConstraint;

    if let Some(constraints) = &spec.topology_spread_constraints {
        if !constraints.is_empty() {
            return constraints.clone();
        }
    }

    let when_unsatisfiable = match spec.pod_anti_affinity {
        PodAntiAffinityStrength::Soft => "ScheduleAnyway".to_string(),
        PodAntiAffinityStrength::Hard | PodAntiAffinityStrength::Disabled => {
            "DoNotSchedule".to_string()
        }
    };

    let selector = network_spread_label_selector(spec);

    vec![
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "kubernetes.io/hostname".to_string(),
            when_unsatisfiable: when_unsatisfiable.clone(),
            label_selector: Some(selector.clone()),
            ..Default::default()
        },
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "topology.kubernetes.io/zone".to_string(),
            when_unsatisfiable,
            label_selector: Some(selector),
            ..Default::default()
        },
    ]
}

pub(crate) fn build_container(node: &StellarNode, enable_mtls: bool) -> Container {
    let mut requests = BTreeMap::new();
    requests.insert(
        "cpu".to_string(),
        Quantity(node.spec.resources.requests.cpu.clone()),
    );
    requests.insert(
        "memory".to_string(),
        Quantity(node.spec.resources.requests.memory.clone()),
    );

    let mut limits = BTreeMap::new();
    limits.insert(
        "cpu".to_string(),
        Quantity(node.spec.resources.limits.cpu.clone()),
    );
    limits.insert(
        "memory".to_string(),
        Quantity(node.spec.resources.limits.memory.clone()),
    );

    let (container_port, data_mount_path, db_env_var_name) = match node.spec.node_type {
        NodeType::Validator => (11625, "/opt/stellar/data", "DATABASE"),
        NodeType::Horizon => (8000, "/data", "DATABASE_URL"),
        NodeType::SorobanRpc => (8000, "/data", "DATABASE_URL"),
    };

    let mut env_vars = vec![EnvVar {
        name: "NETWORK_PASSPHRASE".to_string(),
        value: Some(node.spec.network_passphrase().to_string()),
        ..Default::default()
    }];

    let worker_threads = derive_worker_threads(node);
    match node.spec.node_type {
        NodeType::Validator => {
            env_vars.push(EnvVar {
                name: "STELLAR_CORE_WORKER_THREADS".to_string(),
                value: Some(worker_threads.to_string()),
                ..Default::default()
            });
            env_vars.push(EnvVar {
                name: "STELLAR_CORE_HTTP_QUERY_THREADS".to_string(),
                value: Some((worker_threads.max(2) / 2).max(1).to_string()),
                ..Default::default()
            });
        }
        NodeType::Horizon => {
            let ingest_workers = node
                .spec
                .horizon_config
                .as_ref()
                .map(|cfg| cfg.ingest_workers.max(1))
                .unwrap_or(worker_threads);
            env_vars.push(EnvVar {
                name: "HORIZON_INGEST_WORKERS".to_string(),
                value: Some(ingest_workers.to_string()),
                ..Default::default()
            });
        }
        NodeType::SorobanRpc => {
            env_vars.push(EnvVar {
                name: "SOROBAN_RPC_WORKER_THREADS".to_string(),
                value: Some(worker_threads.to_string()),
                ..Default::default()
            });
            env_vars.push(EnvVar {
                name: "CAPTIVE_CORE_WORKER_THREADS".to_string(),
                value: Some((worker_threads / 2).max(1).to_string()),
                ..Default::default()
            });
        }
    }

    // Source validator seed from Secret or shared RAM volume (KMS)
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            match validator_config.key_source {
                KeySource::Secret => {
                    // Only inject the legacy env var when seed_secret_source is NOT set.
                    // When seed_secret_source IS set, the injection is handled via
                    // seed_injection in build_pod_template so we skip it here.
                    if validator_config.seed_secret_source.is_none()
                        && !validator_config.seed_secret_ref.is_empty()
                    {
                        env_vars.push(EnvVar {
                            name: "STELLAR_CORE_SEED".to_string(),
                            value: None,
                            value_from: Some(EnvVarSource {
                                secret_key_ref: Some(SecretKeySelector {
                                    name: Some(validator_config.seed_secret_ref.clone()),
                                    key: "STELLAR_CORE_SEED".to_string(),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }),
                        });
                    }
                }
                KeySource::KMS => {
                    env_vars.push(EnvVar {
                        name: "STELLAR_CORE_SEED_PATH".to_string(),
                        value: Some("/keys/validator-seed".to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add database environment variable from secret if external database is configured
    if let Some(db_config) = &node.spec.database {
        env_vars.push(EnvVar {
            name: db_env_var_name.to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: db_config
                    .secret_key_ref
                    .as_ref()
                    .map(|r| SecretKeySelector {
                        name: Some(r.name.clone()),
                        key: r.key.clone(),
                        ..Default::default()
                    }),
                ..Default::default()
            }),
        });
    }

    // Add database environment variable from CNPG secret if managed database is configured
    if let Some(_managed_db) = &node.spec.managed_database {
        let secret_name = node.name_any();
        env_vars.push(EnvVar {
            name: db_env_var_name.to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: Some(format!("{secret_name}-app")),
                    key: "uri".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        });
    }

    // Add TLS environment variables if mTLS is enabled
    if enable_mtls {
        match node.spec.node_type {
            NodeType::Horizon | NodeType::SorobanRpc => {
                env_vars.push(EnvVar {
                    name: "TLS_CERT_FILE".to_string(),
                    value: Some("/etc/stellar/tls/tls.crt".to_string()),
                    ..Default::default()
                });
                env_vars.push(EnvVar {
                    name: "TLS_KEY_FILE".to_string(),
                    value: Some("/etc/stellar/tls/tls.key".to_string()),
                    ..Default::default()
                });
                env_vars.push(EnvVar {
                    name: "CA_CERT_FILE".to_string(),
                    value: Some("/etc/stellar/tls/ca.crt".to_string()),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    // Add HSM environment variables and mounts
    let mut extra_volume_mounts = Vec::new();
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if let Some(hsm_config) = &validator_config.hsm_config {
                env_vars.push(EnvVar {
                    name: "PKCS11_MODULE_PATH".to_string(),
                    value: Some(hsm_config.pkcs11_lib_path.clone()),
                    ..Default::default()
                });

                if let Some(ip) = &hsm_config.hsm_ip {
                    env_vars.push(EnvVar {
                        name: "HSM_IP_ADDRESS".to_string(),
                        value: Some(ip.clone()),
                        ..Default::default()
                    });
                }

                if let Some(secret_ref) = &hsm_config.hsm_credentials_secret_ref {
                    env_vars.push(EnvVar {
                        name: "HSM_PIN".to_string(),
                        value: None,
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: "HSM_PIN".to_string(),
                                optional: Some(true),
                            }),
                            ..Default::default()
                        }),
                    });
                    env_vars.push(EnvVar {
                        name: "HSM_USER".to_string(),
                        value: None,
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: "HSM_USER".to_string(),
                                optional: Some(true),
                            }),
                            ..Default::default()
                        }),
                    });
                }

                if hsm_config.provider == HsmProvider::AWS {
                    extra_volume_mounts.push(VolumeMount {
                        name: "cloudhsm-socket".to_string(),
                        mount_path: "/var/run/cloudhsm".to_string(),
                        ..Default::default()
                    });
                } else if hsm_config.provider == HsmProvider::Azure {
                    // Sidecar bridge for PKCS#11 access to Azure Dedicated HSM.
                    extra_volume_mounts.push(VolumeMount {
                        name: "dedicatedhsm-socket".to_string(),
                        mount_path: "/var/run/dedicatedhsm".to_string(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    let mut volume_mounts = vec![
        VolumeMount {
            name: "data".to_string(),
            mount_path: data_mount_path.to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            read_only: Some(true),
            ..Default::default()
        },
    ];

    // Mount keys volume if using KMS
    if node.spec.node_type == NodeType::Validator {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.key_source == KeySource::KMS {
                volume_mounts.push(VolumeMount {
                    name: "keys".to_string(),
                    mount_path: "/keys".to_string(),
                    read_only: Some(true),
                    ..Default::default()
                });
            }
        }
    }

    // Mount mTLS certificates
    volume_mounts.push(VolumeMount {
        name: "tls".to_string(),
        mount_path: "/etc/stellar/tls".to_string(),
        read_only: Some(true),
        ..Default::default()
    });

    // Add extra mounts (HSM)
    volume_mounts.extend(extra_volume_mounts);

    if let Some(custom_volume_mounts) = &node.spec.volume_mounts {
        let existing_mount_names: BTreeSet<String> =
            volume_mounts.iter().map(|m| m.name.clone()).collect();
        for mount in custom_volume_mounts {
            if existing_mount_names.contains(&mount.name) {
                continue;
            }
            volume_mounts.push(mount.clone());
        }
    }

    // Apply node-type specific custom environment variables from the CRD.
    match node.spec.node_type {
        NodeType::Validator => merge_env_overrides(&mut env_vars, &node.spec.stellar_core_env),
        NodeType::Horizon => merge_env_overrides(&mut env_vars, &node.spec.horizon_env),
        NodeType::SorobanRpc => {}
    }

    Container {
        name: "stellar-node".to_string(),
        image: Some(node.spec.container_image()),
        ports: Some(vec![ContainerPort {
            container_port,
            ..Default::default()
        }]),
        env: Some(env_vars),
        resources: Some(K8sResources {
            requests: Some(requests),
            limits: Some(limits),
            claims: None,
        }),
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                add: None,
                drop: Some(vec!["ALL".to_string()]),
            }),
            run_as_non_root: Some(true),
            privileged: Some(false),
            read_only_root_filesystem: Some(true),
            seccomp_profile: Some(SeccompProfile {
                localhost_profile: None,
                type_: "RuntimeDefault".to_string(),
            }),
            ..Default::default()
        }),
        volume_mounts: Some(volume_mounts),
        liveness_probe: apply_probe_override(
            Some(k8s_openapi::api::core::v1::Probe {
                http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
                    path: Some("/healthz".to_string()),
                    port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081),
                    ..Default::default()
                }),
                initial_delay_seconds: Some(30),
                period_seconds: Some(10),
                timeout_seconds: Some(5),
                failure_threshold: Some(3),
                ..Default::default()
            }),
            node.spec.probes.as_ref().and_then(|p| p.liveness.as_ref()),
        ),
        readiness_probe: apply_probe_override(
            Some(k8s_openapi::api::core::v1::Probe {
                http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
                    path: Some("/readyz".to_string()),
                    port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081),
                    ..Default::default()
                }),
                initial_delay_seconds: Some(60),
                period_seconds: Some(5),
                timeout_seconds: Some(5),
                failure_threshold: Some(2),
                ..Default::default()
            }),
            node.spec.probes.as_ref().and_then(|p| p.readiness.as_ref()),
        ),
        startup_probe: apply_probe_override(
            Some(k8s_openapi::api::core::v1::Probe {
                http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
                    path: Some("/healthz".to_string()),
                    port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081),
                    ..Default::default()
                }),
                initial_delay_seconds: Some(0),
                period_seconds: Some(10),
                timeout_seconds: Some(5),
                failure_threshold: Some(30),
                ..Default::default()
            }),
            node.spec.probes.as_ref().and_then(|p| p.startup.as_ref()),
        ),
        ..Default::default()
    }
}

pub(crate) fn build_diagnostic_sidecar_resources(
    override_resources: Option<&ResourceRequirements>,
) -> K8sResources {
    let requests_cpu = override_resources
        .map(|resources| resources.requests.cpu.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DIAGNOSTIC_SIDECAR_DEFAULT_CPU);
    let requests_memory = override_resources
        .map(|resources| resources.requests.memory.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DIAGNOSTIC_SIDECAR_DEFAULT_MEMORY);
    let limits_cpu = override_resources
        .map(|resources| resources.limits.cpu.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DIAGNOSTIC_SIDECAR_DEFAULT_CPU);
    let limits_memory = override_resources
        .map(|resources| resources.limits.memory.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DIAGNOSTIC_SIDECAR_DEFAULT_MEMORY);

    K8sResources {
        requests: Some(
            [
                ("cpu".to_string(), Quantity(requests_cpu.to_string())),
                ("memory".to_string(), Quantity(requests_memory.to_string())),
            ]
            .into_iter()
            .collect(),
        ),
        limits: Some(
            [
                ("cpu".to_string(), Quantity(limits_cpu.to_string())),
                ("memory".to_string(), Quantity(limits_memory.to_string())),
            ]
            .into_iter()
            .collect(),
        ),
        claims: None,
    }
}

fn merge_env_overrides(base: &mut Vec<EnvVar>, overrides: &[EnvVar]) {
    for override_var in overrides {
        if let Some(existing) = base.iter_mut().find(|env| env.name == override_var.name) {
            *existing = override_var.clone();
        } else {
            base.push(override_var.clone());
        }
    }
}

pub(crate) fn build_workload_tolerations(node: &StellarNode) -> Option<Vec<Toleration>> {
    let mut tolerations = node.spec.tolerations.clone();

    if let Some(jurisdiction) = node.spec.placement.jurisdiction.as_ref() {
        crate::controller::jurisdiction::merge_jurisdiction_tolerations(
            &mut tolerations,
            jurisdiction,
        );
    }

    if tolerations.is_empty() {
        None
    } else {
        Some(tolerations)
    }
}

/// Build the migration container for Horizon
pub(crate) fn build_horizon_migration_container(node: &StellarNode) -> Container {
    let mut container = build_container(node, false);
    container.name = "horizon-db-migration".to_string();
    container.command = Some(vec!["/bin/sh".to_string()]);
    container.args = Some(vec![
        "-c".to_string(),
        "horizon db upgrade || horizon db init".to_string(),
    ]);
    container.ports = None;
    container.liveness_probe = None;
    container.readiness_probe = None;
    container.startup_probe = None;
    container.lifecycle = None;
    container
}

/// Build the snapshot-restore init container for compressed DB backup bootstrapping.
///
/// This container runs before Stellar Core and:
/// 1. Checks whether `/data` is already populated (idempotent — skips if data exists).
/// 2. Downloads the archive from `backup_url` (S3 or HTTPS).
/// 3. Extracts it into `/data`.
///
/// Supports `.tar.gz` and `.tar.zst` archives.
/// For S3 URLs, AWS CLI credentials are injected from `credentials_secret_ref`.
pub(crate) fn build_snapshot_restore_container(
    _node: &StellarNode,
    backup_url: &str,
    credentials_secret_ref: Option<&str>,
    restore_image: Option<&str>,
) -> Container {
    // Choose a sensible default image based on the URL scheme.
    let image = restore_image.map(|s| s.to_string()).unwrap_or_else(|| {
        if backup_url.starts_with("s3://") {
            "amazon/aws-cli:latest".to_string()
        } else {
            "alpine:3".to_string()
        }
    });

    // Determine the decompression command based on the file extension.
    let decompress_flag = if backup_url.ends_with(".tar.zst") {
        "--use-compress-program=zstd"
    } else {
        "-z" // default: gzip
    };

    // Build the shell script that runs inside the init container.
    // The script is idempotent: if /data already has content it exits immediately.
    let script = if backup_url.starts_with("s3://") {
        format!(
            r#"set -e
# Skip restore if data volume already has content (idempotent)
if [ "$(ls -A /data 2>/dev/null)" ]; then
  echo "Data volume already populated, skipping snapshot restore."
  exit 0
fi
echo "Restoring from S3 snapshot: {url}"
aws s3 cp "{url}" /tmp/snapshot.archive
echo "Extracting archive..."
tar {decompress} -xf /tmp/snapshot.archive -C /data
rm -f /tmp/snapshot.archive
echo "Snapshot restore complete."
"#,
            url = backup_url,
            decompress = decompress_flag,
        )
    } else {
        format!(
            r#"set -e
# Skip restore if data volume already has content (idempotent)
if [ "$(ls -A /data 2>/dev/null)" ]; then
  echo "Data volume already populated, skipping snapshot restore."
  exit 0
fi
echo "Restoring from backup: {url}"
wget -q -O /tmp/snapshot.archive "{url}" || curl -fsSL -o /tmp/snapshot.archive "{url}"
echo "Extracting archive..."
tar {decompress} -xf /tmp/snapshot.archive -C /data
rm -f /tmp/snapshot.archive
echo "Snapshot restore complete."
"#,
            url = backup_url,
            decompress = decompress_flag,
        )
    };

    // Build environment variables — inject AWS credentials if provided.
    let mut env: Vec<EnvVar> = vec![EnvVar {
        name: "BACKUP_URL".to_string(),
        value: Some(backup_url.to_string()),
        ..Default::default()
    }];

    if let Some(secret_name) = credentials_secret_ref {
        // AWS credentials
        for key in &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_DEFAULT_REGION",
        ] {
            env.push(EnvVar {
                name: key.to_string(),
                value: None,
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some(secret_name.to_string()),
                        key: key.to_string(),
                        optional: Some(true),
                    }),
                    ..Default::default()
                }),
            });
        }
        // Generic bearer token for HTTPS
        env.push(EnvVar {
            name: "BEARER_TOKEN".to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: Some(secret_name.to_string()),
                    key: "BEARER_TOKEN".to_string(),
                    optional: Some(true),
                }),
                ..Default::default()
            }),
        });
    }

    Container {
        name: "snapshot-restore".to_string(),
        image: Some(image),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string(), script]),
        env: Some(env),
        volume_mounts: Some(vec![VolumeMount {
            name: "data".to_string(),
            mount_path: "/data".to_string(),
            ..Default::default()
        }]),
        // Security: run as non-root, read-only root filesystem except /tmp
        security_context: Some(SecurityContext {
            run_as_non_root: Some(false), // aws-cli/alpine may need root for tar
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        resources: Some(K8sResources {
            requests: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("100m".to_string()));
                m.insert("memory".to_string(), Quantity("256Mi".to_string()));
                m
            }),
            limits: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("500m".to_string()));
                m.insert("memory".to_string(), Quantity("512Mi".to_string()));
                m
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

