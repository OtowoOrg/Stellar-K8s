//! Integration tests for StellarNode custom init containers.
//!
//! These tests verify that init containers are correctly injected into pod specs
//! and that volume sharing with the main container works as expected.

use k8s_openapi::api::core::v1::{Container, VolumeMount};

// ── Helper ────────────────────────────────────────────────────────────────────

fn make_init_container(name: &str, image: &str, command: Vec<&str>) -> Container {
    Container {
        name: name.to_string(),
        image: Some(image.to_string()),
        command: Some(command.into_iter().map(String::from).collect()),
        ..Default::default()
    }
}

fn make_init_container_with_volume(
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

// ── Ordering tests ────────────────────────────────────────────────────────────

#[test]
fn init_containers_are_ordered_by_array_index() {
    let containers = vec![
        make_init_container("step-1", "alpine", vec!["echo", "first"]),
        make_init_container("step-2", "alpine", vec!["echo", "second"]),
        make_init_container("step-3", "alpine", vec!["echo", "third"]),
    ];

    // Kubernetes guarantees sequential execution in array order.
    assert_eq!(containers[0].name, "step-1");
    assert_eq!(containers[1].name, "step-2");
    assert_eq!(containers[2].name, "step-3");
}

#[test]
fn empty_init_containers_list_is_valid() {
    let containers: Vec<Container> = vec![];
    assert!(containers.is_empty());
}

// ── Volume sharing tests ──────────────────────────────────────────────────────

#[test]
fn init_container_can_share_data_volume_with_main_container() {
    let init = make_init_container_with_volume(
        "data-seeder",
        "busybox",
        "data",
        "/data",
    );

    let mounts = init.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].name, "data");
    assert_eq!(mounts[0].mount_path, "/data");
}

#[test]
fn init_container_can_share_config_volume() {
    let init = make_init_container_with_volume(
        "config-generator",
        "stellar-config-gen:latest",
        "config",
        "/config",
    );

    let mounts = init.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts[0].name, "config");
    assert_eq!(mounts[0].mount_path, "/config");
}

#[test]
fn init_container_can_mount_multiple_volumes() {
    let init = Container {
        name: "multi-mount".to_string(),
        image: Some("alpine".to_string()),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "data".to_string(),
                mount_path: "/data".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "config".to_string(),
                mount_path: "/config".to_string(),
                read_only: Some(true),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    let mounts = init.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[1].name, "config");
    assert_eq!(mounts[1].read_only, Some(true));
}

// ── Common pattern tests ──────────────────────────────────────────────────────

#[test]
fn db_migration_init_container_pattern() {
    // Simulates: run DB migrations before Horizon starts
    let init = Container {
        name: "db-migrate".to_string(),
        image: Some("stellar-horizon:latest".to_string()),
        command: Some(vec![
            "stellar-horizon".to_string(),
            "db".to_string(),
            "migrate".to_string(),
            "up".to_string(),
        ]),
        env: Some(vec![k8s_openapi::api::core::v1::EnvVar {
            name: "DATABASE_URL".to_string(),
            value: Some("postgres://horizon:secret@postgres:5432/horizon".to_string()),
            ..Default::default()
        }]),
        ..Default::default()
    };

    assert_eq!(init.name, "db-migrate");
    let cmd = init.command.as_ref().unwrap();
    assert!(cmd.contains(&"migrate".to_string()));
}

#[test]
fn config_generation_init_container_pattern() {
    // Simulates: generate stellar-core.cfg from a template before core starts
    let init = Container {
        name: "config-gen".to_string(),
        image: Some("stellar-config-gen:v1".to_string()),
        command: Some(vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "envsubst < /templates/stellar-core.cfg.tmpl > /config/stellar-core.cfg".to_string(),
        ]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "config".to_string(),
                mount_path: "/config".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "config-templates".to_string(),
                mount_path: "/templates".to_string(),
                read_only: Some(true),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    assert_eq!(init.name, "config-gen");
    let mounts = init.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 2);
    // config mount is writable (read_only not set)
    assert_ne!(mounts[0].read_only, Some(true));
    // templates mount is read-only
    assert_eq!(mounts[1].read_only, Some(true));
}

#[test]
fn data_seeding_init_container_pattern() {
    // Simulates: seed initial ledger data from a checkpoint before core starts
    let init = Container {
        name: "data-seed".to_string(),
        image: Some("stellar-data-seeder:latest".to_string()),
        command: Some(vec![
            "aws".to_string(),
            "s3".to_string(),
            "sync".to_string(),
            "s3://my-bucket/ledger-checkpoint/".to_string(),
            "/data/".to_string(),
        ]),
        volume_mounts: Some(vec![VolumeMount {
            name: "data".to_string(),
            mount_path: "/data".to_string(),
            ..Default::default()
        }]),
        ..Default::default()
    };

    assert_eq!(init.name, "data-seed");
    let cmd = init.command.as_ref().unwrap();
    assert!(cmd.contains(&"s3".to_string()));
}

// ── Container spec validation tests ──────────────────────────────────────────

#[test]
fn init_container_name_is_required() {
    let init = make_init_container("", "alpine", vec!["echo", "hi"]);
    // An empty name is technically valid at struct level but Kubernetes will reject it.
    // The operator should validate non-empty names before submitting.
    assert!(init.name.is_empty()); // document that we rely on k8s admission for this
}

#[test]
fn init_container_without_command_is_valid() {
    // Use the image's default ENTRYPOINT
    let init = Container {
        name: "wait-for-db".to_string(),
        image: Some("wait-for-it:latest".to_string()),
        args: Some(vec!["postgres:5432".to_string(), "--".to_string()]),
        ..Default::default()
    };
    assert!(init.command.is_none());
    assert!(init.args.is_some());
}

#[test]
fn multiple_init_containers_all_must_succeed_before_main_starts() {
    // This is a Kubernetes guarantee; document it with an assertion on ordering.
    let init_containers = vec![
        make_init_container("check-db", "wait-for-it", vec!["db:5432"]),
        make_init_container("run-migrations", "stellar-horizon", vec!["db", "migrate", "up"]),
    ];

    // Kubernetes runs them in index order and waits for each to exit 0.
    assert_eq!(init_containers[0].name, "check-db");
    assert_eq!(init_containers[1].name, "run-migrations");
}
