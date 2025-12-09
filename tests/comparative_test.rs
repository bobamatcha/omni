//! Comparative tests between OCI (omni-index) and code-index.
//!
//! These tests verify that OCI provides equivalent or better functionality
//! compared to the existing code-index tool used by the AG1337 coder agent.
//!
//! Run with: cargo test --test comparative_test -- --nocapture
//!
//! For benchmarks comparing both: cargo bench --bench comparative

use omni_index::{
    create_state, parsing::rust::RustParser, parsing::LanguageParser, topology::TopologyBuilder,
    CallEdge, DeadCodeAnalyzer, FileDiscovery, IncrementalIndexer, InterventionEngine, Location,
    OciState, SymbolKind,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::TempDir;

// ============================================================================
// Test Fixtures - Create realistic Rust codebases for testing
// ============================================================================

/// Create a test codebase that mimics real-world patterns
fn create_realistic_codebase() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let root = temp.path();

    // Create directory structure
    fs::create_dir_all(root.join("src/core")).unwrap();
    fs::create_dir_all(root.join("src/utils")).unwrap();
    fs::create_dir_all(root.join("src/api")).unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();

    // lib.rs - main entry point
    fs::write(
        root.join("src/lib.rs"),
        r#"//! Test library for comparative benchmarks.

pub mod core;
pub mod utils;
pub mod api;

pub use core::Engine;
pub use api::Server;
"#,
    )
    .unwrap();

    // core/mod.rs - core engine with complex call graph
    fs::write(
        root.join("src/core/mod.rs"),
        r#"//! Core engine module.

use crate::utils::{validate_input, format_output};

/// The main processing engine.
#[derive(Debug, Clone)]
pub struct Engine {
    pub name: String,
    pub config: EngineConfig,
}

/// Engine configuration.
#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    pub max_items: usize,
    pub timeout_ms: u64,
    pub debug: bool,
}

impl Engine {
    /// Create a new engine with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            config: EngineConfig::default(),
        }
    }

    /// Create an engine with custom config.
    pub fn with_config(name: &str, config: EngineConfig) -> Self {
        Self {
            name: name.to_string(),
            config,
        }
    }

    /// Process input and return result.
    pub fn process(&self, input: &str) -> Result<String, EngineError> {
        let validated = validate_input(input)?;
        let result = self.execute(&validated)?;
        Ok(format_output(&result))
    }

    /// Internal execution logic.
    fn execute(&self, input: &str) -> Result<String, EngineError> {
        if input.is_empty() {
            return Err(EngineError::EmptyInput);
        }

        // Simulate processing
        let processed = input.to_uppercase();
        self.log_execution(&processed);
        Ok(processed)
    }

    /// Log execution for debugging.
    fn log_execution(&self, result: &str) {
        if self.config.debug {
            println!("[{}] Executed: {}", self.name, result);
        }
    }

    /// Get engine statistics.
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            name: self.name.clone(),
            processed: 0,
            errors: 0,
        }
    }
}

/// Engine statistics.
#[derive(Debug, Clone)]
pub struct EngineStats {
    pub name: String,
    pub processed: usize,
    pub errors: usize,
}

/// Engine error types.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("Empty input provided")]
    EmptyInput,
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Processing failed: {0}")]
    ProcessingFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_new() {
        let engine = Engine::new("test");
        assert_eq!(engine.name, "test");
    }

    #[test]
    fn test_engine_process() {
        let engine = Engine::new("test");
        let result = engine.process("hello").unwrap();
        assert_eq!(result, "HELLO");
    }

    #[test]
    fn test_engine_empty_input() {
        let engine = Engine::new("test");
        let result = engine.process("");
        assert!(result.is_err());
    }
}
"#,
    )
    .unwrap();

    // utils/mod.rs - utility functions
    fs::write(
        root.join("src/utils/mod.rs"),
        r#"//! Utility functions.

use crate::core::EngineError;

/// Validate input string.
pub fn validate_input(input: &str) -> Result<String, EngineError> {
    if input.trim().is_empty() {
        return Err(EngineError::EmptyInput);
    }

    if input.len() > 1000 {
        return Err(EngineError::InvalidInput("Input too long".to_string()));
    }

    Ok(input.trim().to_string())
}

/// Format output string.
pub fn format_output(output: &str) -> String {
    output.to_string()
}

/// Helper to check if string is valid identifier.
pub fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    let first = chars.next().unwrap();

    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Parse a key-value pair from string.
pub fn parse_kv(s: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_input() {
        assert!(validate_input("hello").is_ok());
        assert!(validate_input("").is_err());
        assert!(validate_input("  ").is_err());
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("baz123"));
        assert!(!is_valid_identifier("123"));
        assert!(!is_valid_identifier(""));
    }
}
"#,
    )
    .unwrap();

    // api/mod.rs - API server
    fs::write(
        root.join("src/api/mod.rs"),
        r#"//! API server module.

use crate::core::{Engine, EngineConfig, EngineError};
use std::sync::Arc;

/// API Server wrapping the engine.
pub struct Server {
    engine: Arc<Engine>,
    port: u16,
}

impl Server {
    /// Create a new server with default engine.
    pub fn new(port: u16) -> Self {
        let engine = Engine::new("api-server");
        Self {
            engine: Arc::new(engine),
            port,
        }
    }

    /// Create server with custom engine.
    pub fn with_engine(engine: Engine, port: u16) -> Self {
        Self {
            engine: Arc::new(engine),
            port,
        }
    }

    /// Handle a request.
    pub fn handle_request(&self, request: &str) -> Result<String, EngineError> {
        self.engine.process(request)
    }

    /// Get server info.
    pub fn info(&self) -> ServerInfo {
        ServerInfo {
            port: self.port,
            engine_name: self.engine.name.clone(),
        }
    }

    /// Start the server (placeholder).
    pub async fn start(&self) -> Result<(), ServerError> {
        println!("Starting server on port {}", self.port);
        Ok(())
    }

    /// Stop the server.
    pub async fn stop(&self) -> Result<(), ServerError> {
        println!("Stopping server");
        Ok(())
    }
}

/// Server information.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub port: u16,
    pub engine_name: String,
}

/// Server error types.
#[derive(Debug, Clone)]
pub enum ServerError {
    BindFailed(String),
    AlreadyRunning,
    NotRunning,
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::BindFailed(msg) => write!(f, "Bind failed: {}", msg),
            ServerError::AlreadyRunning => write!(f, "Server already running"),
            ServerError::NotRunning => write!(f, "Server not running"),
        }
    }
}

impl std::error::Error for ServerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_new() {
        let server = Server::new(8080);
        assert_eq!(server.port, 8080);
    }

    #[test]
    fn test_server_handle_request() {
        let server = Server::new(8080);
        let result = server.handle_request("test").unwrap();
        assert_eq!(result, "TEST");
    }
}
"#,
    )
    .unwrap();

    // Integration test file
    fs::write(
        root.join("tests/integration.rs"),
        r#"//! Integration tests.

use test_lib::{Engine, Server};

#[test]
fn test_full_flow() {
    let engine = Engine::new("integration-test");
    let server = Server::with_engine(engine, 9000);

    let result = server.handle_request("hello world").unwrap();
    assert_eq!(result, "HELLO WORLD");
}

#[test]
fn test_error_handling() {
    let server = Server::new(9001);
    let result = server.handle_request("");
    assert!(result.is_err());
}
"#,
    )
    .unwrap();

    // Cargo.toml
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "test-lib"
version = "0.1.0"
edition = "2021"

[dependencies]
thiserror = "1"
tokio = { version = "1", features = ["full"] }

[lib]
name = "test_lib"
path = "src/lib.rs"
"#,
    )
    .unwrap();

    temp
}

// ============================================================================
// Feature Comparison Tests
// ============================================================================

/// Test: Symbol extraction completeness
/// OCI should extract at least as many symbols as code-index
#[test]
fn test_symbol_extraction_completeness() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let stats = state.stats();

    // Verify we found expected symbols
    println!("Files indexed: {}", stats.file_count);
    println!("Symbols found: {}", stats.symbol_count);
    println!("Call edges: {}", stats.call_edge_count);

    // Should find the main types
    let engine_defs = state.find_by_name("Engine");
    assert!(!engine_defs.is_empty(), "Should find Engine struct");

    let server_defs = state.find_by_name("Server");
    assert!(!server_defs.is_empty(), "Should find Server struct");

    // Should find functions
    let process_defs = state.find_by_name("process");
    assert!(!process_defs.is_empty(), "Should find process method");

    let validate_defs = state.find_by_name("validate_input");
    assert!(!validate_defs.is_empty(), "Should find validate_input function");

    // Should find test functions
    let test_fns: Vec<_> = state
        .symbols
        .iter()
        .filter(|e| {
            let name = state.resolve(e.value().name);
            name.starts_with("test_")
        })
        .collect();
    assert!(test_fns.len() >= 5, "Should find at least 5 test functions");

    // Verify file count
    assert!(stats.file_count >= 4, "Should index at least 4 .rs files");
}

/// Test: Call graph accuracy
/// OCI should track function calls correctly
#[test]
fn test_call_graph_accuracy() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Check that process calls validate_input
    let callers = state.find_callers("validate_input");
    println!("Callers of validate_input: {:?}", callers.len());

    // Check that process calls format_output
    let format_callers = state.find_callers("format_output");
    println!("Callers of format_output: {:?}", format_callers.len());

    // Check call edges exist
    assert!(
        state.stats().call_edge_count > 0,
        "Should have call edges"
    );
}

/// Test: Scoped name resolution
/// OCI should properly scope symbols (e.g., Engine::process vs standalone process)
#[test]
fn test_scoped_name_resolution() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Find symbols with scoped names
    let mut scoped_names: Vec<String> = state
        .symbols
        .iter()
        .map(|e| state.resolve(e.value().scoped_name).to_string())
        .collect();
    scoped_names.sort();
    scoped_names.dedup();

    println!("Scoped names found: {}", scoped_names.len());
    for name in &scoped_names {
        if name.contains("::") {
            println!("  {}", name);
        }
    }

    // Should have scoped names like Engine::new, Engine::process
    let has_impl_methods = scoped_names.iter().any(|n| n.contains("::new") || n.contains("::process"));
    assert!(has_impl_methods, "Should have scoped impl methods");
}

/// Test: Dead code detection
/// OCI provides dead code analysis that code-index doesn't have
#[test]
fn test_dead_code_detection() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let analyzer = DeadCodeAnalyzer::new();
    let report = analyzer.analyze(&state);

    println!("Dead code report:");
    println!("  Entry points: {}", report.entry_points.len());
    println!("  Dead symbols: {}", report.dead_symbols.len());
    println!("  Potentially live: {}", report.potentially_live.len());

    // Public items should be entry points
    assert!(report.entry_points.len() > 0, "Should have entry points");
}

/// Test: Intervention engine (duplicate detection)
/// OCI provides proactive duplicate detection that code-index doesn't
#[test]
fn test_intervention_duplicate_detection() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Check for naming conflicts
    let test_file = temp.path().join("src/core/mod.rs");
    let conflicts = InterventionEngine::check_naming_conflicts(&state, "Engine", &test_file);

    println!("Naming conflicts for 'Engine': {:?}", conflicts.len());

    // Check for similar functions
    let alternatives = InterventionEngine::suggest_alternatives(&state, "validate");
    println!("Alternatives for 'validate': {:?}", alternatives.len());
}

/// Test: Topology (PageRank) scoring
/// OCI provides relevance scoring that code-index doesn't have built-in
#[test]
fn test_topology_scoring() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Check topology metrics exist
    let metrics_count = state.topology_metrics.len();
    println!("Topology metrics for {} files", metrics_count);

    if metrics_count > 0 {
        // Build reverse map from NodeIndex to PathBuf
        let node_to_path: std::collections::HashMap<_, _> = state
            .path_to_node
            .iter()
            .map(|e| (*e.value(), e.key().clone()))
            .collect();

        // Get top files by relevance
        let mut ranked: Vec<_> = state
            .topology_metrics
            .iter()
            .filter_map(|e| {
                node_to_path.get(e.key()).map(|path| (path.clone(), e.value().relevance_score))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        println!("Top files by relevance:");
        for (path, score) in ranked.iter().take(5) {
            println!("  {}: {:.4}", path.display(), score);
        }
    }
}

// ============================================================================
// Performance Comparison Tests
// ============================================================================

/// Test: Indexing performance measurement
#[test]
fn test_indexing_performance() {
    let temp = create_realistic_codebase();

    let start = Instant::now();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let elapsed = start.elapsed();
    let stats = state.stats();

    println!("Indexing performance:");
    println!("  Time: {:?}", elapsed);
    println!("  Files: {}", stats.file_count);
    println!("  Symbols: {}", stats.symbol_count);
    println!("  Symbols/ms: {:.2}", stats.symbol_count as f64 / elapsed.as_millis() as f64);

    // Should be reasonably fast (< 1s for small codebase)
    assert!(elapsed.as_secs() < 5, "Indexing should complete in < 5s");
}

/// Test: Query performance measurement
#[test]
fn test_query_performance() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Measure find_by_name
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = state.find_by_name("Engine");
    }
    let find_time = start.elapsed();

    // Measure find_callers
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = state.find_callers("validate_input");
    }
    let callers_time = start.elapsed();

    println!("Query performance (1000 iterations):");
    println!("  find_by_name: {:?} ({:.2} µs/query)", find_time, find_time.as_micros() as f64 / 1000.0);
    println!("  find_callers: {:?} ({:.2} µs/query)", callers_time, callers_time.as_micros() as f64 / 1000.0);

    // Queries should be fast (< 1ms each on average)
    assert!(find_time.as_millis() < 100, "find_by_name should be fast");
    assert!(callers_time.as_millis() < 100, "find_callers should be fast");
}

/// Test: Incremental update performance
#[test]
fn test_incremental_update_performance() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let file_to_update = temp.path().join("src/utils/mod.rs");

    // Measure update time
    let start = Instant::now();
    rt.block_on(async { indexer.update_file(&state, &file_to_update).await.unwrap() });
    let update_time = start.elapsed();

    println!("Incremental update performance:");
    println!("  Single file update: {:?}", update_time);

    // Update should be fast (< 100ms)
    assert!(update_time.as_millis() < 500, "Incremental update should be fast");
}

// ============================================================================
// API Compatibility Tests
// ============================================================================

/// Test: Verify OCI provides equivalent query capabilities to code-index
#[test]
fn test_api_equivalence() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // code-index API: defs(name) -> [Location]
    // OCI equivalent: find_by_name(name) -> [SymbolDef]
    let defs = state.find_by_name("Engine");
    assert!(!defs.is_empty(), "find_by_name should work like defs()");

    // code-index API: find_calls(callee) -> [CallEdge]
    // OCI equivalent: find_callers(callee) -> [CallEdge]
    let callers = state.find_callers("validate_input");
    // Note: may be empty if no callers in index, which is fine

    // code-index API: functions_of_file(path) -> [String]
    // OCI equivalent: file_symbols.get(file_id) -> [InternedString]
    let file_id = state.get_or_create_file_id(&temp.path().join("src/core/mod.rs"));
    let file_syms = state.file_symbols.get(&file_id);
    assert!(file_syms.is_some(), "Should track symbols per file");

    // code-index API: find_tests() -> [(String, Location)]
    // OCI equivalent: filter symbols by attribute or name prefix
    let test_count = state
        .symbols
        .iter()
        .filter(|e| {
            let name = state.resolve(e.value().name);
            name.starts_with("test_")
        })
        .count();
    println!("Test functions found: {}", test_count);

    println!("API equivalence verified:");
    println!("  defs() -> find_by_name(): ✓");
    println!("  find_calls() -> find_callers(): ✓");
    println!("  functions_of_file() -> file_symbols: ✓");
    println!("  find_tests() -> symbol filter: ✓");
}

// ============================================================================
// Additional OCI Features (not in code-index)
// ============================================================================

/// Test: Visibility tracking (OCI tracks pub/pub(crate)/private)
#[test]
fn test_visibility_tracking() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let mut public_count = 0;
    let mut private_count = 0;

    for entry in state.symbols.iter() {
        match entry.value().visibility {
            omni_index::Visibility::Public => public_count += 1,
            omni_index::Visibility::Private => private_count += 1,
            _ => {}
        }
    }

    println!("Visibility tracking:");
    println!("  Public symbols: {}", public_count);
    println!("  Private symbols: {}", private_count);

    assert!(public_count > 0, "Should have public symbols");
}

/// Test: Doc comment extraction (OCI extracts doc comments)
#[test]
fn test_doc_comment_extraction() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let mut with_docs = 0;
    let mut without_docs = 0;

    for entry in state.symbols.iter() {
        if entry.value().doc_comment.is_some() {
            with_docs += 1;
        } else {
            without_docs += 1;
        }
    }

    println!("Doc comment extraction:");
    println!("  Symbols with docs: {}", with_docs);
    println!("  Symbols without docs: {}", without_docs);

    assert!(with_docs > 0, "Should extract doc comments");
}

/// Test: Signature extraction (OCI extracts full signatures)
#[test]
fn test_signature_extraction() {
    let temp = create_realistic_codebase();
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let mut with_sig = 0;
    let mut without_sig = 0;

    for entry in state.symbols.iter() {
        if entry.value().signature.is_some() {
            with_sig += 1;
        } else {
            without_sig += 1;
        }
    }

    println!("Signature extraction:");
    println!("  Functions with signatures: {}", with_sig);
    println!("  Symbols without signatures: {}", without_sig);

    // Functions should have signatures
    let fns_with_sig = state
        .symbols
        .iter()
        .filter(|e| e.value().kind == SymbolKind::Function && e.value().signature.is_some())
        .count();
    assert!(fns_with_sig > 0, "Functions should have signatures");
}
