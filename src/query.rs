use crate::cache::{bm25_path, state_path};
use crate::search::{Bm25Index, Bm25Params, FieldWeights};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDoc {
    pub symbol: String,
    pub file: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub preview: String,
    pub indexed_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchState {
    pub docs: Vec<SearchDoc>,
}

#[derive(Debug, Clone)]
pub struct SearchIndex {
    pub root: PathBuf,
    pub docs: Vec<SearchDoc>,
    pub bm25: Bm25Index,
}

#[derive(Debug, Clone, Default)]
pub struct QueryFilters {
    pub include_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub include_exts: Vec<String>,
    pub exclude_exts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub doc_id: u32,
    pub symbol: String,
    pub file: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub score: f32,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResponse {
    pub root: String,
    pub query: String,
    pub top_k: usize,
    pub results: Vec<QueryResult>,
}

pub fn load_search_state(root: &Path) -> Result<Option<SearchState>> {
    let path = state_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read(&path).with_context(|| format!("Failed to read state: {}", path.display()))?;
    let state: SearchState = bincode::deserialize(&data)
        .with_context(|| format!("Failed to decode state: {}", path.display()))?;
    Ok(Some(state))
}

pub fn save_search_state(root: &Path, state: &SearchState) -> Result<()> {
    crate::cache::ensure_cache_dir(root)?;
    let path = state_path(root);
    let data = bincode::serialize(state)?;
    fs::write(&path, data).with_context(|| format!("Failed to write state: {}", path.display()))?;
    Ok(())
}

pub fn load_bm25(root: &Path) -> Result<Option<Bm25Index>> {
    let path = bm25_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read(&path).with_context(|| format!("Failed to read BM25: {}", path.display()))?;
    let index: Bm25Index = bincode::deserialize(&data)
        .with_context(|| format!("Failed to decode BM25: {}", path.display()))?;
    Ok(Some(index))
}

pub fn save_bm25(root: &Path, index: &Bm25Index) -> Result<()> {
    crate::cache::ensure_cache_dir(root)?;
    let path = bm25_path(root);
    let data = bincode::serialize(index)?;
    fs::write(&path, data).with_context(|| format!("Failed to write BM25: {}", path.display()))?;
    Ok(())
}

pub fn load_search_index(root: &Path) -> Result<Option<SearchIndex>> {
    let Some(state) = load_search_state(root)? else {
        return Ok(None);
    };
    let Some(bm25) = load_bm25(root)? else {
        return Ok(None);
    };
    Ok(Some(SearchIndex {
        root: root.to_path_buf(),
        docs: state.docs,
        bm25,
    }))
}

pub fn parse_query_filters(query: &str, extra_filters: &[String]) -> (String, QueryFilters) {
    let mut filters = QueryFilters::default();
    let mut terms = Vec::new();

    let mut handle_token = |token: &str| {
        let token = token.trim();
        if token.is_empty() {
            return;
        }
        let (negated, rest) = token
            .strip_prefix('-')
            .map(|t| (true, t))
            .unwrap_or((false, token));
        if let Some(path) = rest.strip_prefix("path:") {
            if negated {
                filters.exclude_paths.push(path.to_string());
            } else {
                filters.include_paths.push(path.to_string());
            }
            return;
        }
        if let Some(ext) = rest.strip_prefix("ext:") {
            let ext = ext.trim_start_matches('.');
            if negated {
                filters.exclude_exts.push(ext.to_lowercase());
            } else {
                filters.include_exts.push(ext.to_lowercase());
            }
            return;
        }
        terms.push(token.to_string());
    };

    for token in query.split_whitespace() {
        handle_token(token);
    }

    for filter in extra_filters {
        handle_token(filter);
    }

    (terms.join(" "), filters)
}

pub fn execute_query(
    index: &SearchIndex,
    query: &str,
    top_k: usize,
    filters: &QueryFilters,
) -> QueryResponse {
    let search_k = top_k.saturating_mul(5).max(top_k).min(1000);
    let results = index.bm25.search(
        query,
        &FieldWeights::default(),
        Bm25Params::default(),
        search_k,
    );

    let mut filtered = Vec::new();

    for result in results {
        let doc_id = result.doc_id as usize;
        if doc_id >= index.docs.len() {
            continue;
        }
        let doc = &index.docs[doc_id];
        if !matches_filters(doc, filters) {
            continue;
        }
        filtered.push(QueryResult {
            doc_id: result.doc_id,
            symbol: doc.symbol.clone(),
            file: doc.file.clone(),
            start_byte: doc.start_byte,
            end_byte: doc.end_byte,
            start_line: doc.start_line + 1,
            end_line: doc.end_line + 1,
            start_col: doc.start_col + 1,
            end_col: doc.end_col + 1,
            score: result.score,
            preview: doc.preview.clone(),
        });
    }

    filtered.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.start_byte.cmp(&b.start_byte))
    });

    if filtered.len() > top_k {
        filtered.truncate(top_k);
    }

    QueryResponse {
        root: index.root.display().to_string(),
        query: query.to_string(),
        top_k,
        results: filtered,
    }
}

fn matches_filters(doc: &SearchDoc, filters: &QueryFilters) -> bool {
    if !filters.include_paths.is_empty() {
        let mut matched = false;
        for pat in &filters.include_paths {
            if doc.file.contains(pat) {
                matched = true;
                break;
            }
        }
        if !matched {
            return false;
        }
    }

    for pat in &filters.exclude_paths {
        if doc.file.contains(pat) {
            return false;
        }
    }

    let ext = Path::new(&doc.file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !filters.include_exts.is_empty() && !filters.include_exts.contains(&ext) {
        return false;
    }

    if !filters.exclude_exts.is_empty() && filters.exclude_exts.contains(&ext) {
        return false;
    }

    true
}

pub fn rebuild_bm25(docs: &[SearchDoc]) -> Bm25Index {
    let mut index = Bm25Index::new();

    for (doc_id, doc) in docs.iter().enumerate() {
        let doc_id = doc_id as u32;
        let path_tokens = crate::search::path_tokens(Path::new(&doc.file));
        let ident_tokens = crate::search::tokenize(&doc.symbol);
        let code_text = doc.indexed_text.as_str();

        index.add_document(
            doc_id,
            path_tokens,
            ident_tokens,
            std::iter::empty::<&str>(),
            std::iter::empty::<&str>(),
            code_text,
        );
    }

    index.finalize();
    index
}

pub fn prune_docs_for_files(docs: &[SearchDoc], files: &HashSet<String>) -> Vec<SearchDoc> {
    docs.iter()
        .filter(|doc| !files.contains(&doc.file))
        .cloned()
        .collect()
}
