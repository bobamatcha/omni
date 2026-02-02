use omni_index::parsing::{LanguageParser, typescript::TypeScriptParser};
use std::fs;
use tempfile::TempDir;
use tree_sitter::Parser;

#[test]
fn typescript_parser_language_reusable() {
    let parser = TypeScriptParser::new_typescript();
    let _ = parser.language();
    let _ = parser.language();
}

fn parse_ts_source(parser: &TypeScriptParser, source: &str) -> tree_sitter::Tree {
    let mut ts_parser = Parser::new();
    ts_parser
        .set_language(&parser.language())
        .expect("language should load");
    ts_parser.parse(source, None).expect("parse should succeed")
}

#[test]
fn typescript_scoped_names_use_workspace_relative_path() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    fs::write(root.join("package.json"), "{}").expect("package.json");

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).expect("src dir");
    let file_path = src_dir.join("alpha.ts");
    let source = "export function greet() { return 'hi'; }";

    let parser = TypeScriptParser::new_typescript();
    let tree = parse_ts_source(&parser, source);
    let interner = lasso::ThreadedRodeo::default();
    let symbols = parser
        .extract_symbols(&tree, source, &file_path, &interner)
        .expect("symbols");

    let scoped_names: Vec<String> = symbols
        .iter()
        .map(|s| interner.resolve(&s.scoped_name).to_string())
        .collect();

    assert!(
        scoped_names
            .iter()
            .any(|name| name.starts_with("file:src/alpha.ts::greet")),
        "expected scoped name to be workspace-relative, got: {scoped_names:?}"
    );
}

#[test]
fn typescript_calls_extract_computed_string_property() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    fs::write(root.join("package.json"), "{}").expect("package.json");

    let file_path = root.join("index.ts");
    let source = r#"
        function run() {
            obj["foo-bar"]();
        }
    "#;

    let parser = TypeScriptParser::new_typescript();
    let tree = parse_ts_source(&parser, source);
    let interner = lasso::ThreadedRodeo::default();
    let calls = parser
        .extract_calls(&tree, source, &file_path, &interner)
        .expect("calls");

    let has_expected = calls.iter().any(|call| call.callee_name == "foo-bar");
    assert!(
        has_expected,
        "expected computed property call to use literal name"
    );
}
