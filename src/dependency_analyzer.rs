//! Dependency analysis and circular dependency detection

use std::collections::{HashMap, HashSet};

/// Dependency graph for types
#[allow(dead_code)]
pub struct DependencyGraph {
    /// Adjacency list: type -> types it depends on
    dependencies: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
        }
    }

    /// Add a dependency: from_type depends on to_type
    #[allow(dead_code)]
    pub fn add_dependency(&mut self, from_type: String, to_type: String) {
        self.dependencies
            .entry(from_type)
            .or_default()
            .insert(to_type);
    }

    /// Detect circular dependencies using DFS
    ///
    /// # Returns
    ///
    /// A vector of cycles, where each cycle is a vec of type names
    #[allow(dead_code)]
    pub fn detect_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for type_name in self.dependencies.keys() {
            if !visited.contains(type_name) {
                self.dfs_cycle_detect(
                    type_name,
                    &mut visited,
                    &mut rec_stack,
                    &mut path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn dfs_cycle_detect(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(neighbors) = self.dependencies.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    self.dfs_cycle_detect(neighbor, visited, rec_stack, path, cycles);
                } else if rec_stack.contains(neighbor) {
                    // Found a cycle
                    if let Some(pos) = path.iter().position(|x| x == neighbor) {
                        let cycle: Vec<String> = path[pos..].to_vec();
                        if !cycles.contains(&cycle) {
                            cycles.push(cycle);
                        }
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }

    /// Generate a DOT format representation of the dependency graph
    ///
    /// Can be visualized with Graphviz: `dot -Tpng deps.dot -o deps.png`
    #[allow(dead_code)]
    pub fn to_dot(&self) -> String {
        let mut dot = String::from("digraph Dependencies {\n");
        dot.push_str("  rankdir=LR;\n");
        dot.push_str("  node [shape=box, style=rounded];\n\n");

        for (from, tos) in &self.dependencies {
            for to in tos {
                dot.push_str(&format!("  \"{}\" -> \"{}\";\n", from, to));
            }
        }

        dot.push_str("}\n");
        dot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_cycles() {
        let mut graph = DependencyGraph::new();
        graph.add_dependency("A".to_string(), "B".to_string());
        graph.add_dependency("B".to_string(), "C".to_string());

        let cycles = graph.detect_cycles();
        assert!(cycles.is_empty());
    }

    #[test]
    fn test_simple_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_dependency("A".to_string(), "B".to_string());
        graph.add_dependency("B".to_string(), "A".to_string());

        let cycles = graph.detect_cycles();
        assert!(!cycles.is_empty());
    }

    #[test]
    fn test_complex_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_dependency("A".to_string(), "B".to_string());
        graph.add_dependency("B".to_string(), "C".to_string());
        graph.add_dependency("C".to_string(), "A".to_string());

        let cycles = graph.detect_cycles();
        assert!(!cycles.is_empty());
    }

    #[test]
    fn test_dot_generation() {
        let mut graph = DependencyGraph::new();
        graph.add_dependency("A".to_string(), "B".to_string());
        graph.add_dependency("B".to_string(), "C".to_string());

        let dot = graph.to_dot();
        assert!(dot.contains("digraph Dependencies"));
        assert!(dot.contains("\"A\" -> \"B\""));
    }
}
