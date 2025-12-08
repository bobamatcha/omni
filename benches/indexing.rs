//! Comprehensive benchmarks for OCI indexing performance.
//!
//! Measures key metrics for an AI code indexer:
//!
//! ## Parsing Performance
//! - Single file parsing latency vs raw tree-sitter
//! - Symbol extraction overhead
//! - Call graph extraction overhead
//!
//! ## Full Indexing
//! - Repository indexing throughput (files/sec, symbols/sec)
//! - Scaling behavior with repo size
//!
//! ## Incremental Updates
//! - Single file update latency (critical for IDE-like responsiveness)
//! - File removal performance
//!
//! ## Query Performance
//! - Symbol lookup by name (exact match)
//! - Caller/callee resolution
//! - Full symbol iteration
//!
//! ## Analysis Performance
//! - Dead code analysis scaling
//! - Intervention engine response time
//!
//! ## Memory & Infrastructure
//! - String interning performance
//! - State creation overhead
//! - Topology (PageRank) computation

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use omni_index::{
    DeadCodeAnalyzer, FileDiscovery, FileId, IncrementalIndexer, InterventionEngine, OciState,
    create_state, parsing::LanguageParser, parsing::rust::RustParser, topology::TopologyBuilder,
};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use tree_sitter::Parser;

// ============================================================================
// Test Fixture Generation
// ============================================================================

/// Generate a realistic Rust source file with specified complexity.
/// Returns (code, estimated_symbols) where symbols = funcs + structs + impl_methods
fn generate_rust_file(
    num_functions: usize,
    num_structs: usize,
    num_impls: usize,
    lines_per_fn: usize,
) -> (String, usize) {
    let mut code = String::with_capacity(num_functions * lines_per_fn * 50);

    // Module doc
    code.push_str("//! Generated benchmark module.\n\n");

    // Imports
    code.push_str("use std::collections::HashMap;\n");
    code.push_str("use std::sync::Arc;\n\n");

    // Generate structs
    for i in 0..num_structs {
        code.push_str(&format!(
            r#"/// Struct documentation for BenchStruct{}
#[derive(Debug, Clone)]
pub struct BenchStruct{} {{
    /// Field documentation
    pub field_{}: i32,
    pub data_{}: String,
    pub items_{}: Vec<u64>,
}}

"#,
            i, i, i, i, i
        ));
    }

    // Generate impl blocks (3 methods each)
    let impl_methods = num_impls.min(num_structs) * 3;
    for i in 0..num_impls.min(num_structs) {
        code.push_str(&format!(
            r#"impl BenchStruct{} {{
    /// Creates a new instance
    pub fn new() -> Self {{
        Self {{
            field_{}: 0,
            data_{}: String::new(),
            items_{}: Vec::new(),
        }}
    }}

    /// Gets the field value
    pub fn get_field(&self) -> i32 {{
        self.field_{}
    }}

    /// Sets the field value
    pub fn set_field(&mut self, value: i32) {{
        self.field_{} = value;
    }}
}}

"#,
            i, i, i, i, i, i
        ));
    }

    // Generate free functions
    for i in 0..num_functions {
        code.push_str(&format!(
            r#"/// Function documentation for bench_function_{}
///
/// # Arguments
/// * `input` - The input value
///
/// # Returns
/// The processed result
pub fn bench_function_{}(input: i32) -> i32 {{
"#,
            i, i
        ));

        // Add function body lines
        for j in 0..lines_per_fn {
            if j == 0 {
                code.push_str("    let mut result = input;\n");
            } else if j == lines_per_fn - 1 {
                code.push_str("    result\n");
            } else {
                code.push_str(&format!("    result = result.wrapping_add({});\n", j % 100));
            }
        }
        code.push_str("}\n\n");
    }

    // Add cross-reference function (generates call edges)
    code.push_str(
        r#"/// Integration function that calls other functions
pub fn integration_test() -> i32 {
    let mut sum = 0;
"#,
    );
    for i in 0..num_functions.min(10) {
        code.push_str(&format!("    sum += bench_function_{}(sum);\n", i));
    }
    code.push_str("    sum\n}\n");

    let total_symbols = num_functions + num_structs + impl_methods + 1; // +1 for integration_test
    (code, total_symbols)
}

/// Create a test repository with multiple files
fn create_test_repo(num_files: usize, symbols_per_file: usize) -> (TempDir, usize) {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // Create lib.rs
    let mut lib_content = String::from("//! Benchmark test library\n\n");
    for i in 0..num_files {
        lib_content.push_str(&format!("pub mod module_{};\n", i));
    }
    fs::write(src.join("lib.rs"), lib_content).unwrap();

    let mut total_symbols = 0;

    // Create module files
    for i in 0..num_files {
        let funcs = symbols_per_file / 3;
        let structs = symbols_per_file / 6;
        let impls = structs;
        let (code, syms) = generate_rust_file(funcs, structs, impls, 10);
        total_symbols += syms;
        fs::write(src.join(format!("module_{}.rs", i)), code).unwrap();
    }

    (temp, total_symbols)
}

// ============================================================================
// Parsing Benchmarks - Single File Performance
// ============================================================================

fn bench_single_file_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsing/single_file");

    // Different file sizes
    let file_configs = [
        (10, 5, 5, 10, "small"),     // ~500 lines, ~25 symbols
        (50, 20, 20, 15, "medium"),  // ~2500 lines, ~120 symbols
        (100, 40, 40, 20, "large"),  // ~6000 lines, ~260 symbols
        (200, 80, 80, 25, "xlarge"), // ~15000 lines, ~520 symbols
    ];

    for (funcs, structs, impls, lines, label) in file_configs {
        let (code, symbol_count) = generate_rust_file(funcs, structs, impls, lines);
        let code_bytes = code.len();
        let code_lines = code.lines().count();

        group.throughput(Throughput::Bytes(code_bytes as u64));

        // Baseline: raw tree-sitter parsing only
        group.bench_with_input(
            BenchmarkId::new("tree_sitter_parse", label),
            &code,
            |b, code| {
                let mut parser = Parser::new();
                parser
                    .set_language(&tree_sitter_rust::LANGUAGE.into())
                    .unwrap();

                b.iter(|| {
                    let tree = parser.parse(code, None).unwrap();
                    black_box(tree.root_node().child_count())
                });
            },
        );

        // tree-sitter parse + full tree walk
        group.bench_with_input(
            BenchmarkId::new("tree_sitter_walk", label),
            &code,
            |b, code| {
                let mut parser = Parser::new();
                parser
                    .set_language(&tree_sitter_rust::LANGUAGE.into())
                    .unwrap();

                b.iter(|| {
                    let tree = parser.parse(code, None).unwrap();
                    let mut cursor = tree.walk();
                    let mut count = 0usize;

                    loop {
                        count += 1;
                        if cursor.goto_first_child() {
                            continue;
                        }
                        while !cursor.goto_next_sibling() {
                            if !cursor.goto_parent() {
                                return black_box(count);
                            }
                        }
                    }
                });
            },
        );

        // OCI: full symbol extraction
        group.bench_with_input(
            BenchmarkId::new("oci_extract_symbols", label),
            &code,
            |b, code| {
                let state = OciState::new(PathBuf::from("/bench"));
                let rust_parser = RustParser::new();
                let mut ts_parser = Parser::new();
                ts_parser.set_language(&rust_parser.language()).unwrap();

                b.iter(|| {
                    let tree = ts_parser.parse(code, None).unwrap();
                    let symbols = rust_parser
                        .extract_symbols(
                            &tree,
                            code,
                            &PathBuf::from("/bench/test.rs"),
                            &state.interner,
                        )
                        .unwrap();
                    black_box(symbols.len())
                });
            },
        );

        // OCI: full extraction (symbols + calls + imports)
        group.bench_with_input(
            BenchmarkId::new("oci_full_extraction", label),
            &code,
            |b, code| {
                let state = OciState::new(PathBuf::from("/bench"));
                let rust_parser = RustParser::new();
                let mut ts_parser = Parser::new();
                ts_parser.set_language(&rust_parser.language()).unwrap();

                b.iter(|| {
                    let tree = ts_parser.parse(code, None).unwrap();
                    let symbols = rust_parser
                        .extract_symbols(
                            &tree,
                            code,
                            &PathBuf::from("/bench/test.rs"),
                            &state.interner,
                        )
                        .unwrap();
                    let calls = rust_parser
                        .extract_calls(
                            &tree,
                            code,
                            &PathBuf::from("/bench/test.rs"),
                            &state.interner,
                        )
                        .unwrap();
                    let imports = rust_parser
                        .extract_imports(&tree, code, &PathBuf::from("/bench/test.rs"))
                        .unwrap();
                    black_box((symbols.len(), calls.len(), imports.len()))
                });
            },
        );

        // Print file stats for reference
        println!(
            "\n  {} file: {} bytes, {} lines, ~{} symbols",
            label, code_bytes, code_lines, symbol_count
        );
    }

    group.finish();
}

// ============================================================================
// Full Index Benchmarks - Repository Throughput
// ============================================================================

fn bench_full_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/full_repo");
    group.sample_size(10);

    // Different repository sizes
    let repo_configs = [
        (5, 30, "tiny"),      // ~150 symbols
        (10, 50, "small"),    // ~500 symbols
        (25, 80, "medium"),   // ~2000 symbols
        (50, 100, "large"),   // ~5000 symbols
        (100, 100, "xlarge"), // ~10000 symbols
    ];

    for (files, symbols_per_file, label) in repo_configs {
        let (temp, total_symbols) = create_test_repo(files, symbols_per_file);

        // Count total bytes
        let mut total_bytes = 0u64;
        for entry in walkdir::WalkDir::new(temp.path().join("src"))
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
        {
            total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }

        group.throughput(Throughput::Elements(total_symbols as u64));

        group.bench_with_input(BenchmarkId::new("oci", label), &temp, |b, temp| {
            b.iter(|| {
                let state = create_state(temp.path().to_path_buf());
                let indexer = IncrementalIndexer::new();

                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

                black_box(state.stats())
            });
        });

        println!(
            "\n  {} repo: {} files, {} KB, ~{} symbols",
            label,
            files + 1,
            total_bytes / 1024,
            total_symbols
        );
    }

    group.finish();
}

// ============================================================================
// Incremental Update Benchmarks - IDE Responsiveness
// ============================================================================

fn bench_incremental_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/incremental");
    group.sample_size(50);

    // Setup: create and index a medium-sized repo
    let (temp, _) = create_test_repo(20, 50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let update_file = temp.path().join("src/module_5.rs");
    let remove_file = temp.path().join("src/module_10.rs");

    // Single file update (most common IDE operation)
    group.bench_function("update_single_file", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async { indexer.update_file(&state, &update_file).await.unwrap() });
            black_box(())
        });
    });

    // File removal
    group.bench_function("remove_file", |b| {
        b.iter(|| {
            indexer.remove_file(&state, &remove_file);
            black_box(())
        });
    });

    group.finish();
}

// ============================================================================
// Query Performance Benchmarks
// ============================================================================

fn bench_symbol_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/symbol_lookup");

    // Setup indexed state
    let (temp, _) = create_test_repo(30, 100);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let stats = state.stats();
    println!(
        "\n  Indexed: {} files, {} symbols, {} call edges",
        stats.file_count, stats.symbol_count, stats.call_edge_count
    );

    // Exact name lookup (best case)
    group.bench_function("find_by_name_exact", |b| {
        b.iter(|| {
            let results = state.find_by_name("bench_function_5");
            black_box(results.len())
        });
    });

    // Common name lookup (matches many)
    group.bench_function("find_by_name_common", |b| {
        b.iter(|| {
            let results = state.find_by_name("new");
            black_box(results.len())
        });
    });

    // Find callers of a function
    group.bench_function("find_callers", |b| {
        b.iter(|| {
            let results = state.find_callers("bench_function_0");
            black_box(results.len())
        });
    });

    // Iterate all symbols
    group.bench_function("iterate_all_symbols", |b| {
        b.iter(|| {
            let count: usize = state.symbols.iter().count();
            black_box(count)
        });
    });

    // File symbols lookup
    group.bench_function("get_file_symbols", |b| {
        let file_id = state
            .file_ids
            .iter()
            .next()
            .map(|e| *e.value())
            .unwrap_or(FileId(0));
        b.iter(|| {
            let symbols = state.file_symbols.get(&file_id);
            black_box(symbols.map(|s| s.len()))
        });
    });

    group.finish();
}

// ============================================================================
// Topology Benchmarks - PageRank Computation
// ============================================================================

fn bench_topology(c: &mut Criterion) {
    let mut group = c.benchmark_group("topology/pagerank");
    group.sample_size(20);

    for files in [10, 30, 50, 100] {
        let (temp, _) = create_test_repo(files, 50);
        let state = create_state(temp.path().to_path_buf());
        let indexer = IncrementalIndexer::new();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // Index files without topology
        rt.block_on(async {
            let discovery = FileDiscovery::new();
            let files = discovery.discover(temp.path()).unwrap();
            for file in &files {
                indexer.index_file(&state, file).await.ok();
            }
        });

        let builder = TopologyBuilder::new();
        let label = format!("{}files", files);

        group.bench_with_input(BenchmarkId::new("build", &label), &state, |b, state| {
            b.iter(|| {
                state.topology_metrics.clear();
                builder.build(state, temp.path()).unwrap();
                black_box(state.topology_metrics.len())
            });
        });
    }

    group.finish();
}

// ============================================================================
// Analysis Benchmarks
// ============================================================================

fn bench_dead_code_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("analysis/dead_code");
    group.sample_size(20);

    for files in [10, 25, 50] {
        let (temp, total_symbols) = create_test_repo(files, 50);
        let state = create_state(temp.path().to_path_buf());
        let indexer = IncrementalIndexer::new();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

        let label = format!("{}files_{}sym", files, total_symbols);

        group.bench_with_input(BenchmarkId::new("analyze", &label), &state, |b, state| {
            b.iter(|| {
                let analyzer = DeadCodeAnalyzer::new();
                let report = analyzer.analyze(state);
                black_box(report.dead_symbols.len())
            });
        });
    }

    group.finish();
}

fn bench_intervention_engine(c: &mut Criterion) {
    let mut group = c.benchmark_group("analysis/intervention");
    group.sample_size(50);

    let (temp, _) = create_test_repo(30, 80);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let test_file = temp.path().join("src/module_0.rs");

    // Suggest alternatives (semantic duplicate detection)
    group.bench_function("suggest_alternatives", |b| {
        b.iter(|| {
            let results = InterventionEngine::suggest_alternatives(&state, "process_data");
            black_box(results.len())
        });
    });

    // Check naming conflicts
    group.bench_function("check_naming_conflicts", |b| {
        b.iter(|| {
            let results = InterventionEngine::check_naming_conflicts(
                &state,
                "bench_function_new",
                &test_file,
            );
            black_box(results.len())
        });
    });

    // Similar name check (Levenshtein)
    group.bench_function("similar_names_check", |b| {
        b.iter(|| {
            // Typo in name to test fuzzy matching
            let results =
                InterventionEngine::check_naming_conflicts(&state, "bench_functon_5", &test_file);
            black_box(results.len())
        });
    });

    group.finish();
}

// ============================================================================
// String Interning Benchmarks
// ============================================================================

fn bench_string_interning(c: &mut Criterion) {
    let mut group = c.benchmark_group("infrastructure/interning");

    let state = OciState::new(PathBuf::from("/bench"));

    // Pre-populate with symbols
    for i in 0..10000 {
        state.intern(&format!("symbol_{}", i));
    }

    // Intern new string
    group.bench_function("intern_new", |b| {
        let mut counter = 10000u64;
        b.iter(|| {
            counter += 1;
            let key = state.intern(&format!("new_symbol_{}", counter));
            black_box(key)
        });
    });

    // Intern existing string (cache hit)
    group.bench_function("intern_existing", |b| {
        b.iter(|| {
            let key = state.intern("symbol_5000");
            black_box(key)
        });
    });

    // Resolve interned string
    group.bench_function("resolve", |b| {
        let key = state.intern("symbol_5000");
        b.iter(|| {
            let s = state.resolve(key);
            black_box(s)
        });
    });

    group.finish();
}

// ============================================================================
// Memory/State Benchmarks
// ============================================================================

fn bench_state_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("infrastructure/state");
    group.sample_size(10);

    for files in [10, 25, 50] {
        let (temp, _) = create_test_repo(files, 50);
        let label = format!("{}files", files);

        // State creation + full index time
        group.bench_with_input(
            BenchmarkId::new("create_and_index", &label),
            &temp,
            |b, temp| {
                b.iter(|| {
                    let state = create_state(temp.path().to_path_buf());
                    let indexer = IncrementalIndexer::new();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });
                    black_box(state.stats())
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// File Discovery Benchmarks
// ============================================================================

fn bench_file_discovery(c: &mut Criterion) {
    let mut group = c.benchmark_group("infrastructure/discovery");

    for files in [20, 50, 100] {
        let (temp, _) = create_test_repo(files, 30);
        let label = format!("{}files", files);

        group.bench_with_input(BenchmarkId::new("discover", &label), &temp, |b, temp| {
            b.iter(|| {
                let discovery = FileDiscovery::new();
                let files = discovery.discover(temp.path()).unwrap();
                black_box(files.len())
            });
        });
    }

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    name = parsing_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.02)
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_single_file_parsing
);

criterion_group!(
    name = indexing_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(5));
    targets = bench_full_index, bench_incremental_update
);

criterion_group!(
    name = query_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_symbol_lookup
);

criterion_group!(
    name = topology_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .sample_size(20)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_topology
);

criterion_group!(
    name = analysis_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .sample_size(30)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_dead_code_analysis, bench_intervention_engine
);

criterion_group!(
    name = infra_benches;
    config = Criterion::default()
        .significance_level(0.05)
        .warm_up_time(std::time::Duration::from_millis(200))
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_string_interning, bench_state_operations, bench_file_discovery
);

criterion_main!(
    parsing_benches,
    indexing_benches,
    query_benches,
    topology_benches,
    analysis_benches,
    infra_benches
);
