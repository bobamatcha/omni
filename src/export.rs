//! Export utilities for downstream tools (e.g., Engram).

use crate::state::OciState;
use crate::types::{SymbolDef, TopologyNode};
use anyhow::Result;
use serde::Serialize;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct ExportStats {
    pub files: u32,
    pub symbols: u32,
    pub calls: u32,
}

#[derive(Debug, Serialize)]
pub struct ExportFile {
    pub path: String,
    pub relevance: f64,
}

#[derive(Debug, Serialize)]
pub struct ExportSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct EngramMemoryExport {
    pub content: String,
    pub metadata: EngramMetadata,
}

#[derive(Debug, Serialize)]
pub struct EngramMetadata {
    pub source: String,
    pub workspace: String,
    pub generated_at_unix: u64,
    pub stats: ExportStats,
    pub top_files: Vec<ExportFile>,
    pub top_symbols: Vec<ExportSymbol>,
}

pub fn export_engram_memory(
    state: &OciState,
    workspace: &PathBuf,
    max_files: usize,
    max_symbols: usize,
) -> Result<EngramMemoryExport> {
    let stats = state.stats();
    let top_files = collect_top_files(state, max_files);
    let top_symbols = collect_top_symbols(state, max_symbols);
    let generated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let content = format_engram_content(workspace, &stats, &top_files, &top_symbols);

    Ok(EngramMemoryExport {
        content,
        metadata: EngramMetadata {
            source: "omni-export".to_string(),
            workspace: workspace.display().to_string(),
            generated_at_unix,
            stats: ExportStats {
                files: stats.file_count,
                symbols: stats.symbol_count,
                calls: stats.call_edge_count,
            },
            top_files,
            top_symbols,
        },
    })
}

fn collect_top_files(state: &OciState, max_files: usize) -> Vec<ExportFile> {
    let graph = state.topology.read();
    let mut files = Vec::new();

    for entry in state.topology_metrics.iter() {
        let node_idx = *entry.key();
        let metrics = entry.value();
        if let Some(node) = graph.node_weight(node_idx) {
            if let TopologyNode::File { path, .. } = node {
                files.push(ExportFile {
                    path: path.display().to_string(),
                    relevance: metrics.relevance_score,
                });
            }
        }
    }

    files.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(Ordering::Equal)
    });
    files.truncate(max_files);
    files
}

fn collect_top_symbols(state: &OciState, max_symbols: usize) -> Vec<ExportSymbol> {
    let mut symbols: Vec<ExportSymbol> = state
        .symbols
        .iter()
        .map(|entry| to_export_symbol(state, entry.value()))
        .collect();

    symbols.sort_by(|a, b| a.name.cmp(&b.name));
    symbols.truncate(max_symbols);
    symbols
}

fn to_export_symbol(state: &OciState, symbol: &SymbolDef) -> ExportSymbol {
    ExportSymbol {
        name: state.resolve(symbol.scoped_name).to_string(),
        kind: symbol.kind.as_str().to_string(),
        file: symbol.location.file.display().to_string(),
        line: symbol.location.start_line,
    }
}

fn format_engram_content(
    workspace: &PathBuf,
    stats: &crate::state::IndexStats,
    top_files: &[ExportFile],
    top_symbols: &[ExportSymbol],
) -> String {
    let mut content = String::new();
    content.push_str("OMNI Context Summary\n");
    content.push_str(&format!("Workspace: {}\n", workspace.display()));
    content.push_str(&format!(
        "Indexed files: {}, symbols: {}, call edges: {}\n",
        stats.file_count, stats.symbol_count, stats.call_edge_count
    ));

    if !top_files.is_empty() {
        content.push_str("\nTop files by relevance:\n");
        for file in top_files {
            content.push_str(&format!(
                "- {} (score {:.4})\n",
                file.path, file.relevance
            ));
        }
    }

    if !top_symbols.is_empty() {
        content.push_str("\nSample symbols:\n");
        for sym in top_symbols {
            content.push_str(&format!(
                "- {} ({}) at {}:{}\n",
                sym.name, sym.kind, sym.file, sym.line
            ));
        }
    }

    content
}
