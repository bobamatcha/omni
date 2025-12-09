//! Comparative benchmarks between OCI and code-index.
//!
//! Run with: cargo bench --bench comparative
//!
//! To compare against real code-index on a shared codebase:
//! 1. Set BENCHMARK_CODEBASE env var to a Rust repo path
//! 2. Run this benchmark
//! 3. Run code-index benchmarks on the same repo
//! 4. Compare results

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use omni_index::{
    DeadCodeAnalyzer, IncrementalIndexer, InterventionEngine, create_state,
    topology::TopologyBuilder,
};
use std::fs;
use tempfile::TempDir;

// ============================================================================
// Shared Test Fixture (matching code-index benchmark patterns)
// ============================================================================

/// Create a test fixture matching code-index's benchmark structure.
/// This allows direct comparison of results.
fn create_benchmark_fixture(num_files: usize) -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // Create lib.rs with module declarations
    let mut lib = String::from("//! Benchmark fixture\n\n");
    for i in 0..num_files {
        lib.push_str(&format!("pub mod module_{};\n", i));
    }
    fs::write(src.join("lib.rs"), lib).unwrap();

    // Create module files with realistic patterns
    for i in 0..num_files {
        let content = generate_module_content(i, num_files);
        fs::write(src.join(format!("module_{}.rs", i)), content).unwrap();
    }

    temp
}

/// Generate module content with imports, structs, impl blocks, functions, calls, and tests.
/// Mirrors what code-index benchmarks use.
fn generate_module_content(module_idx: usize, total_modules: usize) -> String {
    let mut code = String::with_capacity(4000);

    // Module doc
    code.push_str(&format!(
        "//! Module {} for benchmark testing.\n\n",
        module_idx
    ));

    // Imports (some cross-module)
    code.push_str("use std::collections::HashMap;\n");
    if module_idx > 0 {
        code.push_str(&format!(
            "use crate::module_{}::helper_fn;\n",
            module_idx - 1
        ));
    }
    code.push('\n');

    // Struct with impl
    code.push_str(&format!(
        r#"/// Main struct for module {idx}.
#[derive(Debug, Clone)]
pub struct Module{idx}State {{
    pub data: HashMap<String, i32>,
    pub counter: usize,
}}

impl Module{idx}State {{
    /// Create new state.
    pub fn new() -> Self {{
        Self {{
            data: HashMap::new(),
            counter: 0,
        }}
    }}

    /// Process some data.
    pub fn process(&mut self, key: &str, value: i32) -> i32 {{
        self.counter += 1;
        self.data.insert(key.to_string(), value);
        helper_fn(value)
    }}

    /// Get a value.
    pub fn get(&self, key: &str) -> Option<i32> {{
        self.data.get(key).copied()
    }}
}}

"#,
        idx = module_idx
    ));

    // Helper function (called by process)
    code.push_str(&format!(
        r#"/// Helper function for module {idx}.
pub fn helper_fn(x: i32) -> i32 {{
    transform(x) * 2
}}

/// Transform function.
fn transform(x: i32) -> i32 {{
    x.saturating_add(1)
}}

"#,
        idx = module_idx
    ));

    // Cross-module call (if not first module)
    if module_idx < total_modules - 1 {
        code.push_str(&format!(
            r#"/// Cross-module function.
pub fn call_next() -> i32 {{
    // Would call module_{next}::helper_fn in real code
    42
}}

"#,
            next = module_idx + 1
        ));
    }

    // Test functions
    code.push_str(&format!(
        r#"#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_new() {{
        let state = Module{idx}State::new();
        assert_eq!(state.counter, 0);
    }}

    #[test]
    fn test_process() {{
        let mut state = Module{idx}State::new();
        let result = state.process("key", 42);
        assert!(result > 0);
    }}

    #[test]
    fn test_get() {{
        let mut state = Module{idx}State::new();
        state.process("key", 42);
        assert_eq!(state.get("key"), Some(42));
    }}

    #[test]
    fn test_helper() {{
        assert_eq!(helper_fn(10), 22);
    }}
}}
"#,
        idx = module_idx
    ));

    code
}

// ============================================================================
// Benchmarks matching code-index's benchmark categories
// ============================================================================

/// Benchmark: Index build time
/// Matches code-index's build_index benchmark
fn bench_build_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/build_index");
    group.sample_size(10);

    // Test different sizes (matching code-index patterns)
    for num_files in [10, 25, 50, 100] {
        let temp = create_benchmark_fixture(num_files);
        let label = format!("{}_files", num_files);

        // Count total source bytes
        let total_bytes: usize = walkdir::WalkDir::new(temp.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
            .map(|e| {
                fs::metadata(e.path())
                    .map(|m| m.len() as usize)
                    .unwrap_or(0)
            })
            .sum();

        group.throughput(Throughput::Bytes(total_bytes as u64));

        group.bench_with_input(BenchmarkId::new("oci", &label), &temp, |b, temp| {
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
    }

    group.finish();
}

/// Benchmark: Find function definitions
/// Matches code-index's defs() query
fn bench_find_defs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/find_defs");

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Find by simple name
    group.bench_function("by_simple_name", |b| {
        b.iter(|| {
            let results = state.find_by_name("helper_fn");
            black_box(results.len())
        });
    });

    // Find by common name (many matches)
    group.bench_function("by_common_name", |b| {
        b.iter(|| {
            let results = state.find_by_name("new");
            black_box(results.len())
        });
    });

    // Iterate all to find by prefix
    group.bench_function("by_prefix_scan", |b| {
        b.iter(|| {
            let results: Vec<_> = state
                .symbols
                .iter()
                .filter(|e| {
                    let name = state.resolve(e.value().name);
                    name.starts_with("test_")
                })
                .collect();
            black_box(results.len())
        });
    });

    group.finish();
}

/// Benchmark: Find callers
/// Matches code-index's find_calls() query
fn bench_find_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/find_calls");

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Find callers of a function
    group.bench_function("find_callers", |b| {
        b.iter(|| {
            let results = state.find_callers("helper_fn");
            black_box(results.len())
        });
    });

    // Find callers of a method
    group.bench_function("find_callers_method", |b| {
        b.iter(|| {
            let results = state.find_callers("process");
            black_box(results.len())
        });
    });

    group.finish();
}

/// Benchmark: Find tests
/// Matches code-index's find_tests() query
fn bench_find_tests(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/find_tests");

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    group.bench_function("find_all_tests", |b| {
        b.iter(|| {
            let tests: Vec<_> = state
                .symbols
                .iter()
                .filter(|e| {
                    let name = state.resolve(e.value().name);
                    name.starts_with("test_")
                })
                .collect();
            black_box(tests.len())
        });
    });

    group.finish();
}

/// Benchmark: Functions per file
/// Matches code-index's functions_of_file() query
fn bench_functions_of_file(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/functions_of_file");

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let test_file = temp.path().join("src/module_25.rs");
    let file_id = state.get_or_create_file_id(&test_file);

    group.bench_function("get_file_symbols", |b| {
        b.iter(|| {
            let symbols = state.file_symbols.get(&file_id);
            black_box(symbols.map(|s| s.len()))
        });
    });

    group.finish();
}

/// Benchmark: Additional OCI features (not in code-index)
fn bench_oci_unique_features(c: &mut Criterion) {
    let mut group = c.benchmark_group("oci_unique");
    group.sample_size(20);

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    // Dead code analysis (not in code-index)
    group.bench_function("dead_code_analysis", |b| {
        b.iter(|| {
            let analyzer = DeadCodeAnalyzer::new();
            let report = analyzer.analyze(&state);
            black_box(report.dead_symbols.len())
        });
    });

    // Intervention engine - suggest alternatives
    group.bench_function("suggest_alternatives", |b| {
        b.iter(|| {
            let results = InterventionEngine::suggest_alternatives(&state, "helper");
            black_box(results.len())
        });
    });

    // Intervention engine - naming conflicts
    let test_file = temp.path().join("src/module_0.rs");
    group.bench_function("check_naming_conflicts", |b| {
        b.iter(|| {
            let results =
                InterventionEngine::check_naming_conflicts(&state, "helper_fn", &test_file);
            black_box(results.len())
        });
    });

    // Topology building
    group.bench_function("build_topology", |b| {
        b.iter(|| {
            state.topology_metrics.clear();
            let builder = TopologyBuilder::new();
            builder.build(&state, temp.path()).unwrap();
            black_box(state.topology_metrics.len())
        });
    });

    group.finish();
}

/// Benchmark: Incremental updates (critical for IDE use)
fn bench_incremental(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparative/incremental");
    group.sample_size(30);

    let temp = create_benchmark_fixture(50);
    let state = create_state(temp.path().to_path_buf());
    let indexer = IncrementalIndexer::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async { indexer.full_index(&state, temp.path()).await.unwrap() });

    let update_file = temp.path().join("src/module_25.rs");

    // Single file update
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
    let remove_file = temp.path().join("src/module_49.rs");
    group.bench_function("remove_file", |b| {
        b.iter(|| {
            indexer.remove_file(&state, &remove_file);
            black_box(())
        });
    });

    group.finish();
}

// ============================================================================
// Criterion Groups
// ============================================================================

criterion_group!(
    name = build_benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(5));
    targets = bench_build_index
);

criterion_group!(
    name = query_benches;
    config = Criterion::default()
        .sample_size(100)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_find_defs, bench_find_calls, bench_find_tests, bench_functions_of_file
);

criterion_group!(
    name = unique_benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_oci_unique_features
);

criterion_group!(
    name = incremental_benches;
    config = Criterion::default()
        .sample_size(30)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_incremental
);

criterion_main!(
    build_benches,
    query_benches,
    unique_benches,
    incremental_benches
);
