//! Context synthesis ("Ghost Docs").
//!
//! Auto-generates architectural context documents by intelligently assembling
//! relevant code snippets based on call graphs, type relationships, and PageRank scores.

use crate::state::OciState;
use crate::types::{InternedString, SymbolKind};
use anyhow::{Context as _, Result};
use std::collections::HashSet;
use std::path::PathBuf;

// ============================================================================
// Public Types
// ============================================================================

/// Query for context assembly.
#[derive(Debug, Clone)]
pub struct ContextQuery {
    /// The file of interest
    pub file: PathBuf,
    /// The line number of interest
    pub line: u32,
    /// Number of lines to include around the query point
    pub surrounding_lines: u32,
    /// Optional intent describing what the user is trying to do
    pub intent: Option<String>,
    /// Maximum tokens to include in the result
    pub max_tokens: usize,
}

impl ContextQuery {
    /// Create a new context query.
    pub fn new(file: PathBuf, line: u32) -> Self {
        Self {
            file,
            line,
            surrounding_lines: 5,
            intent: None,
            max_tokens: 4000,
        }
    }

    /// Set the number of surrounding lines to include.
    pub fn with_surrounding_lines(mut self, lines: u32) -> Self {
        self.surrounding_lines = lines;
        self
    }

    /// Set the intent description.
    pub fn with_intent(mut self, intent: String) -> Self {
        self.intent = Some(intent);
        self
    }

    /// Set the maximum token budget.
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

/// Result of context assembly.
#[derive(Debug, Clone)]
pub struct ContextResult {
    /// Primary chunks - most relevant to the query
    pub primary: Vec<ContextChunk>,
    /// Related chunks - less relevant but still useful
    pub related: Vec<ContextChunk>,
    /// Total estimated tokens in the result
    pub total_tokens: usize,
}

impl ContextResult {
    /// Create an empty context result.
    pub fn empty() -> Self {
        Self {
            primary: Vec::new(),
            related: Vec::new(),
            total_tokens: 0,
        }
    }

    /// Get all chunks in priority order (primary first, then related).
    pub fn all_chunks(&self) -> Vec<&ContextChunk> {
        self.primary.iter().chain(self.related.iter()).collect()
    }
}

/// A chunk of context with metadata.
#[derive(Debug, Clone)]
pub struct ContextChunk {
    /// Optional symbol this chunk represents
    pub symbol: Option<InternedString>,
    /// File containing this chunk
    pub file: PathBuf,
    /// The actual content
    pub content: String,
    /// Relevance score (0.0 - 1.0)
    pub relevance: f64,
    /// Explanation of why this was included
    pub reason: String,
}

impl ContextChunk {
    /// Estimate the number of tokens in this chunk.
    /// Uses a simple heuristic: ~4 chars per token.
    pub fn estimate_tokens(&self) -> usize {
        self.content.len() / 4
    }
}

// ============================================================================
// Context Synthesizer
// ============================================================================

/// Synthesizes relevant context from the code index.
pub struct ContextSynthesizer;

impl ContextSynthesizer {
    /// Create a new context synthesizer.
    pub fn new() -> Self {
        Self
    }

    /// Build context for a given query.
    pub async fn build_context(
        &self,
        state: &OciState,
        query: &ContextQuery,
    ) -> Result<ContextResult> {
        // Step 1: Find the symbol at the query location
        let symbol_at_location = self.find_symbol_at_location(state, &query.file, query.line);

        // Step 2: Collect candidate symbols
        let mut candidates = Vec::new();

        if let Some(current_symbol) = symbol_at_location {
            // Add the current symbol with highest priority
            candidates.push((current_symbol, 1.0, "Current location".to_string()));

            // Find callees (functions this symbol calls)
            let callees = state.find_callees(current_symbol);
            for call_edge in callees {
                // Try to resolve the callee to a scoped name
                if let Some(resolved) = self.resolve_callee(state, &call_edge.callee_name) {
                    candidates.push((
                        resolved,
                        0.8,
                        format!("Called by {}", state.resolve(current_symbol)),
                    ));
                }
            }

            // Find callers (functions that call this symbol)
            let current_name = state.resolve(current_symbol);
            let callers = state.find_callers(current_name);
            for call_edge in callers.iter().take(5) {
                // Limit callers to avoid explosion
                candidates.push((call_edge.caller, 0.6, format!("Calls {}", current_name)));
            }

            // Find related types (from signatures)
            if let Some(symbol_def) = state.get_symbol(current_symbol) {
                if let Some(sig) = &symbol_def.signature {
                    // Extract types from parameters and return type
                    let types = self.extract_types_from_signature(sig);
                    for type_name in types {
                        if let Some(type_symbol) = self.find_type_symbol(state, &type_name) {
                            candidates.push((
                                type_symbol,
                                0.5,
                                format!("Type used in signature: {}", type_name),
                            ));
                        }
                    }
                }

                // Find parent symbol (e.g., impl block for methods)
                if let Some(parent) = symbol_def.parent {
                    candidates.push((parent, 0.7, format!("Parent of {}", current_name)));
                }
            }
        }

        // Find relevant imports
        let file_id = state.get_or_create_file_id(&query.file);
        if let Some(imports) = state.imports.get(&file_id) {
            for import in imports.iter().take(5) {
                // Limit imports
                // Try to find symbols matching the imported names
                if let Some(import_symbol) = self.find_symbol_by_name(state, &import.name) {
                    candidates.push((import_symbol, 0.4, format!("Imported: {}", import.name)));
                }
            }
        }

        // Step 3: Rank all candidates
        let ranked = self.rank_symbols_with_reasons(state, candidates);

        // Step 4: Build chunks with token budget
        let mut primary_chunks = Vec::new();
        let mut related_chunks = Vec::new();
        let mut total_tokens = 0;
        let mut seen_symbols = HashSet::new();

        // First, add the query location itself
        if let Ok(location_chunk) = self
            .create_location_chunk(state, &query.file, query.line, query.surrounding_lines)
            .await
        {
            total_tokens += location_chunk.estimate_tokens();
            primary_chunks.push(location_chunk);
        }

        // Then add ranked symbols
        for (symbol, score, reason) in ranked {
            if total_tokens >= query.max_tokens {
                break;
            }

            // Avoid duplicates
            if seen_symbols.contains(&symbol) {
                continue;
            }
            seen_symbols.insert(symbol);

            // Create chunk for this symbol
            if let Ok(chunk) = self.create_symbol_chunk(state, symbol, score, reason).await {
                let chunk_tokens = chunk.estimate_tokens();

                if total_tokens + chunk_tokens > query.max_tokens {
                    // Would exceed budget - skip
                    continue;
                }

                total_tokens += chunk_tokens;

                // Categorize as primary or related based on score
                if score >= 0.6 {
                    primary_chunks.push(chunk);
                } else {
                    related_chunks.push(chunk);
                }
            }
        }

        Ok(ContextResult {
            primary: primary_chunks,
            related: related_chunks,
            total_tokens,
        })
    }

    /// Rank symbols by relevance, returning (symbol, score) pairs.
    pub fn rank_symbols(
        &self,
        state: &OciState,
        symbols: &[InternedString],
    ) -> Vec<(InternedString, f64)> {
        let candidates: Vec<_> = symbols
            .iter()
            .map(|s| (*s, 1.0, "Candidate".to_string()))
            .collect();

        self.rank_symbols_with_reasons(state, candidates)
            .into_iter()
            .map(|(sym, score, _)| (sym, score))
            .collect()
    }

    // ========================================================================
    // Private Helper Methods
    // ========================================================================

    /// Find the symbol at a given file location.
    fn find_symbol_at_location(
        &self,
        state: &OciState,
        file: &PathBuf,
        line: u32,
    ) -> Option<InternedString> {
        // Get symbols in this file
        let file_id = state.file_ids.get(file)?;
        let file_symbols = state.file_symbols.get(&file_id)?;

        // Find symbol containing this line
        for scoped_name in file_symbols.iter() {
            if let Some(symbol) = state.get_symbol(*scoped_name) {
                if symbol.location.start_line <= line as usize
                    && symbol.location.end_line >= line as usize
                {
                    return Some(*scoped_name);
                }
            }
        }

        None
    }

    /// Resolve a callee name to a scoped symbol.
    fn resolve_callee(&self, state: &OciState, callee_name: &str) -> Option<InternedString> {
        // Find symbols with this simple name
        let symbols = state.find_by_name(callee_name);
        if symbols.is_empty() {
            return None;
        }

        // Prefer public symbols
        for symbol in &symbols {
            if matches!(symbol.visibility, crate::types::Visibility::Public) {
                return Some(symbol.scoped_name);
            }
        }

        // Otherwise, return the first one
        symbols.first().map(|s| s.scoped_name)
    }

    /// Extract type names from a signature.
    fn extract_types_from_signature(&self, sig: &crate::types::Signature) -> Vec<String> {
        let mut types = Vec::new();

        // Extract from parameters
        for param in &sig.params {
            if let Some(type_name) = self.extract_type_name(param) {
                types.push(type_name);
            }
        }

        // Extract from return type
        if let Some(ret_type) = &sig.return_type {
            if let Some(type_name) = self.extract_type_name(ret_type) {
                types.push(type_name);
            }
        }

        types
    }

    /// Extract a type name from a parameter or return type string.
    fn extract_type_name(&self, type_str: &str) -> Option<String> {
        // Simple heuristic: extract the main type name
        // e.g., "Vec<String>" -> "Vec", "&mut Foo" -> "Foo"
        let trimmed = type_str.trim();

        // Remove reference markers
        let without_refs = trimmed.trim_start_matches('&').trim_start_matches("mut ");

        // Take the part before '<' or whitespace
        let main_type = without_refs.split('<').next()?.split_whitespace().next()?;

        if main_type.is_empty() || main_type.starts_with(char::is_lowercase) {
            // Likely a primitive type or keyword
            None
        } else {
            Some(main_type.to_string())
        }
    }

    /// Find a type symbol (struct, enum, trait) by name.
    fn find_type_symbol(&self, state: &OciState, type_name: &str) -> Option<InternedString> {
        let symbols = state.find_by_name(type_name);

        for symbol in symbols {
            if matches!(
                symbol.kind,
                SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait
            ) {
                return Some(symbol.scoped_name);
            }
        }

        None
    }

    /// Find a symbol by simple name (prefer public, prefer higher PageRank).
    fn find_symbol_by_name(&self, state: &OciState, name: &str) -> Option<InternedString> {
        let symbols = state.find_by_name(name);
        if symbols.is_empty() {
            return None;
        }

        // Rank them and pick the best
        let scoped_names: Vec<_> = symbols.iter().map(|s| s.scoped_name).collect();
        let ranked = self.rank_symbols(state, &scoped_names);

        ranked.first().map(|(sym, _)| *sym)
    }

    /// Rank symbols with reasons, considering PageRank and other factors.
    fn rank_symbols_with_reasons(
        &self,
        state: &OciState,
        candidates: Vec<(InternedString, f64, String)>,
    ) -> Vec<(InternedString, f64, String)> {
        let mut scored: Vec<_> = candidates
            .into_iter()
            .map(|(symbol, base_score, reason)| {
                let pagerank_score = self.get_pagerank_score(state, symbol);
                let combined_score = base_score * 0.7 + pagerank_score * 0.3;
                (symbol, combined_score, reason)
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
    }

    /// Get the PageRank score for a symbol.
    fn get_pagerank_score(&self, state: &OciState, symbol: InternedString) -> f64 {
        // Get the symbol definition
        let symbol_def = match state.get_symbol(symbol) {
            Some(s) => s,
            None => return 0.0,
        };

        // Find the topology node for this file
        let node_idx = match state.path_to_node.get(&symbol_def.location.file) {
            Some(idx) => *idx,
            None => return 0.0,
        };

        // Get the metrics for this node
        state
            .topology_metrics
            .get(&node_idx)
            .map(|m| m.relevance_score)
            .unwrap_or(0.0)
    }

    /// Create a chunk for a specific file location.
    async fn create_location_chunk(
        &self,
        state: &OciState,
        file: &PathBuf,
        line: u32,
        surrounding_lines: u32,
    ) -> Result<ContextChunk> {
        let contents = state
            .get_file_contents(file)
            .await
            .context("Failed to read file")?;

        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        let start_line = (line as i32 - surrounding_lines as i32).max(0) as usize;
        let end_line = ((line + surrounding_lines) as usize).min(total_lines);

        let content = lines[start_line..end_line].join("\n");

        Ok(ContextChunk {
            symbol: None,
            file: file.clone(),
            content,
            relevance: 1.0,
            reason: format!("Query location at line {}", line),
        })
    }

    /// Create a chunk for a symbol.
    async fn create_symbol_chunk(
        &self,
        state: &OciState,
        symbol: InternedString,
        relevance: f64,
        reason: String,
    ) -> Result<ContextChunk> {
        let symbol_def = state
            .get_symbol(symbol)
            .context("Symbol not found in state")?;

        let contents = state
            .get_file_contents(&symbol_def.location.file)
            .await
            .context("Failed to read file")?;

        let lines: Vec<&str> = contents.lines().collect();

        // Extract the symbol's definition
        let start_line = symbol_def.location.start_line.saturating_sub(1);
        let end_line = symbol_def.location.end_line;

        if start_line >= lines.len() || end_line > lines.len() {
            anyhow::bail!("Symbol location out of bounds");
        }

        let content = lines[start_line..end_line].join("\n");

        Ok(ContextChunk {
            symbol: Some(symbol),
            file: symbol_def.location.file.clone(),
            content,
            relevance,
            reason,
        })
    }
}

impl Default for ContextSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::create_state;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_build_empty_context() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let synthesizer = ContextSynthesizer::new();

        let test_file = temp.path().join("test.rs");
        std::fs::write(&test_file, "fn main() {}").unwrap();

        let query = ContextQuery::new(test_file, 1);
        let result = synthesizer.build_context(&state, &query).await;

        // Should succeed even with no indexed data
        assert!(result.is_ok());
    }

    #[test]
    fn test_rank_symbols_empty() {
        let temp = TempDir::new().unwrap();
        let state = create_state(temp.path().to_path_buf());
        let synthesizer = ContextSynthesizer::new();

        let ranked = synthesizer.rank_symbols(&state, &[]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn test_extract_type_name() {
        let synthesizer = ContextSynthesizer::new();

        assert_eq!(
            synthesizer.extract_type_name("Vec<String>"),
            Some("Vec".to_string())
        );
        assert_eq!(
            synthesizer.extract_type_name("&mut Foo"),
            Some("Foo".to_string())
        );
        assert_eq!(
            synthesizer.extract_type_name("&Bar"),
            Some("Bar".to_string())
        );
        assert_eq!(synthesizer.extract_type_name("i32"), None); // Primitive
    }

    #[tokio::test]
    async fn test_context_chunk_token_estimation() {
        let chunk = ContextChunk {
            symbol: None,
            file: PathBuf::from("test.rs"),
            content: "a".repeat(400), // 400 chars
            relevance: 1.0,
            reason: "Test".to_string(),
        };

        // Should be ~100 tokens (400 / 4)
        assert_eq!(chunk.estimate_tokens(), 100);
    }

    #[tokio::test]
    async fn test_context_query_builder() {
        let query = ContextQuery::new(PathBuf::from("test.rs"), 10)
            .with_surrounding_lines(3)
            .with_max_tokens(2000)
            .with_intent("Testing".to_string());

        assert_eq!(query.line, 10);
        assert_eq!(query.surrounding_lines, 3);
        assert_eq!(query.max_tokens, 2000);
        assert_eq!(query.intent, Some("Testing".to_string()));
    }

    #[test]
    fn test_context_result_all_chunks() {
        let mut result = ContextResult::empty();

        result.primary.push(ContextChunk {
            symbol: None,
            file: PathBuf::from("a.rs"),
            content: "primary".to_string(),
            relevance: 1.0,
            reason: "test".to_string(),
        });

        result.related.push(ContextChunk {
            symbol: None,
            file: PathBuf::from("b.rs"),
            content: "related".to_string(),
            relevance: 0.5,
            reason: "test".to_string(),
        });

        let all = result.all_chunks();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].content, "primary");
        assert_eq!(all[1].content, "related");
    }
}
