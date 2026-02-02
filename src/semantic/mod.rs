//! Semantic embedding layer (Layer 3).
//!
//! Provides vector embeddings for semantic search and duplicate detection.

use crate::state::OciState;
use crate::types::InternedString;
use anyhow::{Context, Result};
use dashmap::DashMap;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use instant_distance::{Builder, HnswMap, Point, Search};
use parking_lot::RwLock;
use std::sync::Arc;

/// Wrapper for f32 vector to implement Point trait
#[derive(Debug, Clone)]
struct Embedding(Vec<f32>);

impl Point for Embedding {
    fn distance(&self, other: &Self) -> f32 {
        // Cosine distance = 1 - cosine similarity
        let dot: f32 = self.0.iter().zip(other.0.iter()).map(|(a, b)| a * b).sum();
        let norm_a: f32 = self.0.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.0.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 1.0;
        }

        1.0 - (dot / (norm_a * norm_b))
    }
}

/// Semantic index using HNSW for approximate nearest neighbor search
pub struct SemanticIndex {
    /// The embedding model
    model: Arc<TextEmbedding>,
    /// HNSW index for fast similarity search
    hnsw: RwLock<Option<HnswMap<Embedding, InternedString>>>,
    /// Map from symbol to embedding (for incremental updates)
    embeddings: DashMap<InternedString, Embedding>,
    /// Map from symbol to index in HNSW
    symbol_to_idx: DashMap<InternedString, usize>,
}

impl SemanticIndex {
    /// Create a new empty semantic index
    pub fn new() -> Result<Self> {
        // Initialize the embedding model (all-MiniLM-L6-v2)
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false),
        )
        .context("Failed to initialize embedding model")?;

        Ok(Self {
            model: Arc::new(model),
            hnsw: RwLock::new(None),
            embeddings: DashMap::new(),
            symbol_to_idx: DashMap::new(),
        })
    }

    /// Generate embedding for text
    fn embed_text(&self, text: &str) -> Result<Embedding> {
        let embeddings = self
            .model
            .embed(vec![text.to_string()], None)
            .context("Failed to generate embedding")?;

        if embeddings.is_empty() {
            anyhow::bail!("No embeddings generated");
        }

        Ok(Embedding(embeddings[0].clone()))
    }

    /// Build the HNSW index from stored embeddings
    fn rebuild_index(&self) -> Result<()> {
        let entries: Vec<_> = self
            .embeddings
            .iter()
            .map(|entry| (entry.value().clone(), *entry.key()))
            .collect();

        if entries.is_empty() {
            *self.hnsw.write() = None;
            self.symbol_to_idx.clear();
            return Ok(());
        }

        // Build HNSW index
        let values: Vec<_> = entries.iter().map(|(emb, _)| emb.clone()).collect();
        let symbols: Vec<_> = entries.iter().map(|(_, sym)| *sym).collect();
        let hnsw = Builder::default().build(values, symbols);

        // Update symbol to index mapping
        self.symbol_to_idx.clear();
        for (idx, (_, symbol)) in entries.iter().enumerate() {
            self.symbol_to_idx.insert(*symbol, idx);
        }

        *self.hnsw.write() = Some(hnsw);
        Ok(())
    }

    /// Add a symbol to the index
    pub fn add_symbol(&mut self, symbol: InternedString, text: &str) -> Result<()> {
        let embedding = self.embed_text(text)?;
        self.embeddings.insert(symbol, embedding);

        // Mark index as needing rebuild
        // We rebuild lazily on next search for efficiency
        *self.hnsw.write() = None;

        Ok(())
    }

    /// Remove a symbol from the index
    pub fn remove_symbol(&mut self, symbol: InternedString) -> Result<()> {
        self.embeddings.remove(&symbol);
        self.symbol_to_idx.remove(&symbol);

        // Mark index as needing rebuild
        *self.hnsw.write() = None;

        Ok(())
    }

    /// Search for k nearest symbols to the query
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<(InternedString, f32)>> {
        // Rebuild index if needed
        {
            let hnsw_guard = self.hnsw.read();
            if hnsw_guard.is_none() && !self.embeddings.is_empty() {
                drop(hnsw_guard);
                self.rebuild_index()?;
            }
        }

        let hnsw_guard = self.hnsw.read();
        let hnsw = match hnsw_guard.as_ref() {
            Some(h) => h,
            None => return Ok(Vec::new()), // Empty index
        };

        // Generate query embedding
        let query_emb = self.embed_text(query)?;

        // Search HNSW
        let mut search = Search::default();
        let neighbors = hnsw.search(&query_emb, &mut search);

        // Convert to results with similarity scores
        let results: Vec<_> = neighbors
            .take(k)
            .map(|item| {
                let symbol = *item.value;
                let distance = item.distance;
                // Convert distance to similarity (1 - distance for cosine)
                let similarity = 1.0 - distance;
                (symbol, similarity)
            })
            .collect();

        Ok(results)
    }

    /// Get the number of indexed symbols
    pub fn len(&self) -> usize {
        self.embeddings.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.embeddings.is_empty()
    }
}

/// Build a semantic index from the current state
pub fn build_index(state: &OciState) -> Result<SemanticIndex> {
    let mut index = SemanticIndex::new()?;

    // Iterate over all symbols and build embeddings
    for entry in state.symbols.iter() {
        let symbol_def = entry.value();
        let scoped_name = symbol_def.scoped_name;

        // Build embedding text from symbol information
        let embedding_text = build_embedding_text(state, symbol_def);

        // Add to index
        index.add_symbol(scoped_name, &embedding_text)?;
    }

    // Build the HNSW index
    index.rebuild_index()?;

    Ok(index)
}

/// Build the embedding text for a symbol
fn build_embedding_text(state: &OciState, symbol: &crate::types::SymbolDef) -> String {
    let mut parts = Vec::new();

    // 1. Symbol name
    let name = state.resolve(symbol.name);
    parts.push(format!("Symbol: {}", name));

    // 2. Symbol kind
    parts.push(format!("Kind: {}", symbol.kind.as_str()));

    // 3. File path context
    let file_path = &symbol.location.file;
    if let Some(file_name) = file_path.file_name() {
        parts.push(format!("File: {}", file_name.to_string_lossy()));
    }

    // 4. Doc comments
    if let Some(doc) = &symbol.doc_comment {
        parts.push(format!("Documentation: {}", doc));
    }

    // 5. Signature (for functions/methods)
    if let Some(sig) = &symbol.signature {
        let mut sig_parts = Vec::new();

        if sig.is_async {
            sig_parts.push("async".to_string());
        }
        if sig.is_unsafe {
            sig_parts.push("unsafe".to_string());
        }
        if sig.is_const {
            sig_parts.push("const".to_string());
        }

        sig_parts.push("fn".to_string());
        sig_parts.push(name.to_string());

        if let Some(generics) = &sig.generics {
            sig_parts.push(generics.clone());
        }

        sig_parts.push(format!("({})", sig.params.join(", ")));

        if let Some(ret) = &sig.return_type {
            sig_parts.push("->".to_string());
            sig_parts.push(ret.clone());
        }

        parts.push(format!("Signature: {}", sig_parts.join(" ")));
    }

    // 6. Parent context (for methods and fields)
    if let Some(parent) = symbol.parent {
        let parent_name = state.resolve(parent);
        parts.push(format!("Parent: {}", parent_name));
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Location, SymbolDef, SymbolKind, Visibility};
    use std::path::PathBuf;

    #[test]
    fn test_semantic_index_basic() -> Result<()> {
        let mut index = match SemanticIndex::new() {
            Ok(index) => index,
            Err(err) => {
                eprintln!("Skipping semantic test: {err}");
                return Ok(());
            }
        };

        // Create a state for testing
        let state = OciState::new(PathBuf::from("/test"));

        // Add some test symbols
        let name1 = state.intern("test_function");
        let name2 = state.intern("another_function");

        index.add_symbol(name1, "Test function for addition")?;
        index.add_symbol(name2, "Test function for subtraction")?;

        // Search
        let results = index.search("addition", 1)?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, name1);

        Ok(())
    }

    #[test]
    fn test_add_remove_symbol() -> Result<()> {
        let mut index = match SemanticIndex::new() {
            Ok(index) => index,
            Err(err) => {
                eprintln!("Skipping semantic test: {err}");
                return Ok(());
            }
        };
        let state = OciState::new(PathBuf::from("/test"));

        let name = state.intern("test_symbol");
        index.add_symbol(name, "A test symbol")?;
        assert_eq!(index.len(), 1);

        index.remove_symbol(name)?;
        assert_eq!(index.len(), 0);

        Ok(())
    }

    #[test]
    fn test_build_embedding_text() {
        let state = OciState::new(PathBuf::from("/test/project"));
        let name = state.intern("my_function");
        let scoped = state.intern("module::my_function");

        let symbol = SymbolDef {
            name,
            scoped_name: scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/project/src/lib.rs"), 0, 100),
            signature: Some(crate::types::Signature {
                params: vec!["x: i32".to_string(), "y: i32".to_string()],
                return_type: Some("i32".to_string()),
                is_async: false,
                is_unsafe: false,
                is_const: false,
                generics: None,
                where_clause: None,
            }),
            visibility: Visibility::Public,
            attributes: vec![],
            doc_comment: Some("Adds two numbers together".to_string()),
            parent: None,
        };

        let text = build_embedding_text(&state, &symbol);

        assert!(text.contains("Symbol: my_function"));
        assert!(text.contains("Kind: function"));
        assert!(text.contains("File: lib.rs"));
        assert!(text.contains("Documentation: Adds two numbers together"));
        assert!(text.contains("Signature: fn my_function (x: i32, y: i32) -> i32"));
    }
}
