//! Directed Acyclic Graph (DAG) for workflow definition

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// DAG Node
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DAGNode {
    pub id: String,
    pub name: String,
    pub task_type: String,
    pub dependencies: Vec<String>,
    pub config: serde_json::Value,
    pub retry_policy: RetryPolicy,
}

impl DAGNode {
    pub fn new(id: String, name: String, task_type: String) -> Self {
        Self {
            id,
            name,
            task_type,
            dependencies: Vec::new(),
            config: serde_json::json!({}),
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_dependency(mut self, dep: String) -> Self {
        self.dependencies.push(dep);
        self
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }
}

/// Retry policy
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub backoff_multiplier: f64,
    pub initial_delay_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_multiplier: 2.0,
            initial_delay_secs: 1,
        }
    }
}

/// Directed Acyclic Graph
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DAG {
    pub id: String,
    pub name: String,
    pub nodes: HashMap<String, DAGNode>,
    pub created_at: DateTime<Utc>,
}

impl DAG {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            nodes: HashMap::new(),
            created_at: Utc::now(),
        }
    }

    pub fn add_node(&mut self, node: DAGNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn get_node(&self, id: &str) -> Option<&DAGNode> {
        self.nodes.get(id)
    }

    pub fn get_root_nodes(&self) -> Vec<&DAGNode> {
        self.nodes
            .values()
            .filter(|n| n.dependencies.is_empty())
            .collect()
    }

    pub fn get_leaf_nodes(&self) -> Vec<&DAGNode> {
        let dependent_nodes: std::collections::HashSet<_> = self
            .nodes
            .values()
            .flat_map(|n| n.dependencies.iter())
            .cloned()
            .collect();

        self.nodes
            .values()
            .filter(|n| !dependent_nodes.contains(&n.id))
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        // Check for cycles
        for node in self.nodes.values() {
            if self.has_cycle(&node.id) {
                return Err(format!("Cycle detected starting from node {}", node.id));
            }
        }

        // Check for missing dependencies
        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if !self.nodes.contains_key(dep) {
                    return Err(format!("Missing dependency {} for node {}", dep, node.id));
                }
            }
        }

        Ok(())
    }

    fn has_cycle(&self, node_id: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut rec_stack = std::collections::HashSet::new();
        self.has_cycle_util(node_id, &mut visited, &mut rec_stack)
    }

    fn has_cycle_util(
        &self,
        node_id: &str,
        visited: &mut std::collections::HashSet<String>,
        rec_stack: &mut std::collections::HashSet<String>,
    ) -> bool {
        visited.insert(node_id.to_string());
        rec_stack.insert(node_id.to_string());

        if let Some(node) = self.nodes.get(node_id) {
            for dep in &node.dependencies {
                if !visited.contains(dep) {
                    if self.has_cycle_util(dep, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(dep) {
                    return true;
                }
            }
        }

        rec_stack.remove(node_id);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_creation() {
        let dag = DAG::new("dag1".to_string(), "Test DAG".to_string());
        assert_eq!(dag.id, "dag1");
        assert_eq!(dag.nodes.len(), 0);
    }

    #[test]
    fn test_add_node() {
        let mut dag = DAG::new("dag1".to_string(), "Test DAG".to_string());
        let node = DAGNode::new("task1".to_string(), "Task 1".to_string(), "shell".to_string());
        dag.add_node(node);

        assert_eq!(dag.nodes.len(), 1);
    }

    #[test]
    fn test_dag_validation() {
        let mut dag = DAG::new("dag1".to_string(), "Test DAG".to_string());
        let node1 = DAGNode::new("task1".to_string(), "Task 1".to_string(), "shell".to_string());
        let node2 = DAGNode::new("task2".to_string(), "Task 2".to_string(), "shell".to_string())
            .with_dependency("task1".to_string());

        dag.add_node(node1);
        dag.add_node(node2);

        assert!(dag.validate().is_ok());
    }
}
