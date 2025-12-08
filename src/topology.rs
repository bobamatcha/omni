//! Module topology graph builder.
//!
//! Builds the high-level view of crates, modules, and files with import relationships.

use crate::discovery::FileDiscovery;
use crate::parsing::parser_for_file;
use crate::state::OciState;
use crate::types::*;
use anyhow::{Context, Result};
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tree_sitter::Parser;

/// Builds and maintains the module topology graph.
pub struct TopologyBuilder;

impl TopologyBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Build topology from scratch for a repository.
    pub fn build(&self, state: &OciState, root: &Path) -> Result<()> {
        // Clear existing topology
        {
            let mut graph = state.topology.write();
            graph.clear();
        }
        state.path_to_node.clear();
        state.topology_metrics.clear();

        // Create root crate node
        let crate_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("root")
            .to_string();

        let root_node = {
            let mut graph = state.topology.write();
            graph.add_node(TopologyNode::Crate {
                name: crate_name,
                path: root.to_path_buf(),
                is_workspace: false,
            })
        };

        state
            .path_to_node
            .insert(root.to_path_buf(), root_node);

        // Initialize metrics for root
        state
            .topology_metrics
            .insert(root_node, TopologyMetrics::default());

        // Discover all source files
        let discovery = FileDiscovery::new();
        let files = discovery.discover(root)?;

        // Add all files to topology
        for file in files {
            if let Err(e) = self.add_file(state, &file) {
                tracing::warn!("Failed to add file {:?}: {}", file, e);
            }
        }

        // Parse imports and create import edges
        self.build_import_edges(state)?;

        // Compute PageRank scores
        self.compute_pagerank(state)?;

        Ok(())
    }

    /// Add a file to the topology.
    pub fn add_file(&self, state: &OciState, path: &Path) -> Result<NodeIndex> {
        // Check if file already exists
        if let Some(existing) = state.path_to_node.get(path) {
            return Ok(*existing);
        }

        // Get or create file ID
        let file_id = state.get_or_create_file_id(&path.to_path_buf());

        // Create file node
        let file_node = {
            let mut graph = state.topology.write();
            graph.add_node(TopologyNode::File {
                path: path.to_path_buf(),
                file_id,
            })
        };

        state.path_to_node.insert(path.to_path_buf(), file_node);

        // Initialize metrics
        state
            .topology_metrics
            .insert(file_node, TopologyMetrics::default());

        // Connect to parent module/crate
        self.connect_to_parent(state, path, file_node)?;

        // Parse imports from the file
        self.parse_file_imports(state, path, file_id)?;

        Ok(file_node)
    }

    /// Remove a file from the topology.
    pub fn remove_file(&self, state: &OciState, path: &Path) -> Result<()> {
        // Get the node index
        let node_idx = match state.path_to_node.remove(path) {
            Some((_, idx)) => idx,
            None => return Ok(()), // File not in topology
        };

        // Remove metrics
        state.topology_metrics.remove(&node_idx);

        // Remove the node from the graph
        {
            let mut graph = state.topology.write();
            graph.remove_node(node_idx);
        }

        // Clear file data from state
        state.clear_file(&path.to_path_buf());

        Ok(())
    }

    /// Compute PageRank scores for relevance ranking.
    pub fn compute_pagerank(&self, state: &OciState) -> Result<()> {
        let graph = state.topology.read();
        let node_count = graph.node_count();

        if node_count == 0 {
            return Ok(());
        }

        // Initialize scores to 1/N
        let initial_score = 1.0 / node_count as f64;
        let mut scores: HashMap<NodeIndex, f64> = HashMap::new();
        let mut new_scores: HashMap<NodeIndex, f64> = HashMap::new();

        for node_idx in graph.node_indices() {
            scores.insert(node_idx, initial_score);
            new_scores.insert(node_idx, 0.0);
        }

        // PageRank parameters
        const DAMPING: f64 = 0.85;
        const MAX_ITERATIONS: usize = 50;
        const CONVERGENCE_THRESHOLD: f64 = 1e-6;

        // Iterative PageRank computation
        for iteration in 0..MAX_ITERATIONS {
            let mut diff = 0.0;

            // For each node, compute new score
            for node_idx in graph.node_indices() {
                let mut rank_sum = 0.0;

                // Sum contributions from incoming edges
                for edge in graph.edges_directed(node_idx, Direction::Incoming) {
                    let source = edge.source();
                    let out_degree = graph
                        .edges_directed(source, Direction::Outgoing)
                        .count();

                    if out_degree > 0 {
                        let source_score = scores.get(&source).copied().unwrap_or(0.0);
                        rank_sum += source_score / out_degree as f64;
                    }
                }

                // Apply PageRank formula: (1-d)/N + d * sum
                let new_score = (1.0 - DAMPING) / node_count as f64 + DAMPING * rank_sum;
                new_scores.insert(node_idx, new_score);

                // Track convergence
                let old_score = scores.get(&node_idx).copied().unwrap_or(0.0);
                diff += (new_score - old_score).abs();
            }

            // Swap scores
            std::mem::swap(&mut scores, &mut new_scores);

            // Check convergence
            if diff < CONVERGENCE_THRESHOLD {
                tracing::debug!("PageRank converged after {} iterations", iteration + 1);
                break;
            }
        }

        // Update metrics with computed scores
        drop(graph); // Release read lock before updating metrics

        for (node_idx, score) in scores {
            state
                .topology_metrics
                .entry(node_idx)
                .and_modify(|m| m.relevance_score = score)
                .or_insert_with(|| TopologyMetrics {
                    relevance_score: score,
                    ..Default::default()
                });
        }

        Ok(())
    }

    /// Connect a file node to its parent module or crate.
    fn connect_to_parent(
        &self,
        state: &OciState,
        path: &Path,
        file_node: NodeIndex,
    ) -> Result<()> {
        // Find parent by traversing up the directory tree
        let mut current_path = path.parent();
        let mut parent_node = None;

        while let Some(dir) = current_path {
            if let Some(node_idx) = state.path_to_node.get(dir) {
                parent_node = Some(*node_idx);
                break;
            }

            // Check if this is a module directory (has mod.rs)
            let mod_rs = dir.join("mod.rs");
            if mod_rs.exists() {
                // Create module node
                let module_name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unnamed")
                    .to_string();

                let module_node = {
                    let mut graph = state.topology.write();
                    graph.add_node(TopologyNode::Module {
                        name: module_name,
                        path: dir.to_path_buf(),
                        is_inline: false,
                    })
                };

                state.path_to_node.insert(dir.to_path_buf(), module_node);
                state
                    .topology_metrics
                    .insert(module_node, TopologyMetrics::default());

                parent_node = Some(module_node);

                // Recursively connect this module to its parent
                self.connect_to_parent(state, dir, module_node)?;
                break;
            }

            current_path = dir.parent();
        }

        // If no parent found, connect to root
        let parent = match parent_node {
            Some(p) => p,
            None => {
                // Find the crate root
                let graph = state.topology.read();
                graph
                    .node_indices()
                    .find(|idx| matches!(graph[*idx], TopologyNode::Crate { .. }))
                    .context("No crate root found in topology")?
            }
        };

        // Add Contains edge from parent to file
        {
            let mut graph = state.topology.write();
            graph.add_edge(parent, file_node, TopologyEdge::Contains);
        }

        Ok(())
    }

    /// Parse imports from a file and store them.
    fn parse_file_imports(&self, state: &OciState, path: &Path, file_id: FileId) -> Result<()> {
        // Read file contents
        let source = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {:?}", path))?;

        // Get parser for the file
        let lang_parser = match parser_for_file(path) {
            Some(p) => p,
            None => return Ok(()), // Not a supported language
        };

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&lang_parser.language())
            .context("Failed to set parser language")?;

        let tree = parser
            .parse(&source, None)
            .context("Failed to parse file")?;

        // Extract imports
        let imports = lang_parser.extract_imports(&tree, &source, path)?;

        if !imports.is_empty() {
            state.imports.insert(file_id, imports);
        }

        Ok(())
    }

    /// Build import edges based on parsed imports.
    fn build_import_edges(&self, state: &OciState) -> Result<()> {
        // Collect all imports and their source files
        let mut import_map: Vec<(NodeIndex, FileId, ImportInfo)> = Vec::new();

        for entry in state.imports.iter() {
            let file_id = *entry.key();

            // Find the node index for this file
            if let Some(node_idx) = state.file_ids.iter().find_map(|e| {
                if *e.value() == file_id {
                    state.path_to_node.get(e.key()).map(|idx| *idx)
                } else {
                    None
                }
            }) {
                for import in entry.value().iter() {
                    import_map.push((node_idx, file_id, import.clone()));
                }
            }
        }

        // Create import edges
        let mut graph = state.topology.write();

        for (source_node, _file_id, import) in import_map {
            // Try to resolve the import target
            // For now, we create edges only if we can find a matching module/file
            // This is simplified - a full implementation would need path resolution

            // Look for a file or module that matches the import path
            let target_node = self.resolve_import_target(state, &import.path);

            if let Some(target) = target_node {
                // Add import edge
                graph.add_edge(
                    source_node,
                    target,
                    TopologyEdge::Imports {
                        use_path: import.path.clone(),
                        is_glob: import.is_glob,
                    },
                );
            }
        }

        Ok(())
    }

    /// Resolve an import path to a target node (simplified).
    fn resolve_import_target(&self, state: &OciState, import_path: &str) -> Option<NodeIndex> {
        // This is a simplified resolution - just tries to match module/file names
        // A full implementation would need proper Rust path resolution

        let graph = state.topology.read();

        // Look for a module or file with a matching name
        for node_idx in graph.node_indices() {
            match &graph[node_idx] {
                TopologyNode::Module { name, .. } => {
                    if import_path.contains(name) {
                        return Some(node_idx);
                    }
                }
                TopologyNode::File { path, .. } => {
                    if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                        if import_path.contains(file_name) {
                            return Some(node_idx);
                        }
                    }
                }
                TopologyNode::Crate { name, .. } => {
                    if import_path.starts_with(name) {
                        return Some(node_idx);
                    }
                }
            }
        }

        None
    }
}

impl Default for TopologyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::create_state;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_build_empty_topology() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();

        let result = builder.build(&state, temp.path());
        assert!(result.is_ok());

        // Should have a crate root
        let graph = state.topology.read();
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_add_remove_file() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();

        // Initialize with crate root
        builder.build(&state, temp.path()).unwrap();

        // Create a test file
        let test_file = temp.path().join("test.rs");
        fs::write(&test_file, "// test file").unwrap();

        // Add file
        let node_idx = builder.add_file(&state, &test_file).unwrap();
        assert!(state.path_to_node.contains_key(&test_file));
        assert!(state.topology_metrics.contains_key(&node_idx));

        // Remove file
        builder.remove_file(&state, &test_file).unwrap();
        assert!(!state.path_to_node.contains_key(&test_file));
        assert!(!state.topology_metrics.contains_key(&node_idx));
    }

    #[test]
    fn test_pagerank_computation() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();

        // Build topology
        builder.build(&state, temp.path()).unwrap();

        // Compute PageRank
        let result = builder.compute_pagerank(&state);
        assert!(result.is_ok());

        // Check that scores were computed
        let graph = state.topology.read();
        for node_idx in graph.node_indices() {
            let metrics = state.topology_metrics.get(&node_idx).unwrap();
            assert!(metrics.relevance_score > 0.0);
        }
    }

    #[test]
    fn test_connect_to_parent() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();

        // Initialize
        builder.build(&state, temp.path()).unwrap();

        // Create a subdirectory with a file
        let subdir = temp.path().join("src");
        fs::create_dir(&subdir).unwrap();
        let file = subdir.join("lib.rs");
        fs::write(&file, "// lib").unwrap();

        // Add file
        builder.add_file(&state, &file).unwrap();

        // Verify connection to parent
        let graph = state.topology.read();
        let file_node = state.path_to_node.get(&file).unwrap();

        // Should have at least one incoming edge (Contains from parent)
        let incoming = graph
            .edges_directed(*file_node, Direction::Incoming)
            .count();
        assert!(incoming > 0);
    }
}
