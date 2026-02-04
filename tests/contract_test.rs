//! Contract tests for Claudette integration.
//!
//! These tests verify the JSON schema that Claudette depends on.
//! DO NOT change these schemas without coordinating with Claudette.
//!
//! Core contract (Claudette interface):
//! ```bash
//! omni search <query> --json -w <workspace> -n <limit>
//! omni index --json
//! ```
//!
//! Expected JSON schemas are defined in these tests.
//!
//! Note: Error handling tests are in error_handling_test.rs (non-core).

use serde_json::Value;
use std::process::Command;

/// Helper to run the CLI and return parsed JSON
fn run_cli_json(args: &[&str]) -> (Value, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(args)
        .output()
        .expect("Failed to execute omni CLI");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (json, output.status.success())
}

fn fixture_root() -> String {
    format!("{}/tests/fixtures/basic", env!("CARGO_MANIFEST_DIR"))
}

/// Ensure index exists for contract tests
fn ensure_indexed(root: &str) {
    let _ = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(["index", "--root", root])
        .output();
}

// =============================================================================
// CORE CONTRACT: SEARCH (Primary Claudette interface)
// =============================================================================

#[test]
fn contract_search_json_schema() {
    // This is the exact interface Claudette uses:
    // omni search <query> --json -w <workspace> -n <limit>

    let root = fixture_root();
    ensure_indexed(&root);

    let (json, success) = run_cli_json(&["search", "add", "--json", "-w", &root, "-n", "3"]);

    assert!(success, "Search command must succeed");

    // Required top-level fields
    assert_eq!(json["ok"], true, "Must have ok: true on success");
    assert_eq!(json["type"], "search", "Must have type: 'search'");

    // Results array must exist
    assert!(json["results"].is_array(), "Must have results array");

    // Each result must have these fields
    if let Some(results) = json["results"].as_array() {
        if !results.is_empty() {
            let first = &results[0];
            assert!(first["symbol"].is_string(), "Result must have symbol: string");
            assert!(first["kind"].is_string(), "Result must have kind: string");
            assert!(first["file"].is_string(), "Result must have file: string");
            assert!(first["line"].is_number(), "Result must have line: number");
            assert!(first["score"].is_number(), "Result must have score: number");
        }
    }
}

#[test]
fn contract_search_respects_limit() {
    let root = fixture_root();
    ensure_indexed(&root);

    let (json, success) = run_cli_json(&["search", "fn", "--json", "-w", &root, "-n", "2"]);

    assert!(success);
    if let Some(results) = json["results"].as_array() {
        assert!(results.len() <= 2, "Must respect -n limit");
    }
}

#[test]
fn contract_search_workspace_flag() {
    // Verify -w flag works as workspace specifier (Claudette interface)
    let root = fixture_root();
    ensure_indexed(&root);

    let (json, success) = run_cli_json(&["search", "add", "--json", "-w", &root, "-n", "5"]);

    assert!(success, "-w flag must work as workspace specifier");
    assert_eq!(json["ok"], true);
}

#[test]
fn contract_search_result_fields_are_stable() {
    // Document the exact fields Claudette expects
    // This test serves as documentation and will fail if fields are renamed

    let root = fixture_root();
    ensure_indexed(&root);

    let (json, _) = run_cli_json(&["search", "add", "--json", "-w", &root, "-n", "1"]);

    if let Some(results) = json["results"].as_array() {
        if !results.is_empty() {
            let result = &results[0];

            // These field names are part of the Claudette contract
            let expected_fields = ["symbol", "kind", "file", "line", "score"];

            for field in expected_fields {
                assert!(
                    result.get(field).is_some(),
                    "Search result must contain field: {}",
                    field
                );
            }
        }
    }
}

// =============================================================================
// CORE CONTRACT: INDEX
// =============================================================================

#[test]
fn contract_index_json_schema() {
    let root = fixture_root();

    let (json, success) = run_cli_json(&["index", "--root", &root, "--json"]);

    assert!(success, "Index command must succeed");

    // Required top-level fields
    assert_eq!(json["ok"], true, "Must have ok: true on success");
    assert_eq!(json["type"], "index", "Must have type: 'index'");

    // Index-specific fields
    assert!(json["files"].is_number(), "Must have files: number");
    assert!(json["symbols"].is_number(), "Must have symbols: number");
    assert!(json["root"].is_string(), "Must have root: string");
}

#[test]
fn contract_index_result_fields_are_stable() {
    let root = fixture_root();

    let (json, _) = run_cli_json(&["index", "--root", &root, "--json"]);

    // These field names are part of the contract
    let expected_fields = ["ok", "type", "files", "symbols", "root"];

    for field in expected_fields {
        assert!(
            json.get(field).is_some(),
            "Index result must contain field: {}",
            field
        );
    }
}
