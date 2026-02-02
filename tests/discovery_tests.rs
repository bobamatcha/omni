use omni_index::FileDiscovery;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("basic")
}

fn copy_fixture() -> tempfile::TempDir {
    let src_root = fixture_root();
    let temp = tempfile::tempdir().expect("tempdir");

    for entry in walkdir::WalkDir::new(&src_root) {
        let entry = entry.expect("walkdir entry");
        let path = entry.path();
        let rel = path.strip_prefix(&src_root).expect("strip prefix");
        let dest = temp.path().join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest).expect("create dir");
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::copy(path, &dest).expect("copy file");
        }
    }

    temp
}

fn rels(root: &PathBuf, files: Vec<PathBuf>) -> HashSet<String> {
    files
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        })
        .collect()
}

#[test]
fn discovery_excludes_defaults() {
    let temp = copy_fixture();
    let root = temp.path().to_path_buf();
    let discovery = FileDiscovery::new();
    let files = discovery.discover(&root).expect("discover should work");
    let rel = rels(&root, files);

    assert!(rel.contains("src/lib.rs"));
    assert!(rel.contains("src/extra.rs"));
    assert!(!rel.contains("target/generated.rs"));
    assert!(!rel.contains("node_modules/ignored.rs"));
    assert!(!rel.contains(".git/ignored.rs"));
    assert!(!rel.contains(".hidden/hidden.rs"));
    assert!(!rel.contains("package-lock.json"));
    assert!(!rel.contains("yarn.lock"));
    assert!(!rel.contains("pnpm-lock.yaml"));
    assert!(!rel.contains("Cargo.lock"));
}

#[test]
fn discovery_include_can_override_default_excludes() {
    let temp = copy_fixture();
    let root = temp.path().to_path_buf();
    let discovery = FileDiscovery::new().with_include("target/**");
    let files = discovery.discover(&root).expect("discover should work");
    let rel = rels(&root, files);

    assert!(rel.contains("target/generated.rs"));
}
