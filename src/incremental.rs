//! Incremental indexing engine.
//!
//! Handles efficient updates when files change, avoiding full re-indexing.

use crate::cache::{FileFingerprint, IndexManifest, load_manifest};
use crate::parsing;
use crate::query::{SearchDoc, SearchState, rebuild_bm25, save_bm25, save_search_state};
use crate::state::OciState;
use crate::topology::TopologyBuilder;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tree_sitter::Parser;

/// Incremental indexer that updates the state when files change.
pub struct IncrementalIndexer {
    topology_builder: TopologyBuilder,
}

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub force: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub no_default_excludes: bool,
    pub include_hidden: bool,
    pub include_large: bool,
    pub max_file_size: u64,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            force: false,
            include: Vec::new(),
            exclude: Vec::new(),
            no_default_excludes: false,
            include_hidden: false,
            include_large: false,
            max_file_size: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndexReport {
    pub total_files: usize,
    pub parsed_files: usize,
    pub skipped_files: usize,
    pub removed_files: usize,
    pub docs_indexed: usize,
}

#[derive(Debug)]
struct ParsedFile {
    symbols: Vec<crate::types::SymbolDef>,
    calls: Vec<crate::types::CallEdge>,
    imports: Vec<crate::types::ImportInfo>,
    docs: Vec<SearchDoc>,
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
        state.reset();

        // Discover files
        let discovery = crate::discovery::FileDiscovery::new();
        let files = discovery.discover(root)?;
        let files: Vec<PathBuf> = files
            .into_iter()
            .filter(|path| parsing::parser_for_file(path).is_some())
            .collect();

        tracing::info!("Discovered {} files", files.len());

        // Index each file
        for file in &files {
            if let Err(e) = self.index_file(state, file, root).await {
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

    pub async fn index(
        &self,
        state: &OciState,
        root: &Path,
        options: &IndexOptions,
    ) -> Result<IndexReport> {
        tracing::info!("Starting incremental index of {}", root.display());

        if options.force {
            crate::cache::clear_cache(root)?;
            state.reset();
        }

        let discovery = build_discovery(options);
        let files = discovery.discover(root)?;
        let files: Vec<PathBuf> = files
            .into_iter()
            .filter(|path| parsing::parser_for_file(path).is_some())
            .collect();

        let mut report = IndexReport {
            total_files: files.len(),
            ..Default::default()
        };

        let (mut manifest, reset_state) = load_or_init_manifest(root, options.force)?;
        if reset_state && !options.force {
            state.reset();
        }
        let mut docs = load_or_init_docs(root, &manifest, options.force)?;

        let mut seen = HashSet::new();
        let mut changed_files = HashSet::new();
        let mut removed_files = HashSet::new();

        for file in &files {
            let rel = relative_path(root, file)?;
            seen.insert(rel.clone());

            let fingerprint = fingerprint(file)?;
            match manifest.files.get(&rel) {
                Some(prev) if *prev == fingerprint => {
                    report.skipped_files += 1;
                }
                _ => {
                    changed_files.insert(rel.clone());
                    manifest.files.insert(rel, fingerprint);
                }
            }
        }

        for existing in manifest.files.keys().cloned().collect::<Vec<_>>() {
            if !seen.contains(&existing) {
                removed_files.insert(existing);
            }
        }

        // Remove deleted entries from manifest
        for removed in &removed_files {
            manifest.files.remove(removed);
        }

        report.removed_files = removed_files.len();

        let mut drop_docs_for = HashSet::new();
        drop_docs_for.extend(changed_files.iter().cloned());
        drop_docs_for.extend(removed_files.iter().cloned());

        if !drop_docs_for.is_empty() {
            docs = crate::query::prune_docs_for_files(&docs, &drop_docs_for);
        }

        for rel in &removed_files {
            let path = root.join(rel);
            self.remove_file(state, &path);
        }

        for rel in &changed_files {
            let path = root.join(rel);
            match self.update_file(state, &path, root).await {
                Ok(file_docs) => {
                    report.parsed_files += 1;
                    report.docs_indexed += file_docs.len();
                    docs.extend(file_docs);
                }
                Err(e) => {
                    tracing::warn!("Failed to index {}: {}", path.display(), e);
                }
            }
        }

        let bm25 = rebuild_bm25(&docs);
        {
            let mut guard = state.bm25_index.write();
            *guard = Some(bm25.clone());
        }

        save_search_state(root, &SearchState { docs: docs.clone() })?;
        save_bm25(root, &bm25)?;
        crate::cache::save_manifest(root, &manifest)?;
        *state.last_indexed.write() = Some(std::time::Instant::now());

        Ok(report)
    }

    async fn parse_file(&self, state: &OciState, path: &Path, root: &Path) -> Result<ParsedFile> {
        let lang_parser = match parsing::parser_for_file(path) {
            Some(p) => p,
            None => {
                return Ok(ParsedFile {
                    symbols: Vec::new(),
                    calls: Vec::new(),
                    imports: Vec::new(),
                    docs: Vec::new(),
                });
            }
        };

        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut parser = Parser::new();
        parser
            .set_language(&lang_parser.language())
            .context("Failed to set parser language")?;

        let tree = parser
            .parse(&contents, None)
            .context("Failed to parse file")?;

        let symbols = lang_parser.extract_symbols(&tree, &contents, path, &state.interner)?;
        let calls = lang_parser.extract_calls(&tree, &contents, path, &state.interner)?;
        let imports = lang_parser.extract_imports(&tree, &contents, path)?;

        let docs = build_search_docs(path, root, &contents, &symbols, state)?;

        Ok(ParsedFile {
            symbols,
            calls,
            imports,
            docs,
        })
    }

    fn apply_parsed(&self, state: &OciState, path: &Path, parsed: &ParsedFile) {
        if parsed.symbols.is_empty() && parsed.calls.is_empty() && parsed.imports.is_empty() {
            return;
        }

        let file_id = state.get_or_create_file_id(&path.to_path_buf());

        let mut file_symbol_names = Vec::with_capacity(parsed.symbols.len());
        for symbol in parsed.symbols.iter().cloned() {
            file_symbol_names.push(symbol.scoped_name);
            state.add_symbol(symbol);
        }
        if !file_symbol_names.is_empty() {
            state.file_symbols.insert(file_id, file_symbol_names);
        }

        for call in parsed.calls.iter().cloned() {
            state.add_call_edge(call);
        }

        if !parsed.imports.is_empty() {
            state.imports.insert(file_id, parsed.imports.clone());
        }
    }

    /// Index a single file.
    pub async fn index_file(
        &self,
        state: &OciState,
        path: &Path,
        root: &Path,
    ) -> Result<Vec<SearchDoc>> {
        let parsed = self.parse_file(state, path, root).await?;
        self.apply_parsed(state, path, &parsed);
        Ok(parsed.docs)
    }

    /// Update a single file (clear old data, re-index).
    pub async fn update_file(
        &self,
        state: &OciState,
        path: &Path,
        root: &Path,
    ) -> Result<Vec<SearchDoc>> {
        // Clear existing data for this file
        state.clear_file(&path.to_path_buf());

        // Re-index
        self.index_file(state, path, root).await
    }

    /// Remove a file from the index.
    pub fn remove_file(&self, state: &OciState, path: &Path) {
        state.clear_file(&path.to_path_buf());
        self.topology_builder.remove_file(state, path).ok();
    }
}

impl Default for IncrementalIndexer {
    fn default() -> Self {
        Self::new()
    }
}

fn build_discovery(options: &IndexOptions) -> crate::discovery::FileDiscovery {
    let mut discovery =
        crate::discovery::FileDiscovery::new().with_max_file_size(options.max_file_size);
    if options.no_default_excludes {
        discovery = discovery.without_default_excludes();
    }
    if options.include_hidden {
        discovery = discovery.include_hidden();
    }
    if options.include_large {
        discovery = discovery.include_large();
    }
    for pattern in &options.include {
        discovery = discovery.with_include(pattern);
    }
    for pattern in &options.exclude {
        discovery = discovery.with_exclude(pattern);
    }
    discovery
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("Path {} is outside root {}", path.display(), root.display()))?;
    Ok(rel.to_string_lossy().to_string())
}

fn fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("Failed to stat {}", path.display()))?;
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    let duration = modified.duration_since(UNIX_EPOCH).unwrap_or_default();
    Ok(FileFingerprint {
        mtime_ms: duration.as_millis() as u64,
        size_bytes: metadata.len(),
    })
}

fn load_or_init_manifest(root: &Path, force: bool) -> Result<(IndexManifest, bool)> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let root_path = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();

    if !force {
        if let Some(manifest) = load_manifest(root)? {
            if manifest.tool_version == version
                && manifest.root.as_deref() == Some(root_path.as_str())
            {
                return Ok((manifest, false));
            }
        }
    }

    crate::cache::clear_cache(root)?;
    Ok((
        IndexManifest {
            tool_version: version,
            root: Some(root_path),
            files: HashMap::new(),
        },
        true,
    ))
}

fn load_or_init_docs(
    root: &Path,
    _manifest: &IndexManifest,
    force: bool,
) -> Result<Vec<SearchDoc>> {
    if force {
        return Ok(Vec::new());
    }
    Ok(crate::query::load_search_state(root)?
        .map(|s| s.docs)
        .unwrap_or_default())
}

fn build_search_docs(
    path: &Path,
    root: &Path,
    contents: &str,
    symbols: &[crate::types::SymbolDef],
    state: &OciState,
) -> Result<Vec<SearchDoc>> {
    let rel_path = relative_path(root, path)?;
    let mut docs = Vec::with_capacity(symbols.len());

    for symbol in symbols {
        let start = symbol.location.start_byte;
        let end = symbol.location.end_byte;
        let span = slice_utf8(contents, start, end);
        if span.is_empty() {
            continue;
        }
        let preview = make_preview(span);
        let doc_comment = symbol.doc_comment.as_deref().unwrap_or("");
        let combined = if doc_comment.is_empty() {
            span.to_string()
        } else {
            format!("{} {}", doc_comment, span)
        };
        let indexed_text = truncate_to_len(&combined, 4000);

        docs.push(SearchDoc {
            symbol: state.resolve(symbol.scoped_name).to_string(),
            file: rel_path.clone(),
            start_byte: symbol.location.start_byte,
            end_byte: symbol.location.end_byte,
            start_line: symbol.location.start_line,
            end_line: symbol.location.end_line,
            start_col: symbol.location.start_col,
            end_col: symbol.location.end_col,
            preview,
            indexed_text,
        });
    }

    Ok(docs)
}

fn slice_utf8(contents: &str, start: usize, end: usize) -> &str {
    let len = contents.len();
    let mut s = start.min(len);
    let mut e = end.min(len);

    while s > 0 && !contents.is_char_boundary(s) {
        s -= 1;
    }
    while e < len && !contents.is_char_boundary(e) {
        e += 1;
    }
    if s >= e {
        return "";
    }
    &contents[s..e]
}

fn make_preview(text: &str) -> String {
    let single = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_to_len(&single, 240)
}

fn truncate_to_len(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}
