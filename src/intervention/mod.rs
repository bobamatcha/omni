//! Active intervention system for duplicate detection.
//!
//! Provides real-time duplicate detection and suggestions before code is written.
//! This module implements the "killer feature" of the OCI - active intervention to
//! prevent code duplication before it happens.

use crate::state::OciState;
use crate::types::*;
use std::path::Path;

/// Parsed signature components for comparison.
#[derive(Debug)]
#[allow(dead_code)]
struct ParsedSignature {
    name: String,
    params: Vec<String>,
    return_type: Option<String>,
    is_async: bool,
    is_unsafe: bool,
}

/// Engine for detecting duplicates and providing interventions
pub struct InterventionEngine {
    /// Similarity threshold for interventions
    threshold: f32,
}

impl InterventionEngine {
    /// Create a new intervention engine
    pub fn new() -> Self {
        Self {
            threshold: 0.85, // Default threshold
        }
    }

    /// Set the similarity threshold for interventions
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Detect similar existing functions based on a proposed signature.
    ///
    /// This analyzes the proposed function signature and finds existing functions
    /// that are similar based on:
    /// - Name similarity (Levenshtein distance)
    /// - Parameter count and types
    /// - Return type
    ///
    /// # Arguments
    /// * `state` - The OCI state containing all indexed symbols
    /// * `proposed_signature` - A function signature string (e.g., "fn foo(x: i32) -> bool")
    ///
    /// # Returns
    /// A list of similarity matches, sorted by score (highest first)
    pub fn detect_duplication(state: &OciState, proposed_signature: &str) -> Vec<SimilarityMatch> {
        // Parse the proposed signature
        let parsed = match Self::parse_signature(proposed_signature) {
            Some(p) => p,
            None => return Vec::new(),
        };

        let mut matches = Vec::new();

        // Iterate through all symbols in the index
        for entry in state.symbols.iter() {
            let symbol = entry.value();

            // Only check functions and methods
            if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }

            // Skip if no signature available
            let sig = match &symbol.signature {
                Some(s) => s,
                None => continue,
            };

            // Calculate similarity score
            let score = Self::calculate_signature_similarity(&parsed, symbol, sig, state);

            // Only include matches with meaningful similarity (> 0.3)
            if score > 0.3 {
                matches.push(SimilarityMatch {
                    symbol: symbol.scoped_name,
                    location: symbol.location.clone(),
                    score,
                    kind: symbol.kind,
                });
            }
        }

        // Sort by score (highest first)
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        matches
    }

    /// Suggest existing code alternatives for a given name.
    ///
    /// This searches for existing symbols that could potentially be reused instead
    /// of creating new code. It looks for:
    /// - Exact name matches
    /// - Prefix/suffix matches
    /// - Similar functionality based on naming patterns
    ///
    /// # Arguments
    /// * `state` - The OCI state
    /// * `name` - The proposed symbol name
    ///
    /// # Returns
    /// A list of interventions suggesting alternatives
    pub fn suggest_alternatives(state: &OciState, name: &str) -> Vec<Intervention> {
        let mut interventions = Vec::new();

        // Search for exact matches
        let exact_matches = state.find_by_name(name);
        for symbol in exact_matches {
            interventions.push(Intervention {
                severity: InterventionSeverity::Warning,
                message: format!(
                    "Symbol '{}' already exists at {}:{}",
                    state.resolve(symbol.name),
                    symbol.location.file.display(),
                    symbol.location.start_line
                ),
                existing_symbol: symbol.scoped_name,
                existing_location: symbol.location.clone(),
                similarity_score: 1.0,
                recommendation: format!(
                    "Consider reusing the existing {} instead of creating a new one",
                    symbol.kind.as_str()
                ),
            });
        }

        // Search for similar names using fuzzy matching
        let name_lower = name.to_lowercase();
        for entry in state.symbols.iter() {
            let symbol = entry.value();
            let symbol_name = state.resolve(symbol.name);
            let symbol_name_lower = symbol_name.to_lowercase();

            // Skip exact matches (already handled)
            if symbol_name == name {
                continue;
            }

            // Check for prefix/suffix matches
            let has_prefix = name_lower.starts_with(&symbol_name_lower)
                || symbol_name_lower.starts_with(&name_lower);
            let has_suffix = name_lower.ends_with(&symbol_name_lower)
                || symbol_name_lower.ends_with(&name_lower);

            if has_prefix || has_suffix {
                let score = strsim::jaro_winkler(&name_lower, &symbol_name_lower) as f32;
                if score > 0.7 {
                    interventions.push(Intervention {
                        severity: InterventionSeverity::Info,
                        message: format!(
                            "Similar symbol '{}' exists at {}:{}",
                            symbol_name,
                            symbol.location.file.display(),
                            symbol.location.start_line
                        ),
                        existing_symbol: symbol.scoped_name,
                        existing_location: symbol.location.clone(),
                        similarity_score: score,
                        recommendation: format!(
                            "Check if '{}' provides similar functionality before implementing '{}'",
                            symbol_name, name
                        ),
                    });
                }
            }

            // Limit to top 10 suggestions
            if interventions.len() >= 10 {
                break;
            }
        }

        // Sort by similarity score
        interventions.sort_by(|a, b| {
            b.similarity_score
                .partial_cmp(&a.similarity_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        interventions
    }

    /// Check for naming conflicts in a specific file context.
    ///
    /// This checks if a proposed name would cause conflicts:
    /// - Same name already exists in the file
    /// - Similar names that might be typos
    /// - Import conflicts
    ///
    /// # Arguments
    /// * `state` - The OCI state
    /// * `name` - The proposed symbol name
    /// * `file` - The file path where the symbol would be defined
    ///
    /// # Returns
    /// A list of interventions for naming conflicts
    pub fn check_naming_conflicts(state: &OciState, name: &str, file: &Path) -> Vec<Intervention> {
        let mut interventions = Vec::new();

        // Get FileId for the file
        let file_id = match state.file_ids.get(&file.to_path_buf()) {
            Some(id) => *id,
            None => return interventions, // File not indexed yet
        };

        // Check symbols in the same file
        if let Some(file_symbols) = state.file_symbols.get(&file_id) {
            for scoped_name in file_symbols.iter() {
                if let Some(symbol) = state.symbols.get(scoped_name) {
                    let symbol_name = state.resolve(symbol.name);

                    // Exact match in same file
                    if symbol_name == name {
                        interventions.push(Intervention {
                            severity: InterventionSeverity::Block,
                            message: format!(
                                "Symbol '{}' already defined in this file at line {}",
                                name, symbol.location.start_line
                            ),
                            existing_symbol: symbol.scoped_name,
                            existing_location: symbol.location.clone(),
                            similarity_score: 1.0,
                            recommendation: "Choose a different name or reuse the existing symbol"
                                .to_string(),
                        });
                        continue;
                    }

                    // Check for typos/similar names
                    let distance = strsim::levenshtein(name, symbol_name);
                    let max_len = name.len().max(symbol_name.len());

                    // If names differ by only 1-2 characters and are reasonably similar
                    if distance <= 2 && max_len > 3 {
                        let similarity = 1.0 - (distance as f32 / max_len as f32);
                        interventions.push(Intervention {
                            severity: InterventionSeverity::Warning,
                            message: format!(
                                "Very similar name '{}' exists at line {} - possible typo?",
                                symbol_name, symbol.location.start_line
                            ),
                            existing_symbol: symbol.scoped_name,
                            existing_location: symbol.location.clone(),
                            similarity_score: similarity,
                            recommendation: format!(
                                "Did you mean '{}'? Or choose a more distinct name to avoid confusion",
                                symbol_name
                            ),
                        });
                    }

                    // Check for case-only differences
                    if name.to_lowercase() == symbol_name.to_lowercase() && name != symbol_name {
                        interventions.push(Intervention {
                            severity: InterventionSeverity::Warning,
                            message: format!(
                                "Name differs only in case from '{}' at line {}",
                                symbol_name, symbol.location.start_line
                            ),
                            existing_symbol: symbol.scoped_name,
                            existing_location: symbol.location.clone(),
                            similarity_score: 0.95,
                            recommendation:
                                "Choose a name that differs in more than just capitalization"
                                    .to_string(),
                        });
                    }
                }
            }
        }

        // Check imports for conflicts
        if let Some(imports) = state.imports.get(&file_id) {
            for import in imports.iter() {
                // Check if importing a symbol with the same name
                if import.name == name {
                    interventions.push(Intervention {
                        severity: InterventionSeverity::Warning,
                        message: format!(
                            "Name '{}' conflicts with import from '{}' at line {}",
                            name, import.path, import.location.start_line
                        ),
                        existing_symbol: state.intern(&import.path),
                        existing_location: import.location.clone(),
                        similarity_score: 1.0,
                        recommendation: format!(
                            "Rename to avoid shadowing the imported '{}' or use a qualified path",
                            import.name
                        ),
                    });
                }
            }
        }

        // Sort by severity (Block > Warning > Info) then by score
        interventions.sort_by(|a, b| {
            let severity_order = |s: &InterventionSeverity| match s {
                InterventionSeverity::Block => 0,
                InterventionSeverity::Warning => 1,
                InterventionSeverity::Info => 2,
            };

            severity_order(&a.severity)
                .cmp(&severity_order(&b.severity))
                .then_with(|| {
                    b.similarity_score
                        .partial_cmp(&a.similarity_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        interventions
    }

    // ========================================================================
    // Internal Helper Methods
    // ========================================================================

    /// Parse a function signature string into components.
    ///
    /// Supports formats like:
    /// - "fn foo(x: i32) -> bool"
    /// - "async fn bar(s: &str)"
    /// - "unsafe fn baz() -> Result<(), Error>"
    fn parse_signature(sig: &str) -> Option<ParsedSignature> {
        let sig = sig.trim();

        // Check for async/unsafe modifiers
        let is_async = sig.contains("async");
        let is_unsafe = sig.contains("unsafe");

        // Find function name (between "fn" and "(")
        let fn_start = sig.find("fn ")?;
        let paren_start = sig.find('(')?;
        let name = sig[fn_start + 3..paren_start].trim().to_string();

        // Extract parameters (between "(" and ")")
        let paren_end = sig.find(')')?;
        let params_str = &sig[paren_start + 1..paren_end];
        let params: Vec<String> = if params_str.trim().is_empty() {
            Vec::new()
        } else {
            params_str
                .split(',')
                .map(|p| {
                    // Extract type from "name: type" or just "type"
                    p.trim()
                        .split(':')
                        .last()
                        .unwrap_or(p.trim())
                        .trim()
                        .to_string()
                })
                .collect()
        };

        // Extract return type (after "->")
        let return_type = if let Some(arrow_pos) = sig.find("->") {
            Some(sig[arrow_pos + 2..].trim().to_string())
        } else {
            None
        };

        Some(ParsedSignature {
            name,
            params,
            return_type,
            is_async,
            is_unsafe,
        })
    }

    /// Calculate similarity between a parsed signature and an existing symbol.
    ///
    /// Scoring factors:
    /// - Name similarity (40% weight): Levenshtein distance
    /// - Parameter count match (20% weight): Exact match or close
    /// - Parameter types match (25% weight): Number of matching types
    /// - Return type match (15% weight): Exact match or compatible
    fn calculate_signature_similarity(
        parsed: &ParsedSignature,
        symbol: &SymbolDef,
        sig: &Signature,
        state: &OciState,
    ) -> f32 {
        let mut total_score = 0.0;

        // Name similarity (40% weight)
        let symbol_name = state.resolve(symbol.name);
        let name_distance = strsim::levenshtein(&parsed.name, symbol_name);
        let max_name_len = parsed.name.len().max(symbol_name.len());
        let name_similarity = if max_name_len > 0 {
            1.0 - (name_distance as f32 / max_name_len as f32)
        } else {
            1.0
        };
        total_score += name_similarity * 0.4;

        // Parameter count match (20% weight)
        let param_count_diff = (parsed.params.len() as i32 - sig.params.len() as i32).abs();
        let param_count_similarity = if param_count_diff == 0 {
            1.0
        } else if param_count_diff == 1 {
            0.7
        } else {
            0.3 / (param_count_diff as f32)
        };
        total_score += param_count_similarity * 0.2;

        // Parameter types match (25% weight)
        let min_params = parsed.params.len().min(sig.params.len());
        if min_params > 0 {
            let mut matching_params = 0;
            for (i, parsed_param) in parsed.params.iter().enumerate().take(min_params) {
                if let Some(existing_param) = sig.params.get(i) {
                    // Normalize types for comparison (remove whitespace, &, etc.)
                    let p1 = Self::normalize_type(parsed_param);
                    let p2 = Self::normalize_type(existing_param);

                    if p1 == p2 || Self::types_compatible(&p1, &p2) {
                        matching_params += 1;
                    }
                }
            }
            let param_type_similarity =
                matching_params as f32 / parsed.params.len().max(sig.params.len()) as f32;
            total_score += param_type_similarity * 0.25;
        }

        // Return type match (15% weight)
        let return_similarity = match (&parsed.return_type, &sig.return_type) {
            (Some(p_ret), Some(s_ret)) => {
                let p_ret_norm = Self::normalize_type(p_ret);
                let s_ret_norm = Self::normalize_type(s_ret);
                if p_ret_norm == s_ret_norm || Self::types_compatible(&p_ret_norm, &s_ret_norm) {
                    1.0
                } else {
                    0.3
                }
            }
            (None, None) => 1.0, // Both return ()
            _ => 0.5,            // One returns something, the other doesn't
        };
        total_score += return_similarity * 0.15;

        total_score
    }

    /// Normalize a type string for comparison.
    ///
    /// Removes whitespace, reference markers, and normalizes common patterns.
    fn normalize_type(ty: &str) -> String {
        ty.trim()
            .replace(" ", "")
            .replace("&mut", "&")
            .replace("&", "")
            .to_lowercase()
    }

    /// Check if two types are compatible (e.g., String and &str).
    fn types_compatible(ty1: &str, ty2: &str) -> bool {
        // Handle common compatible types
        let compatible_pairs = [("string", "str"), ("vec", "slice"), ("&str", "string")];

        for (t1, t2) in &compatible_pairs {
            if (ty1.contains(t1) && ty2.contains(t2)) || (ty1.contains(t2) && ty2.contains(t1)) {
                return true;
            }
        }

        // Check if one is a reference to the other
        ty1.contains(ty2) || ty2.contains(ty1)
    }
}

impl Default for InterventionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_signature_simple() {
        let sig = "fn foo(x: i32) -> bool";
        let parsed = InterventionEngine::parse_signature(sig).unwrap();
        assert_eq!(parsed.name, "foo");
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.params[0], "i32");
        assert_eq!(parsed.return_type, Some("bool".to_string()));
        assert!(!parsed.is_async);
        assert!(!parsed.is_unsafe);
    }

    #[test]
    fn test_parse_signature_async() {
        let sig = "async fn bar(s: &str)";
        let parsed = InterventionEngine::parse_signature(sig).unwrap();
        assert_eq!(parsed.name, "bar");
        assert!(parsed.is_async);
    }

    #[test]
    fn test_parse_signature_multiple_params() {
        let sig = "fn baz(a: i32, b: String, c: &str) -> Result<(), Error>";
        let parsed = InterventionEngine::parse_signature(sig).unwrap();
        assert_eq!(parsed.params.len(), 3);
        assert_eq!(parsed.params[0], "i32");
        assert_eq!(parsed.params[1], "String");
        assert_eq!(parsed.params[2], "&str");
    }

    #[test]
    fn test_normalize_type() {
        assert_eq!(InterventionEngine::normalize_type("&str"), "str");
        assert_eq!(InterventionEngine::normalize_type("&mut String"), "string");
        assert_eq!(InterventionEngine::normalize_type("Vec<i32>"), "vec<i32>");
    }

    #[test]
    fn test_types_compatible() {
        assert!(InterventionEngine::types_compatible("string", "str"));
        assert!(InterventionEngine::types_compatible("str", "string"));
        assert!(InterventionEngine::types_compatible("vec<i32>", "slice"));
    }

    #[test]
    fn test_detect_duplication_empty_state() {
        let state = OciState::new(PathBuf::from("/test"));
        let matches = InterventionEngine::detect_duplication(&state, "fn test() -> bool");
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_suggest_alternatives_empty_state() {
        let state = OciState::new(PathBuf::from("/test"));
        let interventions = InterventionEngine::suggest_alternatives(&state, "test_function");
        assert_eq!(interventions.len(), 0);
    }

    #[test]
    fn test_check_naming_conflicts_empty_state() {
        let state = OciState::new(PathBuf::from("/test"));
        let interventions = InterventionEngine::check_naming_conflicts(
            &state,
            "test_function",
            Path::new("/test/file.rs"),
        );
        assert_eq!(interventions.len(), 0);
    }
}
