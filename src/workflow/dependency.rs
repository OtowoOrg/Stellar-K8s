//! Dependency resolution and topological sorting

use tracing::debug;

use crate::error::Result;
use super::dag::DAG;

/// Topological sort result
pub type TopologicalSort = Vec<String>;

/// Dependency Resolver
pub struct DependencyResolver;

impl DependencyResolver {
    pub fn new() -> Self {
        Self
    }

    pub async fn resolve(&self, dag: &DAG) -> Result<TopologicalSort> {
        debug!("Resolving dependencies for DAG: {}", dag.id);

        // Validate DAG first
        dag.validate()?;

        // Perform topological sort
        let mut sorted = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut rec_stack = std::collections::HashSet::new();

        for node_id in dag.nodes.keys() {
            if !visited.contains(node_id) {
                self.topological_sort_util(
                    node_id,
                    dag,
                    &mut visited,
                    &mut rec_stack,
                    &mut sorted,
                )?;
            }
        }

        sorted.reverse();
        Ok(sorted)
    }

    fn topological_sort_util(
        &self,
        node_id: &str,
        dag: &DAG,
        visited: &mut std::collections::HashSet<String>,
        rec_stack: &mut std::collections::HashSet<String>,
        sorted: &mut Vec<String>,
    ) -> Result<()> {
        visited.insert(node_id.to_string());
        rec_stack.insert(node_id.to_string());

        if let Some(node) = dag.get_node(node_id) {
            for dep in &node.dependencies {
                if !visited.contains(dep) {
                    self.topological_sort_util(dep, dag, visited, rec_stack, sorted)?;
                }
            }
        }

        rec_stack.remove(node_id);
        sorted.push(node_id.to_string());

        Ok(())
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dependency_resolution() {
        let mut dag = DAG::new("dag1".to_string(), "Test DAG".to_string());
        let node1 = super::super::dag::DAGNode::new(
            "task1".to_string(),
            "Task 1".to_string(),
            "shell".to_string(),
        );
        let node2 = super::super::dag::DAGNode::new(
            "task2".to_string(),
            "Task 2".to_string(),
            "shell".to_string(),
        )
        .with_dependency("task1".to_string());

        dag.add_node(node1);
        dag.add_node(node2);

        let resolver = DependencyResolver::new();
        let order = resolver.resolve(&dag).await.unwrap();

        assert_eq!(order.len(), 2);
        assert_eq!(order[0], "task1");
        assert_eq!(order[1], "task2");
    }
}
