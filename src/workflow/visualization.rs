//! Workflow graph visualization — DOT and Mermaid export

use super::crd::{TaskSpec, WorkflowSpec};
use super::audit::AuditTrail;
use crate::workflow::task::TaskStatus;

/// Export a workflow spec as a Graphviz DOT graph
pub fn to_dot(spec: &WorkflowSpec) -> String {
    let mut dot = format!(
        "digraph Workflow {{\n  label=\"{}\";\n  rankdir=LR;\n  node [shape=box, style=filled];\n",
        spec.description.replace('"', "'")
    );

    for task in &spec.tasks {
        dot.push_str(&format!(
            "  \"{}\" [label=\"{}\\n{}\"];\n",
            task.id,
            task.name,
            format!("{:?}", task.action).split('{').next().unwrap_or("action").trim(),
        ));
        for dep in &task.depends_on {
            dot.push_str(&format!("  \"{}\" -> \"{}\";\n", dep, task.id));
        }
    }

    dot.push('}');
    dot
}

/// Export a workflow spec as a Mermaid flowchart
pub fn to_mermaid(spec: &WorkflowSpec) -> String {
    let mut md = String::from("flowchart LR\n");
    for task in &spec.tasks {
        md.push_str(&format!("  {}[\"{}\"]\n", sanitize_id(&task.id), task.name));
    }
    for task in &spec.tasks {
        for dep in &task.depends_on {
            md.push_str(&format!("  {} --> {}\n", sanitize_id(dep), sanitize_id(&task.id)));
        }
    }
    md
}

/// Export an executed workflow's audit trail as annotated Mermaid
pub fn audit_to_mermaid(spec: &WorkflowSpec, trail: &AuditTrail) -> String {
    let mut md = String::from("flowchart LR\n");
    for task in &spec.tasks {
        let status_label = trail
            .entries_for_task(&task.id)
            .last()
            .map(|e| format!("{}\\n{:?}", task.name, e.status))
            .unwrap_or_else(|| task.name.clone());
        md.push_str(&format!("  {}[\"{}\"]\n", sanitize_id(&task.id), status_label));
    }
    for task in &spec.tasks {
        for dep in &task.depends_on {
            md.push_str(&format!("  {} --> {}\n", sanitize_id(dep), sanitize_id(&task.id)));
        }
    }
    md
}

fn sanitize_id(id: &str) -> String {
    id.replace(['-', '.', ' '], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::crd::{TaskAction, RetrySpec};
    use std::collections::HashMap;

    fn simple_spec() -> WorkflowSpec {
        WorkflowSpec {
            description: "Test".into(),
            schedule: None,
            max_parallelism: 2,
            timeout_secs: 300,
            labels: HashMap::new(),
            tasks: vec![
                TaskSpec {
                    id: "task-a".into(),
                    name: "Task A".into(),
                    depends_on: vec![],
                    action: TaskAction::Noop,
                    retry: RetrySpec::default(),
                    condition: None,
                    timeout_secs: 60,
                },
                TaskSpec {
                    id: "task-b".into(),
                    name: "Task B".into(),
                    depends_on: vec!["task-a".into()],
                    action: TaskAction::Noop,
                    retry: RetrySpec::default(),
                    condition: None,
                    timeout_secs: 60,
                },
            ],
        }
    }

    #[test]
    fn test_dot_contains_nodes_and_edges() {
        let dot = to_dot(&simple_spec());
        assert!(dot.contains("task-a"));
        assert!(dot.contains("task-b"));
        assert!(dot.contains("task-a\" -> \"task-b\""));
    }

    #[test]
    fn test_mermaid_contains_arrow() {
        let md = to_mermaid(&simple_spec());
        assert!(md.contains("flowchart LR"));
        assert!(md.contains("-->"));
    }
}
