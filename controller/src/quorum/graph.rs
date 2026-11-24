use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub quorum_threshold: usize,
    pub validators: Vec<String>,
}

#[derive(Default, Clone, Debug)]
pub struct TopologyGraph {
    pub nodes: HashMap<String, Node>,
}

impl TopologyGraph {
    pub fn new() -> Self {
        Self { nodes: HashMap::new() }
    }

    pub fn add_node(&mut self, node: Node) {
        self.nodes.insert(node.id.clone(), node);
    }
}
