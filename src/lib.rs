// Allow some clippy lints that are too strict for our codebase
#![allow(clippy::collapsible_if)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::manual_map)]
#![allow(clippy::manual_strip)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::double_ended_iterator_last)]
#![allow(clippy::cmp_owned)]
#![allow(clippy::unwrap_or_default)]

//! Omniscient Code Index (OCI)
//!
//! A semantic, interventionist code indexer for AI coding agents.
//!
//! # Architecture
//!
//! The OCI maintains three interconnected graph layers:
//!
//! 1. **Module Topology (Layer 1)**: High-level view of crates, modules, and files
//!    with import relationships and PageRank-based relevance scoring.
//!
//! 2. **Symbol Resolution (Layer 2)**: Precise symbol definitions and call graph
//!    using incremental analysis for fast updates.
//!
//! 3. **Semantic Embeddings (Layer 3)**: Vector embeddings for semantic search
//!    and duplicate detection.
//!
//! # Key Features
//!
//! - **Incremental Indexing**: Only re-parse changed files
//! - **Active Intervention**: Detect semantic duplicates before code is written
//! - **Dead Code Analysis**: Global reachability analysis across workspace
//! - **Test Coverage Integration**: Map coverage data to symbols
//! - **Virtual Context Documents**: Auto-generated architectural documentation
//!
//! # Usage
//!
//! ```ignore
//! use omni_index::{OciState, IncrementalIndexer};
//!
//! let state = OciState::new("/path/to/repo".into());
//! let indexer = IncrementalIndexer::new();
//! indexer.full_index(&state, &state.root_path).await?;
//!
//! // Search for symbols
//! let results = state.find_by_name("my_function");
//! ```

pub mod discovery;
pub mod incremental;
pub mod parsing;
pub mod state;
pub mod topology;
pub mod types;

// Phase 3+
pub mod analysis;
pub mod context;
pub mod intervention;
pub mod mcp;
pub mod search;
pub mod semantic;

// Re-exports
pub use analysis::DeadCodeAnalyzer;
pub use context::{ContextChunk, ContextQuery, ContextResult, ContextSynthesizer};
pub use discovery::FileDiscovery;
pub use incremental::IncrementalIndexer;
pub use intervention::InterventionEngine;
pub use search::{
    Bm25Index, HybridSearch, HybridSearchConfig, HybridSearchResult, SearchQualityMetrics,
};
pub use state::{IndexStats, OciState, SharedState, create_state};
pub use types::*;

/// Server name for MCP.
pub const SERVER_NAME: &str = "omni-index";
/// Server version.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
