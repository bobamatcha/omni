//! Global state management for the Omniscient Code Index.
//!
//! The OciState holds all three graph layers and provides thread-safe access
//! for concurrent queries and updates.

use crate::search::Bm25Index;
use crate::semantic::SemanticIndex;
use crate::types::*;
use dashmap::DashMap;
use lasso::ThreadedRodeo;
use parking_lot::RwLock;
use petgraph::stable_graph::{NodeIndex, StableGraph};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// Thread-safe string interner for symbol names.
pub type Interner = ThreadedRodeo;

/// The complete state of the Omniscient Code Index.
pub struct OciState {
    // ========================================================================
    // Layer 1: Module Topology
    // ========================================================================
    /// The module topology graph
    pub topology: RwLock<StableGraph<TopologyNode, TopologyEdge>>,
    /// Map from file path to topology node index
    pub path_to_node: DashMap<PathBuf, NodeIndex>,
    /// Topology metrics per node
    pub topology_metrics: DashMap<NodeIndex, TopologyMetrics>,

    // ========================================================================
    // Layer 2: Symbol Resolution
    // ========================================================================
    /// All symbol definitions, keyed by scoped name
    pub symbols: DashMap<InternedString, SymbolDef>,
    /// Simple name -> list of scoped names (for unscoped lookups)
    pub name_to_scoped: DashMap<InternedString, Vec<InternedString>>,
    /// File -> symbols defined in that file
    pub file_symbols: DashMap<FileId, Vec<InternedString>>,
    /// Call graph edges
    pub call_edges: RwLock<Vec<CallEdge>>,
    /// Import graph
    pub imports: DashMap<FileId, Vec<ImportInfo>>,

    // ========================================================================
    // Layer 3: Semantic Embeddings (lazy)
    // ========================================================================
    /// Semantic index, built on demand
    pub semantic_index: OnceLock<SemanticIndex>,

    // ========================================================================
    // File Management
    // ========================================================================
    /// Cached file contents for fast queries
    pub file_contents: DashMap<PathBuf, Arc<str>>,
    /// File path to FileId mapping
    pub file_ids: DashMap<PathBuf, FileId>,
    /// Next file ID counter
    file_id_counter: AtomicU32,

    // ========================================================================
    // Search Indices (lazy)
    // ========================================================================
    /// BM25 index, built on demand
    pub bm25_index: RwLock<Option<Bm25Index>>,

    // ========================================================================
    // Metadata
    // ========================================================================
    /// String interner for symbol names
    pub interner: Interner,
    /// Root path of the indexed repository
    pub root_path: PathBuf,
    /// Git commit hash at index time
    pub git_hash: RwLock<Option<String>>,
    /// Timestamp of last index operation
    pub last_indexed: RwLock<Option<Instant>>,
    /// Total number of indexed files
    pub file_count: AtomicU32,
    /// Total number of indexed symbols
    pub symbol_count: AtomicU32,
}

impl OciState {
    /// Create a new empty state for the given root path.
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            // Layer 1
            topology: RwLock::new(StableGraph::new()),
            path_to_node: DashMap::new(),
            topology_metrics: DashMap::new(),

            // Layer 2
            symbols: DashMap::new(),
            name_to_scoped: DashMap::new(),
            file_symbols: DashMap::new(),
            call_edges: RwLock::new(Vec::new()),
            imports: DashMap::new(),

            // Layer 3
            semantic_index: OnceLock::new(),

            // Files
            file_contents: DashMap::new(),
            file_ids: DashMap::new(),
            file_id_counter: AtomicU32::new(0),

            // Search
            bm25_index: RwLock::new(None),

            // Metadata
            interner: ThreadedRodeo::default(),
            root_path,
            git_hash: RwLock::new(None),
            last_indexed: RwLock::new(None),
            file_count: AtomicU32::new(0),
            symbol_count: AtomicU32::new(0),
        }
    }

    /// Get or create a FileId for a path.
    pub fn get_or_create_file_id(&self, path: &PathBuf) -> FileId {
        if let Some(id) = self.file_ids.get(path) {
            return *id;
        }
        let id = FileId(self.file_id_counter.fetch_add(1, Ordering::SeqCst));
        self.file_ids.insert(path.clone(), id);
        self.file_count.fetch_add(1, Ordering::SeqCst);
        id
    }

    /// Intern a string, returning a handle.
    pub fn intern(&self, s: &str) -> InternedString {
        self.interner.get_or_intern(s)
    }

    /// Resolve an interned string to its value.
    pub fn resolve(&self, s: InternedString) -> &str {
        self.interner.resolve(&s)
    }

    /// Add a symbol definition to the index.
    pub fn add_symbol(&self, symbol: SymbolDef) {
        let scoped = symbol.scoped_name;
        let simple = symbol.name;

        // Add to scoped lookup
        self.symbols.insert(scoped, symbol);

        // Add to simple name -> scoped mapping
        self.name_to_scoped
            .entry(simple)
            .or_insert_with(Vec::new)
            .push(scoped);

        self.symbol_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Add a call edge to the graph.
    pub fn add_call_edge(&self, edge: CallEdge) {
        self.call_edges.write().push(edge);
    }

    /// Clear all data for a file (for incremental updates).
    pub fn clear_file(&self, path: &PathBuf) {
        // Get file ID
        let file_id = match self.file_ids.get(path) {
            Some(id) => *id,
            None => return,
        };

        // Remove symbols from this file
        if let Some((_, symbols)) = self.file_symbols.remove(&file_id) {
            for scoped_name in symbols {
                // Remove from symbols map
                if let Some((_, sym)) = self.symbols.remove(&scoped_name) {
                    // Remove from name_to_scoped
                    if let Some(mut entry) = self.name_to_scoped.get_mut(&sym.name) {
                        entry.retain(|s| *s != scoped_name);
                    }
                    self.symbol_count.fetch_sub(1, Ordering::SeqCst);
                }
            }
        }

        // Remove imports
        self.imports.remove(&file_id);

        // Remove file contents
        self.file_contents.remove(path);

        // Clear call edges from this file (expensive, but necessary for correctness)
        {
            let mut edges = self.call_edges.write();
            edges.retain(|e| &e.location.file != path);
        }
    }

    /// Look up a symbol by scoped name.
    pub fn get_symbol(&self, scoped_name: InternedString) -> Option<SymbolDef> {
        self.symbols.get(&scoped_name).map(|r| r.clone())
    }

    /// Find all symbols with a given simple name.
    pub fn find_by_name(&self, name: &str) -> Vec<SymbolDef> {
        let name_key = match self.interner.get(name) {
            Some(k) => k,
            None => return Vec::new(),
        };

        self.name_to_scoped
            .get(&name_key)
            .map(|scoped_names| {
                scoped_names
                    .iter()
                    .filter_map(|s| self.symbols.get(s).map(|r| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Find callers of a symbol (by simple name).
    pub fn find_callers(&self, callee_name: &str) -> Vec<CallEdge> {
        let edges = self.call_edges.read();
        edges
            .iter()
            .filter(|e| e.callee_name == callee_name)
            .cloned()
            .collect()
    }

    /// Find callees of a symbol (by scoped name).
    pub fn find_callees(&self, caller_scoped: InternedString) -> Vec<CallEdge> {
        let edges = self.call_edges.read();
        edges
            .iter()
            .filter(|e| e.caller == caller_scoped)
            .cloned()
            .collect()
    }

    /// Get file contents, loading from disk if not cached.
    pub async fn get_file_contents(&self, path: &PathBuf) -> Option<Arc<str>> {
        if let Some(contents) = self.file_contents.get(path) {
            return Some(contents.clone());
        }

        // Load from disk
        match tokio::fs::read_to_string(path).await {
            Ok(s) => {
                let arc: Arc<str> = Arc::from(s);
                self.file_contents.insert(path.clone(), arc.clone());
                Some(arc)
            }
            Err(_) => None,
        }
    }

    /// Get statistics about the index.
    pub fn stats(&self) -> IndexStats {
        IndexStats {
            file_count: self.file_count.load(Ordering::SeqCst),
            symbol_count: self.symbol_count.load(Ordering::SeqCst),
            call_edge_count: self.call_edges.read().len() as u32,
            topology_node_count: self.topology.read().node_count() as u32,
            has_semantic_index: self.semantic_index.get().is_some(),
            has_bm25_index: self.bm25_index.read().is_some(),
        }
    }

    /// Reset all state to empty.
    pub fn reset(&self) {
        {
            let mut graph = self.topology.write();
            graph.clear();
        }
        self.path_to_node.clear();
        self.topology_metrics.clear();

        self.symbols.clear();
        self.name_to_scoped.clear();
        self.file_symbols.clear();
        self.call_edges.write().clear();
        self.imports.clear();

        self.file_contents.clear();
        self.file_ids.clear();
        self.file_id_counter.store(0, Ordering::SeqCst);

        *self.bm25_index.write() = None;

        *self.git_hash.write() = None;
        *self.last_indexed.write() = None;
        self.file_count.store(0, Ordering::SeqCst);
        self.symbol_count.store(0, Ordering::SeqCst);
    }
}

/// Statistics about the index.
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub file_count: u32,
    pub symbol_count: u32,
    pub call_edge_count: u32,
    pub topology_node_count: u32,
    pub has_semantic_index: bool,
    pub has_bm25_index: bool,
}

/// Thread-safe shared state handle.
pub type SharedState = Arc<OciState>;

/// Create a new shared state.
pub fn create_state(root_path: PathBuf) -> SharedState {
    Arc::new(OciState::new(root_path))
}
