//! Property-based tests for the Omniscient Code Index.
//!
//! Uses proptest to generate random inputs and verify invariants hold.

use omni_index::parsing::LanguageParser;
use omni_index::parsing::rust::RustParser;
use omni_index::topology::TopologyBuilder;
use omni_index::*;
use proptest::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

// ============================================================================
// Strategies for generating test data
// ============================================================================

/// Generate valid Rust identifiers
fn rust_identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,20}".prop_filter("must be valid identifier", |s| {
        !s.is_empty()
            && ![
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod",
                "const", "static", "async", "await", "self", "super", "crate", "where", "for",
                "in", "if", "else", "match", "loop", "while", "break", "continue", "return",
                "type", "as", "ref", "move", "dyn", "true", "false",
            ]
            .contains(&s.as_str())
    })
}

/// Generate scoped names like "crate::module::name"
fn scoped_name() -> impl Strategy<Value = String> {
    prop::collection::vec(rust_identifier(), 1..=3)
        .prop_map(|parts| format!("crate::{}", parts.join("::")))
}

/// Generate simple type names
fn type_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("i32".to_string()),
        Just("i64".to_string()),
        Just("u32".to_string()),
        Just("bool".to_string()),
        Just("String".to_string()),
        Just("()".to_string()),
    ]
}

/// Generate function signatures
fn function_signature() -> impl Strategy<Value = String> {
    (
        rust_identifier(),
        prop::collection::vec(
            (rust_identifier(), type_name()).prop_map(|(n, t)| format!("{}: {}", n, t)),
            0..=3,
        ),
        prop::option::of(type_name()),
    )
        .prop_map(|(name, params, ret)| {
            let mut sig = String::from("fn ");
            sig.push_str(&name);
            sig.push('(');
            sig.push_str(&params.join(", "));
            sig.push(')');
            if let Some(ret_type) = ret {
                sig.push_str(" -> ");
                sig.push_str(&ret_type);
            }
            sig
        })
}

/// Generate file paths
fn file_path() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec(rust_identifier(), 1..=3).prop_map(|parts| {
        let mut path = PathBuf::from("src");
        for part in parts {
            path.push(part);
        }
        path.set_extension("rs");
        path
    })
}

// ============================================================================
// State Management Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property: Interning the same string twice returns the same key
    #[test]
    fn intern_idempotent(s in "[a-zA-Z_][a-zA-Z0-9_]{0,50}") {
        let state = OciState::new(PathBuf::from("/test"));
        let key1 = state.intern(&s);
        let key2 = state.intern(&s);
        prop_assert_eq!(key1, key2);
    }

    /// Property: Resolving an interned string returns the original
    #[test]
    fn intern_roundtrip(s in "[a-zA-Z_][a-zA-Z0-9_]{0,50}") {
        let state = OciState::new(PathBuf::from("/test"));
        let key = state.intern(&s);
        let resolved = state.resolve(key);
        prop_assert_eq!(resolved, &s);
    }

    /// Property: File IDs are unique for different paths
    #[test]
    fn file_ids_unique(paths in prop::collection::hash_set(file_path(), 1..20)) {
        let state = OciState::new(PathBuf::from("/test"));
        let mut ids = HashSet::new();

        for path in paths {
            let id = state.get_or_create_file_id(&path);
            prop_assert!(!ids.contains(&id), "Duplicate file ID generated");
            ids.insert(id);
        }
    }

    /// Property: Getting file ID twice returns the same ID
    #[test]
    fn file_id_idempotent(path in file_path()) {
        let state = OciState::new(PathBuf::from("/test"));
        let id1 = state.get_or_create_file_id(&path);
        let id2 = state.get_or_create_file_id(&path);
        prop_assert_eq!(id1, id2);
    }

    /// Property: Symbol count matches number of added symbols
    #[test]
    fn symbol_count_accurate(names in prop::collection::vec(scoped_name(), 1..30)) {
        let state = OciState::new(PathBuf::from("/test"));
        let mut added = HashSet::new();

        for name in &names {
            if added.insert(name.clone()) {
                let scoped = state.intern(name);
                let simple = state.intern(name.split("::").last().unwrap());

                let symbol = SymbolDef {
                    name: simple,
                    scoped_name: scoped,
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    location: Location {
                        file: PathBuf::from("test.rs"),
                        start_byte: 0,
                        end_byte: 10,
                        start_line: 1,
                        start_col: 0,
                        end_line: 1,
                        end_col: 10,
                    },
                    signature: None,
                    doc_comment: None,
                    attributes: vec![],
                    parent: None,
                };
                state.add_symbol(symbol);
            }
        }

        let stats = state.stats();
        prop_assert_eq!(stats.symbol_count as usize, added.len());
    }

    /// Property: Finding by name returns only symbols with that name
    #[test]
    fn find_by_name_correct(
        target_name in rust_identifier(),
        other_names in prop::collection::vec(rust_identifier(), 0..5)
    ) {
        let state = OciState::new(PathBuf::from("/test"));

        // Add the target symbol
        let scoped = state.intern(&format!("crate::{}", target_name));
        let simple = state.intern(&target_name);

        state.add_symbol(SymbolDef {
            name: simple,
            scoped_name: scoped,
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            location: Location {
                file: PathBuf::from("test.rs"),
                start_byte: 0,
                end_byte: 10,
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 10,
            },
            signature: None,
            doc_comment: None,
            attributes: vec![],
            parent: None,
        });

        // Add other symbols
        for (i, name) in other_names.iter().enumerate() {
            if name != &target_name {
                let scoped = state.intern(&format!("crate::other{}::{}", i, name));
                let simple = state.intern(name);

                state.add_symbol(SymbolDef {
                    name: simple,
                    scoped_name: scoped,
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    location: Location {
                        file: PathBuf::from("other.rs"),
                        start_byte: 0,
                        end_byte: 10,
                        start_line: 1,
                        start_col: 0,
                        end_line: 1,
                        end_col: 10,
                    },
                    signature: None,
                    doc_comment: None,
                    attributes: vec![],
                    parent: None,
                });
            }
        }

        // Find by target name
        let found = state.find_by_name(&target_name);

        // All found symbols should have the target name
        for sym in &found {
            let name = state.resolve(sym.name);
            prop_assert_eq!(name, target_name.as_str());
        }

        // Should find at least one
        prop_assert!(!found.is_empty());
    }

    /// Property: Call edges are recorded correctly
    #[test]
    fn call_edges_recorded(
        caller_name in rust_identifier(),
        callee_name in rust_identifier()
    ) {
        let state = OciState::new(PathBuf::from("/test"));
        let caller_scoped = state.intern(&format!("crate::{}", caller_name));

        let edge = CallEdge {
            caller: caller_scoped,
            callee_name: callee_name.clone(),
            location: Location {
                file: PathBuf::from("test.rs"),
                start_byte: 0,
                end_byte: 10,
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 10,
            },
            is_method_call: false,
        };

        state.add_call_edge(edge);

        let callers = state.find_callers(&callee_name);
        prop_assert!(!callers.is_empty());
        prop_assert_eq!(&callers[0].callee_name, &callee_name);
    }
}

// ============================================================================
// Intervention Engine Property Tests (requires intervention feature)
// ============================================================================

#[cfg(feature = "intervention")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// Property: Similarity scores are always between 0 and 1
    #[test]
    fn similarity_scores_bounded(sig in function_signature()) {
        use omni_index::InterventionEngine;
        let state = OciState::new(PathBuf::from("/test"));

        // Add some symbols first
        for i in 0..3 {
            let name = format!("func_{}", i);
            let scoped = state.intern(&format!("crate::{}", name));
            let simple = state.intern(&name);

            state.add_symbol(SymbolDef {
                name: simple,
                scoped_name: scoped,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                location: Location {
                    file: PathBuf::from("test.rs"),
                    start_byte: 0,
                    end_byte: 10,
                    start_line: i + 1,
                    start_col: 0,
                    end_line: i + 1,
                    end_col: 10,
                },
                signature: Some(Signature {
                    params: vec!["x: i32".to_string()],
                    return_type: Some("bool".to_string()),
                    generics: None,
                    where_clause: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                }),
                doc_comment: None,
                attributes: vec![],
                parent: None,
            });
        }

        let matches = InterventionEngine::detect_duplication(&state, &sig);

        for m in matches {
            prop_assert!(m.score >= 0.0, "Score {} is negative", m.score);
            prop_assert!(m.score <= 1.0, "Score {} exceeds 1.0", m.score);
        }
    }

    /// Property: Empty state returns no duplicates
    #[test]
    fn empty_state_no_duplicates(sig in function_signature()) {
        use omni_index::InterventionEngine;
        let state = OciState::new(PathBuf::from("/test"));

        let matches = InterventionEngine::detect_duplication(&state, &sig);
        prop_assert!(matches.is_empty());
    }
}

// ============================================================================
// Topology Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// Property: PageRank scores are bounded between 0 and 1
    #[test]
    fn pagerank_scores_bounded(num_files in 2usize..10) {
        use tempfile::TempDir;
        use std::fs;

        let temp = TempDir::new().unwrap();

        // Create test files with some cross-references to create a connected graph
        for i in 0..num_files {
            let file = temp.path().join(format!("file_{}.rs", i));
            let next = (i + 1) % num_files;
            fs::write(&file, format!(
                "// file {}\nfn func_{}() {{ func_{}(); }}",
                i, i, next
            )).unwrap();
        }

        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();
        builder.build(&state, temp.path()).unwrap();

        // All scores should be in [0, 1]
        for entry in state.topology_metrics.iter() {
            let score = entry.value().relevance_score;
            prop_assert!(
                (0.0..=1.0).contains(&score),
                "PageRank score {} not in [0, 1]",
                score
            );
        }
    }

    /// Property: All PageRank scores are non-negative
    #[test]
    fn pagerank_non_negative(num_files in 1usize..8) {
        use tempfile::TempDir;
        use std::fs;

        let temp = TempDir::new().unwrap();

        for i in 0..num_files {
            let file = temp.path().join(format!("test_{}.rs", i));
            fs::write(&file, format!("fn f{}() {{}}", i)).unwrap();
        }

        let state = create_state(temp.path().to_path_buf());
        let builder = TopologyBuilder::new();
        builder.build(&state, temp.path()).unwrap();

        for entry in state.topology_metrics.iter() {
            prop_assert!(
                entry.value().relevance_score >= 0.0,
                "Negative PageRank score: {}",
                entry.value().relevance_score
            );
        }
    }
}

// ============================================================================
// Parser Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property: Parsed function has correct name
    #[test]
    fn parser_extracts_function_name(name in rust_identifier()) {
        use tree_sitter::Parser;
        use lasso::ThreadedRodeo;

        let source = format!("fn {}() {{}}", name);
        let interner = ThreadedRodeo::default();

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let symbols = rust_parser.extract_symbols(
            &tree,
            &source,
            &PathBuf::from("test.rs"),
            &interner
        ).unwrap();

        prop_assert!(!symbols.is_empty(), "No symbols extracted");

        let func = symbols.iter().find(|s| s.kind == SymbolKind::Function);
        prop_assert!(func.is_some(), "No function found");

        let func_name = interner.resolve(&func.unwrap().name);
        prop_assert_eq!(func_name, name);
    }

    /// Property: Parser handles all visibility modifiers
    #[test]
    fn parser_handles_visibility(
        name in rust_identifier(),
        vis in prop_oneof![
            Just(""),
            Just("pub "),
            Just("pub(crate) "),
            Just("pub(super) "),
        ]
    ) {
        use tree_sitter::Parser;
        use lasso::ThreadedRodeo;

        let source = format!("{}fn {}() {{}}", vis, name);
        let interner = ThreadedRodeo::default();

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let symbols = rust_parser.extract_symbols(
            &tree,
            &source,
            &PathBuf::from("test.rs"),
            &interner
        ).unwrap();

        // Should parse without error
        prop_assert!(!symbols.is_empty());
    }

    /// Property: Struct with fields parses correctly
    #[test]
    fn parser_handles_struct(name in rust_identifier(), field in rust_identifier()) {
        use tree_sitter::Parser;
        use lasso::ThreadedRodeo;

        let source = format!("pub struct {} {{ {}: i32 }}", name, field);
        let interner = ThreadedRodeo::default();

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let symbols = rust_parser.extract_symbols(
            &tree,
            &source,
            &PathBuf::from("test.rs"),
            &interner
        ).unwrap();

        prop_assert!(!symbols.is_empty());

        let struct_sym = symbols.iter().find(|s| s.kind == SymbolKind::Struct);
        prop_assert!(struct_sym.is_some(), "No struct found");

        let struct_name = interner.resolve(&struct_sym.unwrap().name);
        prop_assert_eq!(struct_name, name);
    }

    /// Property: Impl block methods are extracted
    #[test]
    fn parser_handles_impl(struct_name in rust_identifier(), method_name in rust_identifier()) {
        use tree_sitter::Parser;
        use lasso::ThreadedRodeo;

        let source = format!(
            "struct {} {{}}\nimpl {} {{ fn {}(&self) {{}} }}",
            struct_name, struct_name, method_name
        );
        let interner = ThreadedRodeo::default();

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let symbols = rust_parser.extract_symbols(
            &tree,
            &source,
            &PathBuf::from("test.rs"),
            &interner
        ).unwrap();

        prop_assert!(symbols.len() >= 2, "Expected struct and method");

        let method = symbols.iter().find(|s| s.kind == SymbolKind::Method);
        prop_assert!(method.is_some(), "No method found");

        let extracted_method_name = interner.resolve(&method.unwrap().name);
        prop_assert_eq!(extracted_method_name, method_name);
    }
}

// ============================================================================
// Dead Code Analysis Property Tests (requires analysis feature)
// ============================================================================

#[cfg(feature = "analysis")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// Property: Entry points are never reported as dead code
    #[test]
    fn entry_points_not_dead(
        pub_funcs in prop::collection::vec(rust_identifier(), 1..5)
    ) {
        let state = OciState::new(PathBuf::from("/test"));

        // Add public functions (entry points)
        for (i, name) in pub_funcs.iter().enumerate() {
            let scoped = state.intern(&format!("crate::pub_{}", name));
            let simple = state.intern(name);

            state.add_symbol(SymbolDef {
                name: simple,
                scoped_name: scoped,
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                location: Location {
                    file: PathBuf::from("lib.rs"),
                    start_byte: i * 20,
                    end_byte: i * 20 + 10,
                    start_line: i + 1,
                    start_col: 0,
                    end_line: i + 1,
                    end_col: 10,
                },
                signature: None,
                doc_comment: None,
                attributes: vec![],
                parent: None,
            });
        }

        let analyzer = omni_index::analysis::DeadCodeAnalyzer::new();
        let report = analyzer.analyze(&state);

        // Public functions should be entry points and not dead
        for name in &pub_funcs {
            let scoped = format!("crate::pub_{}", name);
            if let Some(key) = state.interner.get(&scoped) {
                prop_assert!(
                    report.entry_points.contains(&key),
                    "Public function {} not an entry point",
                    name
                );
                prop_assert!(
                    !report.dead_symbols.contains(&key),
                    "Public function {} reported as dead",
                    name
                );
            }
        }
    }

    /// Property: Private unreachable functions are marked dead
    #[test]
    fn private_unreachable_marked_dead(
        priv_funcs in prop::collection::vec(rust_identifier(), 1..5)
    ) {
        let state = OciState::new(PathBuf::from("/test"));

        // Add private functions (not entry points, not called)
        for (i, name) in priv_funcs.iter().enumerate() {
            let scoped = state.intern(&format!("crate::priv_{}", name));
            let simple = state.intern(name);

            state.add_symbol(SymbolDef {
                name: simple,
                scoped_name: scoped,
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                location: Location {
                    file: PathBuf::from("lib.rs"),
                    start_byte: i * 20,
                    end_byte: i * 20 + 10,
                    start_line: i + 1,
                    start_col: 0,
                    end_line: i + 1,
                    end_col: 10,
                },
                signature: None,
                doc_comment: None,
                attributes: vec![],
                parent: None,
            });
        }

        let analyzer = omni_index::analysis::DeadCodeAnalyzer::new();
        let report = analyzer.analyze(&state);

        // Private functions should be dead if not called
        for name in &priv_funcs {
            let scoped = format!("crate::priv_{}", name);
            if let Some(key) = state.interner.get(&scoped) {
                prop_assert!(
                    !report.entry_points.contains(&key),
                    "Private function {} is an entry point",
                    name
                );
                // Should be in dead_symbols or potentially_live
                let is_dead = report.dead_symbols.contains(&key);
                let is_potentially_live = report.potentially_live.contains(&key);
                prop_assert!(
                    is_dead || is_potentially_live,
                    "Private function {} not tracked",
                    name
                );
            }
        }
    }
}

// ============================================================================
// Import Extraction Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// Property: Use statements are extracted correctly
    #[test]
    fn parser_extracts_imports(module in rust_identifier(), item in rust_identifier()) {
        use tree_sitter::Parser;

        let source = format!("use {}::{};", module, item);

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let imports = rust_parser.extract_imports(
            &tree,
            &source,
            &PathBuf::from("test.rs")
        ).unwrap();

        prop_assert!(!imports.is_empty(), "No imports extracted");
        prop_assert!(imports.iter().any(|i| i.name == item), "Item not found in imports");
    }

    /// Property: Glob imports are marked correctly
    #[test]
    fn parser_marks_glob_imports(module in rust_identifier()) {
        use tree_sitter::Parser;

        let source = format!("use {}::*;", module);

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(&source, None).unwrap();
        let imports = rust_parser.extract_imports(
            &tree,
            &source,
            &PathBuf::from("test.rs")
        ).unwrap();

        prop_assert!(!imports.is_empty(), "No imports extracted");
        prop_assert!(imports.iter().any(|i| i.is_glob), "Glob import not detected");
    }
}
