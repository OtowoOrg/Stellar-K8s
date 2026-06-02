//! Workflow templates for common operations: upgrades, migrations, disaster recovery

use std::collections::HashMap;

use super::crd::{RetrySpec, TaskAction, TaskSpec, WorkflowSpec};

/// Build the standard StellarNode upgrade workflow
pub fn upgrade_workflow(node_name: &str, from_version: &str, to_version: &str) -> WorkflowSpec {
    WorkflowSpec {
        description: format!("Upgrade {node_name} from {from_version} to {to_version}"),
        schedule: None,
        max_parallelism: 1,
        timeout_secs: 900,
        labels: [("template".into(), "upgrade".into())].into_iter().collect(),
        tasks: vec![
            TaskSpec {
                id: "preflight".into(),
                name: "Pre-flight checks".into(),
                depends_on: vec![],
                action: TaskAction::WaitForCondition {
                    condition: format!("stellarnode/{node_name}.status.health == 'ready'"),
                    poll_interval_secs: 10,
                },
                retry: RetrySpec { max_attempts: 5, ..Default::default() },
                condition: None,
                timeout_secs: 120,
            },
            TaskSpec {
                id: "backup".into(),
                name: "Snapshot state".into(),
                depends_on: vec!["preflight".into()],
                action: TaskAction::Shell {
                    command: "kubectl".into(),
                    args: vec!["exec".into(), format!("{node_name}-0"), "--".into(), "/opt/stellar/backup.sh".into()],
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 300,
            },
            TaskSpec {
                id: "upgrade".into(),
                name: format!("Upgrade to {to_version}"),
                depends_on: vec!["backup".into()],
                action: TaskAction::StellarNodeUpgrade {
                    node_name: node_name.into(),
                    target_version: to_version.into(),
                },
                retry: RetrySpec { max_attempts: 2, ..Default::default() },
                condition: None,
                timeout_secs: 600,
            },
            TaskSpec {
                id: "verify".into(),
                name: "Verify upgrade".into(),
                depends_on: vec!["upgrade".into()],
                action: TaskAction::WaitForCondition {
                    condition: format!("stellarnode/{node_name}.status.version == '{to_version}'"),
                    poll_interval_secs: 15,
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 180,
            },
        ],
    }
}

/// Build a database migration workflow
pub fn migration_workflow(migration_name: &str, image: &str, namespace: &str) -> WorkflowSpec {
    WorkflowSpec {
        description: format!("Run migration: {migration_name}"),
        schedule: None,
        max_parallelism: 1,
        timeout_secs: 1800,
        labels: [("template".into(), "migration".into())].into_iter().collect(),
        tasks: vec![
            TaskSpec {
                id: "validate-schema".into(),
                name: "Validate schema version".into(),
                depends_on: vec![],
                action: TaskAction::KubernetesJob {
                    image: image.into(),
                    command: vec!["migrate".into(), "validate".into()],
                    namespace: namespace.into(),
                },
                retry: RetrySpec { max_attempts: 1, ..Default::default() },
                condition: None,
                timeout_secs: 120,
            },
            TaskSpec {
                id: "run-migration".into(),
                name: format!("Apply migration {migration_name}"),
                depends_on: vec!["validate-schema".into()],
                action: TaskAction::KubernetesJob {
                    image: image.into(),
                    command: vec!["migrate".into(), "up".into(), "--name".into(), migration_name.into()],
                    namespace: namespace.into(),
                },
                retry: RetrySpec { max_attempts: 1, ..Default::default() },
                condition: None,
                timeout_secs: 1200,
            },
            TaskSpec {
                id: "verify-migration".into(),
                name: "Verify migration applied".into(),
                depends_on: vec!["run-migration".into()],
                action: TaskAction::KubernetesJob {
                    image: image.into(),
                    command: vec!["migrate".into(), "status".into()],
                    namespace: namespace.into(),
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 60,
            },
        ],
    }
}

/// Build a disaster recovery failover workflow
pub fn disaster_recovery_workflow(primary_node: &str, dr_node: &str) -> WorkflowSpec {
    WorkflowSpec {
        description: format!("Disaster recovery: failover from {primary_node} to {dr_node}"),
        schedule: None,
        max_parallelism: 1,
        timeout_secs: 600,
        labels: [("template".into(), "disaster-recovery".into())].into_iter().collect(),
        tasks: vec![
            TaskSpec {
                id: "detect-failure".into(),
                name: "Detect primary failure".into(),
                depends_on: vec![],
                action: TaskAction::WaitForCondition {
                    condition: format!("stellarnode/{primary_node}.status.health != 'ready'"),
                    poll_interval_secs: 5,
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 60,
            },
            TaskSpec {
                id: "promote-dr".into(),
                name: "Promote DR node to primary".into(),
                depends_on: vec!["detect-failure".into()],
                action: TaskAction::Shell {
                    command: "kubectl".into(),
                    args: vec![
                        "patch".into(), "stellarnode".into(), dr_node.into(),
                        "--patch".into(), "{\"spec\":{\"role\":\"validator\"}}".into(),
                    ],
                },
                retry: RetrySpec { max_attempts: 3, ..Default::default() },
                condition: None,
                timeout_secs: 120,
            },
            TaskSpec {
                id: "update-dns".into(),
                name: "Update DNS/service endpoint".into(),
                depends_on: vec!["promote-dr".into()],
                action: TaskAction::Shell {
                    command: "kubectl".into(),
                    args: vec!["patch".into(), "service".into(), "stellar-primary".into()],
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 60,
            },
            TaskSpec {
                id: "notify".into(),
                name: "Send alert notification".into(),
                depends_on: vec!["promote-dr".into()],
                action: TaskAction::HttpCall {
                    url: "http://alertmanager:9093/api/v1/alerts".into(),
                    method: "POST".into(),
                    body: Some(format!(
                        r#"[{{"labels":{{"alertname":"DRFailover","primary":"{primary_node}","dr":"{dr_node}"}}}}]"#
                    )),
                },
                retry: RetrySpec::default(),
                condition: None,
                timeout_secs: 30,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upgrade_workflow_task_count() {
        let wf = upgrade_workflow("validator-0", "v19.0", "v20.0");
        assert_eq!(wf.tasks.len(), 4);
        assert!(wf.tasks.iter().any(|t| t.id == "preflight"));
        assert!(wf.tasks.iter().any(|t| t.id == "verify"));
    }

    #[test]
    fn test_upgrade_workflow_dependency_chain() {
        let wf = upgrade_workflow("v-0", "v1", "v2");
        let deps: Vec<&str> = wf.tasks.iter().flat_map(|t| t.depends_on.iter().map(|d| d.as_str())).collect();
        assert!(deps.contains(&"preflight"));
        assert!(deps.contains(&"backup"));
        assert!(deps.contains(&"upgrade"));
    }

    #[test]
    fn test_dr_workflow_parallel_tasks() {
        let wf = disaster_recovery_workflow("primary-0", "dr-0");
        // notify and update-dns both depend on promote-dr (can run in parallel)
        let parallel: Vec<&TaskSpec> = wf.tasks.iter().filter(|t| t.depends_on.contains(&"promote-dr".to_string())).collect();
        assert_eq!(parallel.len(), 2);
    }
}
