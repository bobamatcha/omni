//! MCP server implementation.
//!
//! Exposes OCI functionality via Model Context Protocol.

use crate::state::{create_state, SharedState};
use crate::incremental::IncrementalIndexer;
use crate::topology::TopologyBuilder;
use anyhow::Result;
use petgraph::visit::EdgeRef;
use rmcp::handler::server::{router::tool::ToolRouter, tool::Parameters};
use rmcp::model::{ErrorData as McpError, *};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub const SERVER_NAME: &str = "omni-index";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Server state wrapper with async access
pub struct OciServerState {
    pub oci_state: SharedState,
    pub indexer: IncrementalIndexer,
    pub topology: TopologyBuilder,
    pub workspace_root: PathBuf,
}

impl OciServerState {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            oci_state: create_state(workspace_root.clone()),
            indexer: IncrementalIndexer::new(),
            topology: TopologyBuilder::new(),
            workspace_root,
        }
    }
}

/// The MCP server handler that implements all tool methods.
#[derive(Clone)]
pub struct OciServer {
    state: Arc<RwLock<OciServerState>>,
    tool_router: ToolRouter<Self>,
}

impl OciServer {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            state: Arc::new(RwLock::new(OciServerState::new(workspace_root))),
            tool_router: Self::tool_router(),
        }
    }
}

// ============================================================================
// Tool Argument Types
// ============================================================================

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexRequest {
    #[schemars(description = "Operation: build, rebuild, status")]
    pub op: String,
    #[schemars(description = "Force full rebuild even if index exists")]
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolRequest {
    #[schemars(description = "Symbol name to search for")]
    pub name: String,
    #[schemars(description = "Whether to search by scoped name (e.g., 'crate::module::Foo')")]
    #[serde(default)]
    pub scoped: bool,
    #[schemars(description = "Maximum number of results")]
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CallGraphRequest {
    #[schemars(description = "Operation: callers, callees")]
    pub op: String,
    #[schemars(description = "Symbol name to find callers/callees for")]
    pub name: String,
    #[schemars(description = "Maximum depth to traverse (default: 1)")]
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnalysisRequest {
    #[schemars(description = "Analysis type: dead_code, coverage, churn, hotspots")]
    pub analysis: String,
    #[schemars(description = "Path to coverage JSON file (for coverage analysis)")]
    pub coverage_file: Option<String>,
    #[schemars(description = "Number of days to analyze (for churn analysis)")]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRequest {
    #[schemars(description = "Search query")]
    pub query: String,
    #[schemars(description = "Search type: semantic, bm25, hybrid")]
    #[serde(default = "default_search_type")]
    pub search_type: String,
    #[schemars(description = "Maximum number of results")]
    pub max_results: Option<usize>,
}

fn default_search_type() -> String {
    "hybrid".to_string()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContextRequest {
    #[schemars(description = "File path for context")]
    pub file: String,
    #[schemars(description = "Line number (1-indexed)")]
    pub line: u32,
    #[schemars(description = "Number of surrounding lines to include")]
    pub surrounding: Option<u32>,
    #[schemars(description = "Intent/goal for context (helps prioritize)")]
    pub intent: Option<String>,
    #[schemars(description = "Maximum tokens in response")]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InterventionRequest {
    #[schemars(description = "Check type: duplication, naming, alternatives")]
    pub check: String,
    #[schemars(description = "Proposed function signature (for duplication check)")]
    pub signature: Option<String>,
    #[schemars(description = "Proposed name (for naming/alternatives check)")]
    pub name: Option<String>,
    #[schemars(description = "File path context")]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TopologyRequest {
    #[schemars(description = "Operation: modules, imports, pagerank, dependencies")]
    pub op: String,
    #[schemars(description = "File or module path to query")]
    pub path: Option<String>,
    #[schemars(description = "Maximum results")]
    pub max_results: Option<usize>,
}

// ============================================================================
// Tool Implementations
// ============================================================================

#[tool_router]
impl OciServer {
    #[tool(description = "Build or rebuild the code index. Operations: build, rebuild, status")]
    async fn index(
        &self,
        Parameters(req): Parameters<IndexRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let oci = &state.oci_state;
        let root = state.workspace_root.clone();

        match req.op.as_str() {
            "build" | "rebuild" => {
                if req.force || req.op == "rebuild" {
                    drop(state);
                    let state = self.state.write().await;
                    match state.indexer.full_index(&state.oci_state, &root).await {
                        Ok(()) => {
                            let stats = state.oci_state.stats();
                            Ok(CallToolResult::success(vec![Content::text(format!(
                                "Index built successfully:\n- {} files\n- {} symbols\n- {} call edges\n- {} topology nodes",
                                stats.file_count, stats.symbol_count, stats.call_edge_count, stats.topology_node_count
                            ))]))
                        }
                        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                            "Index build failed: {}", e
                        ))])),
                    }
                } else {
                    let stats = oci.stats();
                    if stats.file_count > 0 {
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Index already exists with {} files. Use force=true to rebuild.",
                            stats.file_count
                        ))]))
                    } else {
                        drop(state);
                        let state = self.state.write().await;
                        match state.indexer.full_index(&state.oci_state, &root).await {
                            Ok(()) => {
                                let stats = state.oci_state.stats();
                                Ok(CallToolResult::success(vec![Content::text(format!(
                                    "Index built: {} files, {} symbols",
                                    stats.file_count, stats.symbol_count
                                ))]))
                            }
                            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
                        }
                    }
                }
            }
            "status" => {
                let stats = oci.stats();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Index Status:\n- Files: {}\n- Symbols: {}\n- Call edges: {}\n- Topology nodes: {}\n- Semantic index: {}\n- BM25 index: {}",
                    stats.file_count,
                    stats.symbol_count,
                    stats.call_edge_count,
                    stats.topology_node_count,
                    if stats.has_semantic_index { "ready" } else { "not built" },
                    if stats.has_bm25_index { "ready" } else { "not built" }
                ))]))
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown operation: {}. Valid: build, rebuild, status", req.op
            ))])),
        }
    }

    #[tool(description = "Find symbols by name. Returns definitions with locations and signatures.")]
    async fn find_symbol(
        &self,
        Parameters(req): Parameters<SymbolRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let oci = &state.oci_state;

        let max = req.max_results.unwrap_or(10);

        if req.scoped {
            // Search by scoped name
            let key = oci.interner.get(&req.name);
            if let Some(key) = key {
                if let Some(sym) = oci.get_symbol(key) {
                    let name = oci.resolve(sym.name);
                    let scoped = oci.resolve(sym.scoped_name);
                    let sig = sym.signature.as_ref().map(|s| format!("{} -> {}", s.params.join(", "), s.return_type.as_deref().unwrap_or("()"))).unwrap_or_default();

                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Found: {} ({})\n  Kind: {:?}\n  Location: {}:{}\n  Signature: {}\n  Visibility: {:?}",
                        scoped, name, sym.kind, sym.location.file.display(), sym.location.start_line, sig, sym.visibility
                    ))]));
                }
            }
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No symbol found with scoped name: {}", req.name
            ))]))
        } else {
            // Search by simple name
            let symbols = oci.find_by_name(&req.name);
            if symbols.is_empty() {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "No symbols found with name: {}", req.name
                ))]));
            }

            let mut output = format!("Found {} symbols:\n\n", symbols.len().min(max));
            for sym in symbols.iter().take(max) {
                let scoped = oci.resolve(sym.scoped_name);
                let sig = sym.signature.as_ref().map(|s| format!("({}) -> {}", s.params.join(", "), s.return_type.as_deref().unwrap_or("()"))).unwrap_or_default();
                output.push_str(&format!(
                    "- {} [{:?}]\n  {}:{}\n  {}\n\n",
                    scoped, sym.kind, sym.location.file.display(), sym.location.start_line, sig
                ));
            }

            Ok(CallToolResult::success(vec![Content::text(output)]))
        }
    }

    #[tool(description = "Query the call graph. Find callers or callees of a symbol.")]
    async fn call_graph(
        &self,
        Parameters(req): Parameters<CallGraphRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let oci = &state.oci_state;

        match req.op.as_str() {
            "callers" => {
                let callers = oci.find_callers(&req.name);
                if callers.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "No callers found for: {}", req.name
                    ))]));
                }

                let mut output = format!("Found {} call sites for '{}':\n\n", callers.len(), req.name);
                for call in &callers {
                    let caller_name = oci.resolve(call.caller);
                    output.push_str(&format!(
                        "- {} calls {} at {}:{}\n",
                        caller_name, call.callee_name, call.location.file.display(), call.location.start_line
                    ));
                }

                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "callees" => {
                // Find the symbol first
                let symbols = oci.find_by_name(&req.name);
                if symbols.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "No symbol found: {}", req.name
                    ))]));
                }

                let mut output = String::new();
                for sym in &symbols {
                    let callees = oci.find_callees(sym.scoped_name);
                    let scoped = oci.resolve(sym.scoped_name);

                    if callees.is_empty() {
                        output.push_str(&format!("{} has no recorded calls.\n", scoped));
                    } else {
                        output.push_str(&format!("{} calls {} functions:\n", scoped, callees.len()));
                        for call in &callees {
                            output.push_str(&format!("  - {} at line {}\n", call.callee_name, call.location.start_line));
                        }
                    }
                    output.push('\n');
                }

                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown operation: {}. Valid: callers, callees", req.op
            ))])),
        }
    }

    #[tool(description = "Run analysis: dead_code, coverage, churn, hotspots")]
    async fn analyze(
        &self,
        Parameters(req): Parameters<AnalysisRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let _oci = &state.oci_state;

        match req.analysis.as_str() {
            "dead_code" => {
                // TODO: Integrate with dead_code analysis module
                Ok(CallToolResult::success(vec![Content::text(
                    "Dead code analysis not yet implemented. Will analyze reachability from entry points."
                )]))
            }
            "coverage" => {
                match &req.coverage_file {
                    Some(_path) => {
                        // TODO: Integrate with coverage analysis module
                        Ok(CallToolResult::success(vec![Content::text(
                            "Coverage analysis not yet implemented. Will parse LLVM/tarpaulin JSON."
                        )]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "coverage_file parameter required for coverage analysis"
                    )])),
                }
            }
            "churn" => {
                let _days = req.days.unwrap_or(30);
                // TODO: Integrate with churn analysis module
                Ok(CallToolResult::success(vec![Content::text(
                    "Churn analysis not yet implemented. Will analyze git history."
                )]))
            }
            "hotspots" => {
                // TODO: Combine churn + complexity metrics
                Ok(CallToolResult::success(vec![Content::text(
                    "Hotspot analysis not yet implemented. Will combine churn frequency with complexity."
                )]))
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown analysis: {}. Valid: dead_code, coverage, churn, hotspots", req.analysis
            ))])),
        }
    }

    #[tool(description = "Search the codebase. Types: semantic, bm25, hybrid")]
    async fn search(
        &self,
        Parameters(req): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let _oci = &state.oci_state;
        let _max = req.max_results.unwrap_or(10);

        match req.search_type.as_str() {
            "semantic" => {
                // TODO: Integrate with semantic search
                Ok(CallToolResult::success(vec![Content::text(
                    "Semantic search not yet implemented. Will use embeddings + HNSW."
                )]))
            }
            "bm25" => {
                // TODO: Integrate with BM25 search
                Ok(CallToolResult::success(vec![Content::text(
                    "BM25 search not yet implemented. Will use inverted index."
                )]))
            }
            "hybrid" => {
                // TODO: Combine semantic + BM25
                Ok(CallToolResult::success(vec![Content::text(
                    "Hybrid search not yet implemented. Will combine semantic + BM25 with RRF."
                )]))
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown search type: {}. Valid: semantic, bm25, hybrid", req.search_type
            ))])),
        }
    }

    #[tool(description = "Get smart context for a location. Includes callers, callees, related types.")]
    async fn get_context(
        &self,
        Parameters(req): Parameters<ContextRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let _oci = &state.oci_state;
        let _file = PathBuf::from(&req.file);
        let _line = req.line;
        let _surrounding = req.surrounding.unwrap_or(10);
        let _max_tokens = req.max_tokens.unwrap_or(4000);

        // TODO: Integrate with context synthesis
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Context synthesis not yet implemented for {}:{}.\nWill include callers, callees, types, and imports ranked by PageRank.",
            req.file, req.line
        ))]))
    }

    #[tool(description = "Check for potential issues before writing code: duplication, naming conflicts")]
    async fn intervene(
        &self,
        Parameters(req): Parameters<InterventionRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let _oci = &state.oci_state;

        match req.check.as_str() {
            "duplication" => {
                match &req.signature {
                    Some(_sig) => {
                        // TODO: Integrate with intervention engine
                        Ok(CallToolResult::success(vec![Content::text(
                            "Duplication check not yet implemented. Will find similar signatures."
                        )]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "signature parameter required for duplication check"
                    )])),
                }
            }
            "naming" => {
                match &req.name {
                    Some(_name) => {
                        // TODO: Integrate with intervention engine
                        Ok(CallToolResult::success(vec![Content::text(
                            "Naming conflict check not yet implemented. Will check for conflicts."
                        )]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "name parameter required for naming check"
                    )])),
                }
            }
            "alternatives" => {
                match &req.name {
                    Some(_name) => {
                        // TODO: Integrate with intervention engine
                        Ok(CallToolResult::success(vec![Content::text(
                            "Alternatives suggestion not yet implemented. Will find reusable code."
                        )]))
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "name parameter required for alternatives check"
                    )])),
                }
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown check: {}. Valid: duplication, naming, alternatives", req.check
            ))])),
        }
    }

    #[tool(description = "Query module topology: modules, imports, pagerank, dependencies")]
    async fn topology(
        &self,
        Parameters(req): Parameters<TopologyRequest>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.read().await;
        let oci = &state.oci_state;
        let max = req.max_results.unwrap_or(20);

        match req.op.as_str() {
            "modules" => {
                let graph = oci.topology.read();
                let mut modules = Vec::new();

                for idx in graph.node_indices() {
                    if let crate::types::TopologyNode::Module { name, path, .. } = &graph[idx] {
                        modules.push(format!("- {} ({})", name, path.display()));
                    }
                }

                if modules.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No modules found. Run index build first."
                    )]))
                } else {
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Found {} modules:\n{}", modules.len(), modules.join("\n")
                    ))]))
                }
            }
            "imports" => {
                let mut output = String::from("File imports:\n\n");
                let mut count = 0;

                for entry in oci.imports.iter() {
                    if count >= max { break; }
                    let file_id = *entry.key();
                    let imports = entry.value();

                    // Find file path for this ID
                    if let Some(path_entry) = oci.file_ids.iter().find(|e| *e.value() == file_id) {
                        output.push_str(&format!("{}:\n", path_entry.key().display()));
                        for imp in imports.iter().take(5) {
                            output.push_str(&format!("  - {}{}\n", imp.path, if imp.is_glob { "::*" } else { "" }));
                        }
                        if imports.len() > 5 {
                            output.push_str(&format!("  ... and {} more\n", imports.len() - 5));
                        }
                        output.push('\n');
                        count += 1;
                    }
                }

                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "pagerank" => {
                let mut scores: Vec<_> = oci.topology_metrics.iter()
                    .map(|e| (*e.key(), e.value().relevance_score))
                    .collect();
                scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                let graph = oci.topology.read();
                let mut output = String::from("Top nodes by PageRank:\n\n");

                for (idx, score) in scores.iter().take(max) {
                    if let Some(node) = graph.node_weight(*idx) {
                        let name = match node {
                            crate::types::TopologyNode::Crate { name, .. } => format!("crate:{}", name),
                            crate::types::TopologyNode::Module { name, .. } => format!("mod:{}", name),
                            crate::types::TopologyNode::File { path, .. } => path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        };
                        output.push_str(&format!("{:.4}  {}\n", score, name));
                    }
                }

                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "dependencies" => {
                match &req.path {
                    Some(path) => {
                        let path = PathBuf::from(path);
                        if let Some(node_idx) = oci.path_to_node.get(&path) {
                            let graph = oci.topology.read();
                            let mut deps = Vec::new();

                            for edge in graph.edges(*node_idx) {
                                if let Some(target) = graph.node_weight(edge.target()) {
                                    let name = match target {
                                        crate::types::TopologyNode::File { path, .. } => path.display().to_string(),
                                        crate::types::TopologyNode::Module { name, .. } => name.clone(),
                                        crate::types::TopologyNode::Crate { name, .. } => name.clone(),
                                    };
                                    deps.push(format!("  -> {}", name));
                                }
                            }

                            Ok(CallToolResult::success(vec![Content::text(format!(
                                "Dependencies of {}:\n{}", path.display(), deps.join("\n")
                            ))]))
                        } else {
                            Ok(CallToolResult::error(vec![Content::text(format!(
                                "Path not found in topology: {}", path.display()
                            ))]))
                        }
                    }
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "path parameter required for dependencies query"
                    )])),
                }
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown operation: {}. Valid: modules, imports, pagerank, dependencies", req.op
            ))])),
        }
    }
}

// ============================================================================
// MCP Server Handler Implementation
// ============================================================================

#[tool_handler]
impl ServerHandler for OciServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(format!(
                "{} v{} - Omniscient Code Index. Semantic code search, call graphs, dead code analysis, and smart context synthesis.",
                SERVER_NAME, SERVER_VERSION
            )),
        }
    }
}

// ============================================================================
// Server Entry Point
// ============================================================================

pub async fn run_server(workspace_root: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("omni_index=info".parse().unwrap()),
        )
        .init();

    tracing::info!("Starting {} v{}", SERVER_NAME, SERVER_VERSION);
    tracing::info!("Workspace root: {}", workspace_root.display());

    let server = OciServer::new(workspace_root);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    tracing::info!("Server shutdown");
    Ok(())
}
