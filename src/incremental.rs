//! Incremental indexing engine.
//!
//! Handles efficient updates when files change, avoiding full re-indexing.

use crate::parsing;
use crate::state::OciState;
use crate::topology::TopologyBuilder;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use tree_sitter::Parser;

/// Incremental indexer that updates the state when files change.
pub struct IncrementalIndexer {
    topology_builder: TopologyBuilder,
}

impl IncrementalIndexer {
    pub fn new() -> Self {
        Self {
            topology_builder: TopologyBuilder::new(),
        }
    }

    /// Perform a full index of the repository.
    pub async fn full_index(&self, state: &OciState, root: &Path) -> Result<()> {
        tracing::info!("Starting full index of {}", root.display());

        // Discover files
        let discovery = crate::discovery::FileDiscovery::new();
        let files = discovery.discover(root)?;

        tracing::info!("Discovered {} files", files.len());

        // Index each file
        for file in &files {
            if let Err(e) = self.index_file(state, file).await {
                tracing::warn!("Failed to index {}: {}", file.display(), e);
            }
        }

        // Build topology
        self.topology_builder.build(state, root)?;

        // Update metadata
        *state.last_indexed.write() = Some(std::time::Instant::now());

        let stats = state.stats();
        tracing::info!(
            "Index complete: {} files, {} symbols, {} call edges",
            stats.file_count,
            stats.symbol_count,
            stats.call_edge_count
        );

        Ok(())
    }

    /// Index a single file.
    pub async fn index_file(&self, state: &OciState, path: &Path) -> Result<()> {
        // Get parser for this file type
        let lang_parser = match parsing::parser_for_file(path) {
            Some(p) => p,
            None => return Ok(()), // Skip unsupported files
        };

        // Read file contents
        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read {}", path.display()))?;

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&lang_parser.language())
            .context("Failed to set parser language")?;

        let tree = parser
            .parse(&contents, None)
            .context("Failed to parse file")?;

        // Extract symbols
        let symbols = lang_parser.extract_symbols(&tree, &contents, path, &state.interner)?;

        // Extract calls
        let calls = lang_parser.extract_calls(&tree, &contents, path, &state.interner)?;

        // Extract imports
        let imports = lang_parser.extract_imports(&tree, &contents, path)?;

        // Get or create file ID
        let file_id = state.get_or_create_file_id(&path.to_path_buf());

        // Store file contents
        state
            .file_contents
            .insert(path.to_path_buf(), Arc::from(contents));

        // Store symbols
        let mut file_symbol_names = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            file_symbol_names.push(symbol.scoped_name);
            state.add_symbol(symbol);
        }
        state.file_symbols.insert(file_id, file_symbol_names);

        // Store calls
        for call in calls {
            state.add_call_edge(call);
        }

        // Store imports
        state.imports.insert(file_id, imports);

        Ok(())
    }

    /// Update a single file (clear old data, re-index).
    pub async fn update_file(&self, state: &OciState, path: &Path) -> Result<()> {
        // Clear existing data for this file
        state.clear_file(&path.to_path_buf());

        // Re-index
        self.index_file(state, path).await
    }

    /// Remove a file from the index.
    pub fn remove_file(&self, state: &OciState, path: &Path) {
        state.clear_file(&path.to_path_buf());
        self.topology_builder
            .remove_file(state, path)
            .ok();
    }
}

impl Default for IncrementalIndexer {
    fn default() -> Self {
        Self::new()
    }
}
