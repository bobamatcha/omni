//! Parsing module for extracting symbols from source code.
//!
//! Uses tree-sitter for incremental, error-tolerant parsing.

pub mod rust;
pub mod typescript;

use crate::types::*;
use anyhow::Result;
use std::path::Path;
use tree_sitter::{Language, Tree};

/// Trait for language-specific parsers.
pub trait LanguageParser: Send + Sync {
    /// Get the tree-sitter language.
    fn language(&self) -> Language;

    /// File extensions this parser handles.
    fn extensions(&self) -> &[&str];

    /// Extract symbol definitions from a parsed tree.
    fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        file: &Path,
        interner: &lasso::ThreadedRodeo,
    ) -> Result<Vec<SymbolDef>>;

    /// Extract call edges from a parsed tree.
    fn extract_calls(
        &self,
        tree: &Tree,
        source: &str,
        file: &Path,
        interner: &lasso::ThreadedRodeo,
    ) -> Result<Vec<CallEdge>>;

    /// Extract import information from a parsed tree.
    fn extract_imports(&self, tree: &Tree, source: &str, file: &Path) -> Result<Vec<ImportInfo>>;
}

/// Get a parser for a file based on its extension.
pub fn parser_for_file(path: &Path) -> Option<Box<dyn LanguageParser>> {
    let ext = path.extension()?.to_str()?;
    match ext.to_lowercase().as_str() {
        "rs" => Some(Box::new(rust::RustParser::new())),
        "ts" | "mts" | "cts" => Some(Box::new(typescript::TypeScriptParser::new_typescript())),
        "tsx" => Some(Box::new(typescript::TypeScriptParser::new_tsx())),
        _ => None,
    }
}
