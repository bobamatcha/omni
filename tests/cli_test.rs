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

fn fixture_root() -> String {
    format!("{}/tests/fixtures/basic", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn test_help_command() {
    let (stdout, _, success) = run_cli(&["--help"]);
    assert!(success, "Help command should succeed");
    assert!(stdout.contains("omni"), "Should mention omni");
    assert!(stdout.contains("index"), "Should mention index command");
    assert!(stdout.contains("query"), "Should mention query command");
}

#[test]
fn test_version_command() {
    let (stdout, _, success) = run_cli(&["--version"]);
    assert!(success, "Version command should succeed");
    assert!(stdout.contains("0.1.0"), "Should show version");
}

#[test]
fn test_index_command_on_fixture() {
    let root = fixture_root();
    let (stdout, stderr, success) = run_cli(&["index", "--root", &root]);
    assert!(success, "Index command should succeed: {}", stderr);
    assert!(
        stdout.contains("Indexed"),
        "Should report indexing results: {}",
        stdout
    );
}

#[test]
fn test_index_all_command_on_multiple_repos() {
    let root = fixture_root();
    let (stdout, stderr, success) = run_cli(&["index-all", env!("CARGO_MANIFEST_DIR"), &root]);
    assert!(success, "Index-all command should succeed: {}", stderr);
    assert!(
        stdout.contains("Indexed"),
        "Should report indexing results: {}",
        stdout
    );
}

#[test]
fn test_query_command() {
    let root = fixture_root();
    let _ = run_cli(&["index", "--root", &root]);

    let (_stdout, stderr, success) = run_cli(&["query", "--root", &root, "add numbers"]);
    assert!(success, "Query command should succeed: {}", stderr);
}

#[test]
fn test_symbol_command() {
    let _ = run_cli(&["index", "--root", env!("CARGO_MANIFEST_DIR")]);

    let (stdout, stderr, success) = run_cli(&[
        "symbol",
        "--root",
        env!("CARGO_MANIFEST_DIR"),
        "HybridSearch",
    ]);
    assert!(success, "Symbol command should succeed: {}", stderr);
    assert!(
        stdout.contains("HybridSearch"),
        "Should find HybridSearch: {}",
        stdout
    );
}

#[test]
fn test_json_output() {
    let root = fixture_root();
    let _ = run_cli(&["index", "--root", &root]);

    let (stdout, _, success) = run_cli(&["query", "--root", &root, "--json", "add numbers"]);
    assert!(success, "JSON output should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON");
    assert_eq!(value["ok"], true);
    assert_eq!(value["type"], "query");
}

#[test]
#[cfg(feature = "analysis")]
fn test_dead_code_analysis() {
    let _ = run_cli(&["index", "--root", env!("CARGO_MANIFEST_DIR")]);

    let (stdout, stderr, success) =
        run_cli(&["analyze", "--root", env!("CARGO_MANIFEST_DIR"), "dead-code"]);
    assert!(success, "Dead code analysis should succeed: {}", stderr);
    assert!(!stdout.is_empty(), "Should produce output");
}
