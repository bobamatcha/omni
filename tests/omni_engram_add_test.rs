use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn omni_engram_add_uses_engram_cli_fallback() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();

    let omni_bin = root.join("omni-mock.sh");
    fs::write(
        &omni_bin,
        r#"#!/usr/bin/env bash
set -euo pipefail
cat <<'JSON'
{"type":"ExportEngram","export":{"content":"hello world","metadata":{}}}
JSON
"#,
    )
    .expect("write omni mock");
    make_executable(&omni_bin);

    let node_bin = root.join("node-mock.sh");
    let log_path = root.join("engram.log");
    fs::write(
        &node_bin,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
echo "$@" > "{}"
"#,
            log_path.display()
        ),
    )
    .expect("write node mock");
    make_executable(&node_bin);

    let engram_cli = root.join("engram-cli.js");
    fs::write(&engram_cli, "// mock").expect("write engram cli");

    let script = PathBuf::from("scripts/omni-engram-add.sh");
    let output = Command::new("bash")
        .arg(script)
        .arg(root)
        .env("OMNI_BIN", &omni_bin)
        .env("ENGRAM_BIN", "/missing/engram")
        .env("ENGRAM_NODE", &node_bin)
        .env("ENGRAM_CLI", &engram_cli)
        .output()
        .expect("run script");

    assert!(
        output.status.success(),
        "script should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let logged = fs::read_to_string(&log_path).expect("read log");
    assert!(
        logged.contains("add"),
        "engram CLI should be invoked with add command"
    );
}

fn make_executable(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("set perms");
}
