//! BM25 (Okapi) text search index.
//!
//! Optimized for code search with field-weighted scoring:
//! - Path tokens (file/module names)
//! - Identifiers (function/struct/variable names)
//! - Doc comments
//! - String literals
//! - General code tokens

use crate::types::InternedString;
use std::collections::HashMap;
use std::path::Path;

/// Field types for weighted BM25 scoring.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Field {
    Path,
    Ident,
    Doc,
    StringLit,
    Code,
}

impl Field {
    fn index(self) -> usize {
        match self {
            Field::Path => 0,
            Field::Ident => 1,
            Field::Doc => 2,
            Field::StringLit => 3,
            Field::Code => 4,
        }
    }
}

/// Weights for each field in BM25 scoring.
#[derive(Clone, Debug)]
pub struct FieldWeights {
    pub path: f32,
    pub ident: f32,
    pub doc: f32,
    pub string_lit: f32,
    pub code: f32,
}

impl Default for FieldWeights {
    fn default() -> Self {
        Self {
            path: 2.0,       // File paths very important for code search
            ident: 1.8,      // Identifiers highly relevant
            doc: 1.4,        // Doc comments helpful
            string_lit: 1.1, // String literals sometimes useful
            code: 1.0,       // General code baseline
        }
    }
}

/// BM25 parameters.
#[derive(Copy, Clone, Debug)]
pub struct Bm25Params {
    /// Term frequency saturation parameter (typically 1.2-2.0).
    pub k1: f32,
    /// Length normalization parameter (typically 0.75).
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// A posting entry for a term.
#[derive(Clone, Debug)]
struct Posting {
    doc_id: u32,
    tf_by_field: [u32; 5],
}

/// Document statistics.
#[derive(Clone, Debug)]
struct DocStats {
    symbol: InternedString,
    len_by_field: [u32; 5],
    /// Original text for snippet extraction.
    text: String,
}

/// BM25 search index.
#[derive(Default, Clone, Debug)]
pub struct Bm25Index {
    /// Inverted index: term -> postings list.
    inv: HashMap<String, Vec<Posting>>,
    /// Document statistics.
    docs: Vec<DocStats>,
    /// Average document length per field.
    avg_len_by_field: [f32; 5],
    /// Document frequency per term.
    df: HashMap<String, u32>,
    /// Symbol to doc_id mapping.
    symbol_to_doc: HashMap<InternedString, u32>,
}

/// Search result from BM25.
#[derive(Debug, Clone)]
pub struct Bm25SearchResult {
    pub symbol: InternedString,
    pub score: f32,
    pub doc_id: u32,
}

impl Bm25Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a document (symbol) to the index.
    pub fn add_document(
        &mut self,
        symbol: InternedString,
        path_tokens: impl IntoIterator<Item = impl AsRef<str>>,
        ident_tokens: impl IntoIterator<Item = impl AsRef<str>>,
        doc_tokens: impl IntoIterator<Item = impl AsRef<str>>,
        string_tokens: impl IntoIterator<Item = impl AsRef<str>>,
        code_text: &str,
    ) {
        let doc_id = self.docs.len() as u32;
        let mut lens = [0u32; 5];

        // Helper to add tokens
        let mut add = |field: Field, token: &str| {
            if token.is_empty() {
                return;
            }
            let term = token.to_ascii_lowercase();
            let postings = self.inv.entry(term).or_default();

            match postings.last_mut() {
                Some(last) if last.doc_id == doc_id => {
                    last.tf_by_field[field.index()] += 1;
                }
                _ => {
                    let mut tf = [0u32; 5];
                    tf[field.index()] = 1;
                    postings.push(Posting {
                        doc_id,
                        tf_by_field: tf,
                    });
                }
            }
            lens[field.index()] += 1;
        };

        // Add tokens from each field
        for t in path_tokens {
            add(Field::Path, t.as_ref());
        }
        for t in ident_tokens {
            add(Field::Ident, t.as_ref());
        }
        for t in doc_tokens {
            add(Field::Doc, t.as_ref());
        }
        for t in string_tokens {
            add(Field::StringLit, t.as_ref());
        }

        // Tokenize and add code text
        for t in tokenize(code_text) {
            add(Field::Code, t);
        }

        self.symbol_to_doc.insert(symbol, doc_id);
        self.docs.push(DocStats {
            symbol,
            len_by_field: lens,
            text: code_text.to_string(),
        });
    }

    /// Finalize the index (compute statistics).
    pub fn finalize(&mut self) {
        let n_docs = self.docs.len().max(1) as f32;

        // Compute average lengths per field
        let mut sum = [0u64; 5];
        for doc in &self.docs {
            for (s, &len) in sum.iter_mut().zip(doc.len_by_field.iter()) {
                *s += len as u64;
            }
        }
        for (avg, &s) in self.avg_len_by_field.iter_mut().zip(sum.iter()) {
            *avg = s as f32 / n_docs;
        }

        // Compute document frequencies
        self.df.clear();
        for (term, postings) in &self.inv {
            self.df.insert(term.clone(), postings.len() as u32);
        }
    }

    /// Search the index.
    pub fn search(
        &self,
        query: &str,
        weights: &FieldWeights,
        params: Bm25Params,
        top_k: usize,
    ) -> Vec<Bm25SearchResult> {
        let mut scores: HashMap<u32, f32> = HashMap::new();
        let n_docs = self.docs.len().max(1) as f32;
        let field_weights = [
            weights.path,
            weights.ident,
            weights.doc,
            weights.string_lit,
            weights.code,
        ];

        // Score each query term
        for term in tokenize(query) {
            let term_lower = term.to_ascii_lowercase();
            let Some(postings) = self.inv.get(&term_lower) else {
                continue;
            };

            let df = *self.df.get(&term_lower).unwrap_or(&1) as f32;
            // BM25 IDF: ln((N - df + 0.5) / (df + 0.5) + 1)
            let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();

            for posting in postings {
                let doc = &self.docs[posting.doc_id as usize];

                // Compute weighted term frequency
                let mut tf_weighted = 0.0f32;
                for (i, &weight) in field_weights.iter().enumerate() {
                    if posting.tf_by_field[i] > 0 {
                        tf_weighted += weight * posting.tf_by_field[i] as f32;
                    }
                }

                // Compute blended document length
                let mut len = 0.0f32;
                let mut avg_len = 0.0f32;
                for (i, &weight) in field_weights.iter().enumerate() {
                    len += weight * doc.len_by_field[i] as f32;
                    avg_len += weight * self.avg_len_by_field[i];
                }

                // BM25 scoring
                let norm = 1.0 - params.b + params.b * (len / avg_len.max(1e-6));
                let denom = tf_weighted + params.k1 * norm;
                let score = idf * (tf_weighted * (params.k1 + 1.0)) / denom.max(1e-6);

                *scores.entry(posting.doc_id).or_default() += score;
            }
        }

        // Sort by score and return top-k
        let mut results: Vec<_> = scores
            .into_iter()
            .map(|(doc_id, score)| Bm25SearchResult {
                symbol: self.docs[doc_id as usize].symbol,
                score,
                doc_id,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }

    /// Get the number of indexed documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Get document text for a symbol.
    pub fn get_text(&self, symbol: InternedString) -> Option<&str> {
        self.symbol_to_doc
            .get(&symbol)
            .map(|&doc_id| self.docs[doc_id as usize].text.as_str())
    }
}

/// Simple tokenizer for code.
///
/// Splits on non-word characters and handles camelCase/snake_case.
pub fn tokenize(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .flat_map(split_identifier)
}

/// Split an identifier into sub-tokens (camelCase, snake_case).
fn split_identifier(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;

    for i in 1..bytes.len() {
        let prev = bytes[i - 1] as char;
        let curr = bytes[i] as char;

        // Split on: underscore, lowercase->uppercase transition
        let boundary = curr == '_' || (prev.is_ascii_lowercase() && curr.is_ascii_uppercase());

        if boundary {
            if start < i && bytes[start] != b'_' {
                tokens.push(&s[start..i]);
            }
            start = if curr == '_' { i + 1 } else { i };
        }
    }

    if start < s.len() && bytes[start] != b'_' {
        tokens.push(&s[start..]);
    }

    // Also add the full identifier as a token
    if tokens.len() > 1 {
        tokens.push(s);
    }

    tokens
}

/// Extract tokens from a file path.
pub fn path_tokens(path: &Path) -> Vec<String> {
    path.iter()
        .filter_map(|c| c.to_str())
        .flat_map(|s| {
            // Remove extension
            let base = s.rsplit_once('.').map(|(a, _)| a).unwrap_or(s);
            tokenize(base).map(|t| t.to_lowercase()).collect::<Vec<_>>()
        })
        .collect()
}

/// Extract identifiers from Rust code (simple regex-free approach).
pub fn extract_identifiers(code: &str) -> Vec<&str> {
    // Find potential identifiers: sequences of word chars
    tokenize(code)
        .filter(|s| {
            // Filter out pure numbers and very short tokens
            s.len() >= 2 && !s.chars().all(|c| c.is_ascii_digit())
        })
        .collect()
}

/// Extract doc comments from Rust code.
pub fn extract_doc_comments(code: &str) -> Vec<String> {
    let mut docs = Vec::new();

    // Line doc comments (///)
    for line in code.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("///") {
            let content = trimmed
                .trim_start_matches('/')
                .trim_start_matches('/')
                .trim_start_matches('/')
                .trim();
            if !content.is_empty() {
                docs.push(content.to_string());
            }
        }
    }

    // Block doc comments (/** ... */)
    let mut rest = code;
    while let Some(start) = rest.find("/**") {
        let after = &rest[start + 3..];
        if let Some(end) = after.find("*/") {
            let content = &after[..end];
            for line in content.lines() {
                let trimmed = line.trim().trim_start_matches('*').trim();
                if !trimmed.is_empty() {
                    docs.push(trimmed.to_string());
                }
            }
            rest = &after[end + 2..];
        } else {
            break;
        }
    }

    docs
}

/// Extract string literals from Rust code (naive).
pub fn extract_string_literals(code: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            i += 1;
            // Find closing quote (ignoring escapes for simplicity)
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1; // Skip escaped char
                }
                i += 1;
            }
            if i < bytes.len() {
                let content = &code[start..i];
                if !content.is_empty() && content.len() < 200 {
                    strings.push(content.to_string());
                }
            }
        }
        i += 1;
    }

    strings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens: Vec<_> = tokenize("hello_world").collect();
        assert!(tokens.contains(&"hello"));
        assert!(tokens.contains(&"world"));

        let tokens: Vec<_> = tokenize("HelloWorld").collect();
        assert!(tokens.contains(&"Hello"));
        assert!(tokens.contains(&"World"));
    }

    #[test]
    fn test_bm25_basic() {
        use lasso::ThreadedRodeo;

        let mut index = Bm25Index::new();

        let interner = ThreadedRodeo::default();
        let sym1 = InternedString::from(interner.get_or_intern("add_numbers"));
        let sym2 = InternedString::from(interner.get_or_intern("subtract_numbers"));

        index.add_document(
            sym1,
            vec!["utils"],
            vec!["add", "numbers"],
            vec!["adds", "two", "integers"],
            Vec::<&str>::new(),
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        );

        index.add_document(
            sym2,
            vec!["math"],
            vec!["subtract", "numbers"],
            vec!["subtracts", "integers"],
            Vec::<&str>::new(),
            "fn subtract(a: i32, b: i32) -> i32 { a - b }",
        );

        index.finalize();

        let results = index.search(
            "add integers",
            &FieldWeights::default(),
            Bm25Params::default(),
            10,
        );
        assert!(!results.is_empty());
        assert_eq!(results[0].symbol, sym1);
    }

    #[test]
    fn test_extract_doc_comments() {
        let code = r#"
/// This is a doc comment
/// with multiple lines
fn foo() {}

/**
 * Block doc comment
 * also with lines
 */
fn bar() {}
"#;
        let docs = extract_doc_comments(code);
        assert!(docs.iter().any(|d| d.contains("doc comment")));
        assert!(docs.iter().any(|d| d.contains("Block")));
    }

    #[test]
    fn test_extract_string_literals() {
        let code = r#"
fn foo() {
    let s = "hello world";
    let t = "goodbye";
}
"#;
        let strings = extract_string_literals(code);
        assert!(strings.iter().any(|s| s == "hello world"));
        assert!(strings.iter().any(|s| s == "goodbye"));
    }
}
