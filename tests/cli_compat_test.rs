//! CLI compatibility tests (not part of Claudette contract).
//!
//! These tests verify CLI ergonomics and aliases that are useful
//! but not part of the stable Claudette interface.
//!
//! Only runs in full builds (requires `extras` feature).

#![cfg(feature = "extras")]

use serde_json::Value;
use std::process::Command;

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

#[test]
fn compat_index_workspace_alias() {
    // Regression test: --workspace should work as alias for --root
    // This is a convenience alias, not part of the Claudette contract
    let root = fixture_root();

    let (json, success) = run_cli_json(&["index", "--workspace", &root, "--json"]);

    assert!(success, "--workspace alias should work for index command");
    assert_eq!(json["ok"], true);
    assert_eq!(json["type"], "index");
}

#[test]
fn compat_query_workspace_alias() {
    // --workspace should work for query command too
    let root = fixture_root();

    // Index first
    let _ = run_cli_json(&["index", "--root", &root]);

    let (json, success) = run_cli_json(&["query", "--workspace", &root, "--json", "add"]);

    assert!(success, "--workspace alias should work for query command");
    assert_eq!(json["ok"], true);
}
