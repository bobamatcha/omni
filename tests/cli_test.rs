//! CLI integration tests for omni-cli
//!
//! Following TDD: These tests define the expected CLI behavior.

use std::process::Command;

/// Helper to run the CLI
fn run_cli(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(args)
        .output()
        .expect("Failed to execute omni CLI");
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

#[test]
fn test_help_command() {
    let (stdout, _, success) = run_cli(&["--help"]);
    assert!(success, "Help command should succeed");
    assert!(stdout.contains("omni"), "Should mention omni");
    assert!(stdout.contains("index"), "Should mention index command");
    assert!(stdout.contains("search"), "Should mention search command");
}

#[test]
fn test_version_command() {
    let (stdout, _, success) = run_cli(&["--version"]);
    assert!(success, "Version command should succeed");
    assert!(stdout.contains("0.1.0"), "Should show version");
}

#[test]
fn test_index_command_on_self() {
    // Index the omni repo itself
    let (stdout, stderr, success) = run_cli(&[
        "index",
        "--workspace", env!("CARGO_MANIFEST_DIR"),
    ]);
    assert!(success, "Index command should succeed: {}", stderr);
    assert!(stdout.contains("Indexed") || stdout.contains("symbols"), 
        "Should report indexing results: {}", stdout);
}

#[test]
fn test_search_command() {
    // First index, then search
    let _ = run_cli(&["index", "--workspace", env!("CARGO_MANIFEST_DIR")]);
    
    let (stdout, stderr, success) = run_cli(&[
        "search",
        "--workspace", env!("CARGO_MANIFEST_DIR"),
        "hybrid search",
    ]);
    assert!(success, "Search command should succeed: {}", stderr);
    // Should find something since omni has hybrid search code
}

#[test]
fn test_symbol_command() {
    let _ = run_cli(&["index", "--workspace", env!("CARGO_MANIFEST_DIR")]);
    
    let (stdout, stderr, success) = run_cli(&[
        "symbol",
        "--workspace", env!("CARGO_MANIFEST_DIR"),
        "HybridSearch",
    ]);
    assert!(success, "Symbol command should succeed: {}", stderr);
    assert!(stdout.contains("HybridSearch"), "Should find HybridSearch: {}", stdout);
}

#[test]
fn test_json_output() {
    let _ = run_cli(&["index", "--workspace", env!("CARGO_MANIFEST_DIR")]);
    
    let (stdout, _, success) = run_cli(&[
        "symbol",
        "--workspace", env!("CARGO_MANIFEST_DIR"),
        "--json",
        "OciState",
    ]);
    assert!(success, "JSON output should succeed");
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
}

#[test]
fn test_dead_code_analysis() {
    let _ = run_cli(&["index", "--workspace", env!("CARGO_MANIFEST_DIR")]);
    
    let (stdout, stderr, success) = run_cli(&[
        "analyze",
        "--workspace", env!("CARGO_MANIFEST_DIR"),
        "dead-code",
    ]);
    assert!(success, "Dead code analysis should succeed: {}", stderr);
    // Should produce some output
    assert!(!stdout.is_empty(), "Should produce output");
}
