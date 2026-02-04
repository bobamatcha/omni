//! Error handling tests (not part of Claudette contract).
//!
//! These tests verify error behavior for robustness. They are NOT part
//! of the stable Claudette contract - the error schema may change.
//!
//! Only runs in full builds (requires `extras` feature).

#![cfg(feature = "extras")]

use serde_json::Value;
use std::process::Command;

fn fixture_root() -> String {
    format!("{}/tests/fixtures/basic", env!("CARGO_MANIFEST_DIR"))
}

fn ensure_indexed(root: &str) {
    let _ = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(["index", "--root", root])
        .output();
}

#[test]
fn error_schema_invalid_query() {
    let root = fixture_root();
    ensure_indexed(&root);

    // Empty query should fail with proper error schema
    let output = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(["search", "", "--json", "-w", &root])
        .output()
        .expect("Failed to execute omni CLI");

    // Error output goes to stderr for JSON mode
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: Value = serde_json::from_str(&stderr).unwrap_or(Value::Null);

    assert!(!output.status.success(), "Empty query should fail");

    // Error schema
    assert_eq!(json["ok"], false, "Must have ok: false on error");
    assert!(json["error"].is_object(), "Must have error object");
    assert!(
        json["error"]["code"].is_string(),
        "Error must have code: string"
    );
    assert!(
        json["error"]["message"].is_string(),
        "Error must have message: string"
    );
}

#[test]
fn error_schema_missing_workspace() {
    // Query a non-existent workspace
    let output = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(["search", "test", "--json", "-w", "/nonexistent/path"])
        .output()
        .expect("Failed to execute omni CLI");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: Value = serde_json::from_str(&stderr).unwrap_or(Value::Null);

    // Should fail gracefully with error schema
    if !output.status.success() {
        assert_eq!(json["ok"], false);
        assert!(json["error"].is_object());
    }
}
