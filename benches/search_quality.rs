//! Benchmark comparing search quality: BM25 vs Semantic vs Hybrid.
//!
//! Tests the hypothesis that:
//! 1. Embeddings fix BM25's synonym blindness
//! 2. BM25 protects against junk semantic matches
//! 3. Hybrid combining both performs best

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use omni_index::search::{
    Bm25Index, Bm25Params, FieldWeights, FoundBy, HybridSearch, extract_doc_comments,
    extract_identifiers, extract_string_literals, path_tokens,
};
use omni_index::types::InternedString;
use std::collections::HashMap;

/// Test case for search quality evaluation.
struct SearchTestCase {
    name: &'static str,
    query: &'static str,
    /// Expected relevant results (symbol names).
    relevant: Vec<&'static str>,
    /// This tests a specific weakness.
    tests: SearchWeakness,
}

#[derive(Debug, Clone, Copy)]
enum SearchWeakness {
    /// BM25 fails on synonyms (semantic should win).
    Synonyms,
    /// Both methods work well (hybrid reinforces).
    BothGood,
    /// Partial match query (tests tokenization).
    PartialMatch,
}

/// Create a test corpus with known symbols.
fn create_test_corpus() -> (lasso::ThreadedRodeo, Vec<(InternedString, String, String)>) {
    let interner = lasso::ThreadedRodeo::new();

    // Each entry: (symbol, path, code)
    let entries = vec![
        (
            "add_numbers",
            "src/math/arithmetic.rs",
            r#"
/// Adds two numbers together and returns the sum.
pub fn add_numbers(a: i32, b: i32) -> i32 {
    a + b
}
"#,
        ),
        (
            "sum_values",
            "src/math/aggregate.rs",
            r#"
/// Computes the sum of a slice of values.
/// This is an aggregation function.
pub fn sum_values(values: &[i32]) -> i32 {
    values.iter().sum()
}
"#,
        ),
        (
            "calculate_total",
            "src/billing/invoice.rs",
            r#"
/// Calculate the total amount for an invoice.
/// Adds up all line items and applies tax.
pub fn calculate_total(items: &[LineItem], tax_rate: f64) -> f64 {
    let subtotal: f64 = items.iter().map(|i| i.price).sum();
    subtotal * (1.0 + tax_rate)
}
"#,
        ),
        (
            "subtract_numbers",
            "src/math/arithmetic.rs",
            r#"
/// Subtracts the second number from the first.
pub fn subtract_numbers(a: i32, b: i32) -> i32 {
    a - b
}
"#,
        ),
        (
            "multiply",
            "src/math/arithmetic.rs",
            r#"
/// Multiplies two numbers.
pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
        ),
        (
            "divide",
            "src/math/arithmetic.rs",
            r#"
/// Divides the first number by the second.
/// Returns None if divisor is zero.
pub fn divide(a: i32, b: i32) -> Option<i32> {
    if b == 0 { None } else { Some(a / b) }
}
"#,
        ),
        (
            "parse_config",
            "src/config/parser.rs",
            r#"
/// Parse a configuration file from a string.
pub fn parse_config(input: &str) -> Result<Config, ParseError> {
    serde_json::from_str(input).map_err(ParseError::Json)
}
"#,
        ),
        (
            "load_settings",
            "src/config/loader.rs",
            r#"
/// Load application settings from disk.
/// Reads the configuration file and parses it.
pub fn load_settings(path: &Path) -> Result<Settings, Error> {
    let content = std::fs::read_to_string(path)?;
    parse_config(&content)
}
"#,
        ),
        (
            "send_request",
            "src/http/client.rs",
            r#"
/// Send an HTTP request to a remote server.
pub async fn send_request(url: &str, method: Method) -> Result<Response, HttpError> {
    let client = reqwest::Client::new();
    client.request(method, url).send().await.map_err(HttpError::from)
}
"#,
        ),
        (
            "fetch_data",
            "src/api/fetcher.rs",
            r#"
/// Fetch data from the API endpoint.
/// Makes an HTTP GET request and deserializes the response.
pub async fn fetch_data<T: DeserializeOwned>(endpoint: &str) -> Result<T, ApiError> {
    let response = send_request(endpoint, Method::GET).await?;
    response.json().await.map_err(ApiError::Deserialize)
}
"#,
        ),
        (
            "process_batch",
            "src/batch/processor.rs",
            r#"
/// Process a batch of items in parallel.
pub fn process_batch<T, F>(items: Vec<T>, f: F) -> Vec<T::Output>
where
    T: Send,
    F: Fn(T) -> T::Output + Send + Sync,
{
    items.into_par_iter().map(f).collect()
}
"#,
        ),
        (
            "validate_input",
            "src/validation/input.rs",
            r#"
/// Validate user input according to schema rules.
pub fn validate_input(input: &str, schema: &Schema) -> ValidationResult {
    schema.validate(input)
}
"#,
        ),
        // Add some noise/decoy functions
        (
            "log_message",
            "src/logging/logger.rs",
            r#"
/// Log a message at the specified level.
pub fn log_message(level: Level, msg: &str) {
    println!("[{}] {}", level, msg);
}
"#,
        ),
        (
            "format_string",
            "src/utils/format.rs",
            r#"
/// Format a string with placeholders.
pub fn format_string(template: &str, args: &[&str]) -> String {
    // Simple placeholder replacement
    template.to_string()
}
"#,
        ),
        (
            "hash_password",
            "src/auth/password.rs",
            r#"
/// Hash a password using bcrypt.
pub fn hash_password(password: &str) -> Result<String, HashError> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST).map_err(HashError::from)
}
"#,
        ),
    ];

    let mut symbols = Vec::new();
    for (name, path, code) in entries {
        let sym = InternedString::from(interner.get_or_intern(name));
        symbols.push((sym, path.to_string(), code.to_string()));
    }

    (interner, symbols)
}

/// Build a BM25 index from the test corpus.
fn build_bm25_index(
    _interner: &lasso::ThreadedRodeo,
    symbols: &[(InternedString, String, String)],
) -> Bm25Index {
    let mut index = Bm25Index::new();

    for (sym, path, code) in symbols {
        let path_toks = path_tokens(std::path::Path::new(path));
        let ident_toks = extract_identifiers(code);
        let doc_comments = extract_doc_comments(code);
        let doc_toks: Vec<&str> = doc_comments
            .iter()
            .flat_map(|s| s.split_whitespace())
            .collect();
        let string_toks = extract_string_literals(code);

        index.add_document(
            *sym,
            path_toks.iter().map(|s| s.as_str()),
            ident_toks,
            doc_toks,
            string_toks.iter().map(|s| s.as_str()),
            code,
        );
    }

    index.finalize();
    index
}

/// Simulate semantic search results (in real usage, would come from embedding model).
/// Returns symbols with cosine similarity scores.
fn simulate_semantic_search(
    query: &str,
    interner: &lasso::ThreadedRodeo,
    top_k: usize,
) -> Vec<(InternedString, f32)> {
    // Simulate semantic understanding with hand-crafted relevance
    // In production, this would be actual embedding similarity

    let semantic_map: HashMap<&str, Vec<(&str, f32)>> = [
        // Synonym test: "addition" should find sum/add/total
        (
            "addition",
            vec![
                ("add_numbers", 0.92),
                ("sum_values", 0.88),
                ("calculate_total", 0.75),
                ("subtract_numbers", 0.20), // Low - opposite
            ],
        ),
        // "sum" query
        (
            "sum",
            vec![
                ("sum_values", 0.95),
                ("add_numbers", 0.80),
                ("calculate_total", 0.70),
            ],
        ),
        // HTTP/network query with semantic understanding
        (
            "make network call",
            vec![
                ("send_request", 0.90),
                ("fetch_data", 0.85),
                ("log_message", 0.15), // Semantic noise
            ],
        ),
        // Configuration query
        (
            "read config file",
            vec![
                ("load_settings", 0.92),
                ("parse_config", 0.88),
                ("format_string", 0.12), // Noise
            ],
        ),
        // "compute total" - tests synonym matching
        (
            "compute total",
            vec![
                ("calculate_total", 0.93),
                ("sum_values", 0.80),
                ("add_numbers", 0.65),
                ("process_batch", 0.15), // Semantic junk
            ],
        ),
        // Partial/fuzzy match
        (
            "arithmetic",
            vec![
                ("add_numbers", 0.85),
                ("subtract_numbers", 0.85),
                ("multiply", 0.85),
                ("divide", 0.85),
            ],
        ),
        // Security/auth query
        (
            "secure password",
            vec![
                ("hash_password", 0.95),
                ("validate_input", 0.30), // Somewhat related
            ],
        ),
    ]
    .into_iter()
    .collect();

    // Find best matching query pattern
    let query_lower = query.to_lowercase();
    for (pattern, results) in &semantic_map {
        if query_lower.contains(pattern) || pattern.contains(&query_lower) {
            return results
                .iter()
                .filter_map(|(name, score)| {
                    interner
                        .get(*name)
                        .map(|spur| (InternedString::from(spur), *score))
                })
                .take(top_k)
                .collect();
        }
    }

    // Fallback: no matches
    Vec::new()
}

/// Define test cases.
fn test_cases() -> Vec<SearchTestCase> {
    vec![
        SearchTestCase {
            name: "synonym_addition",
            query: "addition",
            relevant: vec!["add_numbers", "sum_values", "calculate_total"],
            tests: SearchWeakness::Synonyms,
        },
        SearchTestCase {
            name: "synonym_total",
            query: "compute total",
            relevant: vec!["calculate_total", "sum_values"],
            tests: SearchWeakness::Synonyms,
        },
        SearchTestCase {
            name: "network_call",
            query: "make network call",
            relevant: vec!["send_request", "fetch_data"],
            tests: SearchWeakness::Synonyms,
        },
        SearchTestCase {
            name: "exact_match",
            query: "add_numbers",
            relevant: vec!["add_numbers"],
            tests: SearchWeakness::BothGood,
        },
        SearchTestCase {
            name: "partial_arithmetic",
            query: "arithmetic",
            relevant: vec!["add_numbers", "subtract_numbers", "multiply", "divide"],
            tests: SearchWeakness::PartialMatch,
        },
        SearchTestCase {
            name: "config_loading",
            query: "read config file",
            relevant: vec!["load_settings", "parse_config"],
            tests: SearchWeakness::Synonyms,
        },
    ]
}

/// Run quality evaluation and print results.
fn evaluate_search_quality(c: &mut Criterion) {
    let (interner, symbols) = create_test_corpus();
    let bm25_index = build_bm25_index(&interner, &symbols);
    let hybrid = HybridSearch::with_default_config();

    let mut group = c.benchmark_group("search_quality");

    println!("\n{}", "=".repeat(80));
    println!("SEARCH QUALITY EVALUATION: BM25 vs Semantic vs Hybrid");
    println!("{}\n", "=".repeat(80));

    for test_case in test_cases() {
        println!("Test: {} ({})", test_case.name, test_case.query);
        println!("  Tests: {:?}", test_case.tests);
        println!("  Expected: {:?}", test_case.relevant);

        // Get relevant symbol IDs
        let relevant_symbols: Vec<InternedString> = test_case
            .relevant
            .iter()
            .filter_map(|name| interner.get(*name))
            .collect();

        // BM25 only
        let bm25_results = bm25_index.search(
            test_case.query,
            &FieldWeights::default(),
            Bm25Params::default(),
            10,
        );
        let bm25_found: Vec<_> = bm25_results.iter().map(|r| r.symbol).collect();

        // Semantic only (simulated)
        let semantic_results = simulate_semantic_search(test_case.query, &interner, 10);
        let semantic_found: Vec<_> = semantic_results.iter().map(|(s, _)| *s).collect();

        // Hybrid
        let bm25_for_hybrid: Vec<_> = bm25_results.iter().map(|r| (r.symbol, r.score)).collect();
        let hybrid_results =
            hybrid.search(test_case.query, semantic_results.clone(), bm25_for_hybrid);

        // Calculate metrics
        let bm25_mrr = calculate_mrr(&bm25_found, &relevant_symbols);
        let semantic_mrr = calculate_mrr(&semantic_found, &relevant_symbols);
        let hybrid_mrr = calculate_mrr(
            &hybrid_results.iter().map(|r| r.symbol).collect::<Vec<_>>(),
            &relevant_symbols,
        );

        let bm25_recall = calculate_recall(&bm25_found, &relevant_symbols);
        let semantic_recall = calculate_recall(&semantic_found, &relevant_symbols);
        let hybrid_recall = calculate_recall(
            &hybrid_results.iter().map(|r| r.symbol).collect::<Vec<_>>(),
            &relevant_symbols,
        );

        // Count found by both in hybrid
        let both_count = hybrid_results
            .iter()
            .filter(|r| r.found_by == FoundBy::Both)
            .count();

        println!("  Results:");
        println!(
            "    BM25:     MRR={:.3} Recall={:.3}",
            bm25_mrr, bm25_recall
        );
        println!(
            "    Semantic: MRR={:.3} Recall={:.3}",
            semantic_mrr, semantic_recall
        );
        println!(
            "    Hybrid:   MRR={:.3} Recall={:.3} (both={})",
            hybrid_mrr, hybrid_recall, both_count
        );

        // Highlight improvements
        let best_single = bm25_mrr.max(semantic_mrr);
        if hybrid_mrr > best_single {
            println!(
                "    ✅ Hybrid IMPROVED by {:.1}%",
                (hybrid_mrr - best_single) / best_single * 100.0
            );
        } else if hybrid_mrr >= best_single {
            println!("    ✓ Hybrid matched best single method");
        } else {
            println!(
                "    ⚠ Hybrid degraded by {:.1}%",
                (best_single - hybrid_mrr) / best_single * 100.0
            );
        }
        println!();

        // Benchmark latency
        group.bench_with_input(
            BenchmarkId::new("bm25", test_case.name),
            &test_case.query,
            |b, query| {
                b.iter(|| {
                    bm25_index.search(
                        black_box(query),
                        &FieldWeights::default(),
                        Bm25Params::default(),
                        10,
                    )
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("hybrid", test_case.name),
            &test_case.query,
            |b, query| {
                b.iter(|| {
                    let semantic = simulate_semantic_search(query, &interner, 50);
                    let bm25: Vec<_> = bm25_index
                        .search(query, &FieldWeights::default(), Bm25Params::default(), 50)
                        .iter()
                        .map(|r| (r.symbol, r.score))
                        .collect();
                    hybrid.search(black_box(query), semantic, bm25)
                })
            },
        );
    }

    group.finish();

    // Print summary
    println!("{}", "=".repeat(80));
    println!("SUMMARY");
    println!("{}", "=".repeat(80));
    println!("The hybrid approach should:");
    println!("  1. Match or beat BM25 on exact keyword queries");
    println!("  2. Match or beat semantic on synonym queries");
    println!("  3. Filter out semantic junk via BM25 verification");
    println!("  4. Items found by BOTH methods should rank highest");
    println!();
}

fn calculate_mrr(results: &[InternedString], relevant: &[InternedString]) -> f32 {
    let relevant_set: std::collections::HashSet<_> = relevant.iter().copied().collect();
    results
        .iter()
        .enumerate()
        .find(|(_, r)| relevant_set.contains(r))
        .map(|(i, _)| 1.0 / (i + 1) as f32)
        .unwrap_or(0.0)
}

fn calculate_recall(results: &[InternedString], relevant: &[InternedString]) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    let relevant_set: std::collections::HashSet<_> = relevant.iter().copied().collect();
    let found = results.iter().filter(|r| relevant_set.contains(r)).count();
    found as f32 / relevant.len() as f32
}

criterion_group!(benches, evaluate_search_quality);
criterion_main!(benches);
