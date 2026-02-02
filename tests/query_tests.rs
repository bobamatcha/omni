use omni_index::query::{execute_query, load_search_index};
use omni_index::{IncrementalIndexer, IndexOptions, create_state};
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

#[tokio::test]
async fn test_incremental_indexing_skips_unchanged() {
    let temp = copy_fixture();
    let root = temp.path();
    let state = create_state(root.to_path_buf());
    let indexer = IncrementalIndexer::new();

    let report = indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");
    assert_eq!(report.parsed_files, report.total_files);

    let report = indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");
    assert_eq!(report.parsed_files, 0);
    assert_eq!(report.skipped_files, report.total_files);

    let extra_path = root.join("src/extra.rs");
    fs::write(
        &extra_path,
        "fn helper_token_magic() {\n    let name = \"magic\";\n}\n\nfn new_function() {}\n",
    )
    .expect("write");

    let report = indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");
    assert_eq!(report.parsed_files, 1);
}

#[tokio::test]
async fn test_query_ranking_and_schema() {
    let temp = copy_fixture();
    let root = temp.path();
    let state = create_state(root.to_path_buf());
    let indexer = IncrementalIndexer::new();

    indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");

    let index = load_search_index(root)
        .expect("load index")
        .expect("index exists");
    let response = execute_query(&index, "add numbers", 5, &Default::default());

    assert!(!response.results.is_empty());
    let top = &response.results[0];
    assert!(top.symbol.contains("add_numbers"));
    assert!(top.start_byte < top.end_byte);
    assert!(top.start_line >= 1);
    assert!(top.end_line >= top.start_line);
    assert!(top.start_col >= 1);
    assert!(top.end_col >= top.start_col);
    assert!(top.preview.contains("add_numbers"));
    assert_eq!(top.start_line, 2);
}

#[tokio::test]
async fn test_deleted_files_removed_from_index() {
    let temp = copy_fixture();
    let root = temp.path();
    let state = create_state(root.to_path_buf());
    let indexer = IncrementalIndexer::new();

    indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");

    let extra_path = root.join("src/extra.rs");
    fs::remove_file(&extra_path).expect("remove file");

    indexer
        .index(&state, root, &IndexOptions::default())
        .await
        .expect("index");

    let index = load_search_index(root)
        .expect("load index")
        .expect("index exists");
    let response = execute_query(&index, "helper token magic", 5, &Default::default());
    assert!(response.results.is_empty());
}
