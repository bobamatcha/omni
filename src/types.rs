//! Core types for the Omniscient Code Index.
//!
//! This module defines the fundamental data structures used across all layers:
//! - Module Topology (Layer 1)
//! - Symbol Resolution (Layer 2)
//! - Semantic Embeddings (Layer 3)

use lasso::Spur;
use std::path::PathBuf;

/// Interned string handle for memory-efficient symbol storage.
pub type InternedString = Spur;

/// Unique identifier for symbols in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Unique identifier for files in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

// ============================================================================
// Layer 1: Module Topology Types
// ============================================================================

/// Node types for the Module Topology Graph.
#[derive(Debug, Clone)]
pub enum TopologyNode {
    /// Workspace or crate root
    Crate {
        name: String,
        path: PathBuf,
        is_workspace: bool,
    },
    /// A module (from mod declaration or directory)
    Module {
        name: String,
        path: PathBuf,
        is_inline: bool,
    },
    /// A source file
    File {
        path: PathBuf,
        file_id: FileId,
    },
}

/// Edge types for the Module Topology Graph.
#[derive(Debug, Clone)]
pub enum TopologyEdge {
    /// Parent contains child (crate -> module, module -> file)
    Contains,
    /// Import relationship from `use` statement
    Imports {
        use_path: String,
        is_glob: bool,
    },
    /// Re-export via `pub use`
    ReExports {
        original_path: String,
    },
}

/// Metrics for a topology node.
#[derive(Debug, Clone, Default)]
pub struct TopologyMetrics {
    /// PageRank score for relevance ranking
    pub relevance_score: f64,
    /// Number of modifications in recent history
    pub churn_count: u32,
    /// Test coverage percentage (0.0 - 1.0)
    pub coverage: Option<f32>,
}

// ============================================================================
// Layer 2: Symbol Resolution Types
// ============================================================================

/// Location of a syntax element in a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub file: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl Location {
    pub fn new(file: PathBuf, start_byte: usize, end_byte: usize) -> Self {
        Self {
            file,
            start_byte,
            end_byte,
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 0,
        }
    }

    pub fn with_positions(
        mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> Self {
        self.start_line = start_line;
        self.start_col = start_col;
        self.end_line = end_line;
        self.end_col = end_col;
        self
    }
}

/// Kind of symbol in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    Module,
    TypeAlias,
    Macro,
    Field,
    Variant,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Const => "const",
            Self::Static => "static",
            Self::Module => "module",
            Self::TypeAlias => "type",
            Self::Macro => "macro",
            Self::Field => "field",
            Self::Variant => "variant",
        }
    }
}

/// Visibility of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Private,
    /// pub(crate)
    Crate,
    /// pub(super)
    Super,
    /// pub(in path)
    Restricted,
    /// pub
    Public,
}

/// Function/method signature information.
#[derive(Debug, Clone, Default)]
pub struct Signature {
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub generics: Option<String>,
    pub where_clause: Option<String>,
}

/// A symbol definition in the codebase.
#[derive(Debug, Clone)]
pub struct SymbolDef {
    /// Simple name (e.g., "foo")
    pub name: InternedString,
    /// Fully qualified scoped name (e.g., "crate::module::Struct::foo")
    pub scoped_name: InternedString,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Source location
    pub location: Location,
    /// Signature (for functions/methods)
    pub signature: Option<Signature>,
    /// Visibility
    pub visibility: Visibility,
    /// Attributes (e.g., #[test], #[derive(...)])
    pub attributes: Vec<String>,
    /// Documentation comments
    pub doc_comment: Option<String>,
    /// Parent symbol (for methods -> impl, fields -> struct)
    pub parent: Option<InternedString>,
}

/// A call edge in the call graph.
#[derive(Debug, Clone)]
pub struct CallEdge {
    /// Scoped name of the caller
    pub caller: InternedString,
    /// Simple name of the callee (unscoped for dynamic resolution)
    pub callee_name: String,
    /// Location of the call site
    pub location: Location,
    /// Whether this is a method call (has receiver)
    pub is_method_call: bool,
}

/// Import information from `use` statements.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    /// The full use path (e.g., "std::collections::HashMap")
    pub path: String,
    /// Imported name or alias
    pub name: String,
    /// Whether it's a glob import (use foo::*)
    pub is_glob: bool,
    /// Location of the use statement
    pub location: Location,
}

// ============================================================================
// Layer 3: Semantic Types
// ============================================================================

/// Entry in the semantic embedding index.
#[derive(Debug, Clone)]
pub struct SemanticEntry {
    pub symbol_id: SymbolId,
    pub scoped_name: InternedString,
    /// Embedding vector (may be quantized)
    pub embedding: EmbeddingData,
}

/// Embedding storage format.
#[derive(Debug, Clone)]
pub enum EmbeddingData {
    /// Full float32 embedding
    Float32(Vec<f32>),
    /// Binary quantized for memory efficiency
    Binary(Vec<u8>),
}

impl EmbeddingData {
    pub fn dimension(&self) -> usize {
        match self {
            Self::Float32(v) => v.len(),
            Self::Binary(v) => v.len() * 8,
        }
    }
}

// ============================================================================
// Analysis Types
// ============================================================================

/// Result of dead code analysis.
#[derive(Debug, Clone)]
pub struct DeadCodeReport {
    /// Symbols that are potentially dead (unreachable)
    pub dead_symbols: Vec<InternedString>,
    /// Entry points used for reachability analysis
    pub entry_points: Vec<InternedString>,
    /// Symbols marked as potentially live (conservative)
    pub potentially_live: Vec<InternedString>,
}

/// Coverage data for a symbol.
#[derive(Debug, Clone)]
pub struct SymbolCoverage {
    pub symbol: InternedString,
    pub lines_covered: u32,
    pub lines_total: u32,
    pub branches_covered: u32,
    pub branches_total: u32,
}

// ============================================================================
// Intervention Types
// ============================================================================

/// Severity of an intervention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterventionSeverity {
    /// Informational - similar code exists
    Info,
    /// Warning - likely duplicate
    Warning,
    /// Block - high confidence duplicate, should use existing
    Block,
}

/// An intervention triggered by duplicate detection.
#[derive(Debug, Clone)]
pub struct Intervention {
    pub severity: InterventionSeverity,
    pub message: String,
    pub existing_symbol: InternedString,
    pub existing_location: Location,
    pub similarity_score: f32,
    pub recommendation: String,
}

/// Result of similarity detection.
#[derive(Debug, Clone)]
pub struct SimilarityMatch {
    pub symbol: InternedString,
    pub location: Location,
    pub score: f32,
    pub kind: SymbolKind,
}

// ============================================================================
// Query Types
// ============================================================================

/// Options for search queries.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Maximum results to return
    pub limit: Option<usize>,
    /// Filter by symbol kind
    pub kind_filter: Option<Vec<SymbolKind>>,
    /// Filter by file pattern (glob)
    pub file_pattern: Option<String>,
    /// Include private symbols
    pub include_private: bool,
    /// Lines of context before match
    pub context_before: usize,
    /// Lines of context after match
    pub context_after: usize,
}

/// A search result with snippet.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub symbol: InternedString,
    pub kind: SymbolKind,
    pub location: Location,
    pub score: f32,
    pub snippet: Option<String>,
}
