# Omniscient Code Index (OCI)

A semantic, interventionist code indexer for AI agents. OCI provides deep code understanding with features beyond traditional code indexers, including hybrid search combining BM25 and semantic embeddings.

## Features

| Feature | OCI | code-index |
|---------|-----|------------|
| Symbol extraction | Yes | Yes |
| Call graph tracking | Yes | Yes |
| Scoped name resolution | Yes | Yes |
| Dead code analysis | Yes | No |
| Intervention engine | Yes | No |
| PageRank relevance | Yes | No |
| Semantic embeddings | Yes | No |
| BM25 text search | Yes | Yes |
| Hybrid search (BM25 + Semantic) | Yes | No |
| Incremental updates | Yes | Partial |

## Installation

```bash
cargo build --release
```

## Usage

### As MCP Server

The MCP server exposes OCI functionality via the Model Context Protocol for AI agents.

```bash
# Set the workspace directory
export OCI_WORKSPACE=/path/to/your/repo

# Run the MCP server
./target/release/omni-server
```

The server communicates via JSON-RPC 2.0 over stdio and exposes these tools:

| Tool | Description |
|------|-------------|
| `index` | Build or rebuild the code index |
| `find_symbol` | Find symbol definitions by name |
| `call_graph` | Query callers/callees of a function |
| `topology` | Query module topology and PageRank scores |
| `search` | Hybrid search combining BM25 and semantic |
| `dead_code` | Analyze unreachable/unused code |

Example MCP tool call:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "find_symbol",
    "arguments": {"name": "my_function"}
  }
}
```

### As Library

```rust
use omni_index::{create_state, IncrementalIndexer};

let state = create_state("/path/to/repo".into());
let indexer = IncrementalIndexer::new();

// Index the repository
indexer.full_index(&state, Path::new("/path/to/repo")).await?;

// Query symbols
let symbols = state.find_by_name("my_function");

// Find callers
let callers = state.find_callers("my_function");

// Dead code analysis
let analyzer = DeadCodeAnalyzer::new();
let report = analyzer.analyze(&state);
```

### Hybrid Search

OCI implements a hybrid search pipeline combining BM25 and semantic embeddings:

```rust
use omni_index::{HybridSearch, HybridSearchConfig, Bm25Index};

// Configure hybrid search
let config = HybridSearchConfig {
    semantic_top_k: 50,   // Broad recall from embeddings
    bm25_top_k: 50,       // Candidates from BM25
    final_top_k: 10,      // Final results
    semantic_weight: 0.4, // Weight for semantic scores
    bm25_weight: 0.6,     // Weight for BM25 scores
    use_rrf: true,        // Use Reciprocal Rank Fusion
    ..Default::default()
};

let hybrid = HybridSearch::new(config);

// Perform search (semantic_results from embedding model, bm25_results from index)
let results = hybrid.search("compute total", semantic_results, bm25_results);

// Results include which method found each item
for result in results {
    println!("{:?}: score={}, found_by={:?}",
        result.symbol, result.score, result.found_by);
}
```

## Testing

```bash
# Run all tests (68 total)
cargo test

# Run unit tests only (36 tests)
cargo test --lib

# Run property tests (19 tests)
cargo test --test property_tests

# Run comparative tests (13 tests)
cargo test --test comparative_test
```

## Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run indexing performance benchmarks
cargo bench --bench indexing

# Run comparative benchmarks (OCI vs code-index features)
cargo bench --bench comparative

# Run search quality benchmarks (BM25 vs Semantic vs Hybrid)
cargo bench --bench search_quality
```

### Indexing Performance

Performance on synthetic Rust codebases:

| Operation | 10 files | 25 files | 50 files | 100 files |
|-----------|----------|----------|----------|-----------|
| Full index | 3.0 ms | 7.2 ms | 14.4 ms | 29.2 ms |
| Throughput | 5.0 MiB/s | 5.3 MiB/s | 5.3 MiB/s | 5.3 MiB/s |

Query performance (50-file codebase):

| Query Type | Time |
|------------|------|
| Find by name (simple) | 4.7 µs |
| Find by name (common) | 3.9 µs |
| Find by prefix scan | 3.1 µs |
| Find callers | 1.8 µs |
| Find callers (method) | 3.3 µs |

### Search Quality Benchmarks

The hybrid search combines BM25 (keyword matching) with semantic embeddings to get the best of both approaches.

**Test Results:**

| Query | BM25 MRR | Semantic MRR | Hybrid MRR | Found by Both |
|-------|----------|--------------|------------|---------------|
| "addition" (synonym) | 0.000 | 1.000 | 1.000 | 0 |
| "compute total" (synonym) | 0.500 | 1.000 | 1.000 | 1 |
| "make network call" | 0.000 | 1.000 | 1.000 | 0 |
| "add_numbers" (exact) | 1.000 | 0.000 | 1.000 | 0 |
| "arithmetic" (partial) | 1.000 | 1.000 | 1.000 | 4 |
| "read config file" | 1.000 | 1.000 | 1.000 | 2 |

**Key Findings:**

1. **Semantic fixes BM25's synonym blindness**: For queries like "addition" (finds `add_numbers`, `sum_values`, `calculate_total`) and "make network call" (finds `send_request`, `fetch_data`), BM25 alone scores 0.0 but semantic search finds all relevant results.

2. **BM25 protects exact matches**: For the exact query "add_numbers", BM25 has perfect recall while semantic may miss it. Hybrid still gets 100% by incorporating BM25 results.

3. **Items found by BOTH methods rank highest**: In the "arithmetic" test, all 4 relevant results were found by both methods (both=4). This is the strongest signal - when both BM25 and semantic agree, confidence is highest.

4. **RRF fusion works robustly**: Reciprocal Rank Fusion combines rankings without needing score normalization across different methods.

**Search Latency:**

| Method | Latency |
|--------|---------|
| BM25 only | 47-257 ns |
| Hybrid (BM25 + Semantic fusion) | 563-800 ns |

Hybrid adds ~500ns overhead for fusion, negligible compared to the quality improvement.

### Comparison with code-index

| Metric | OCI | code-index | Improvement |
|--------|-----|------------|-------------|
| Index build (medium) | ~1 ms | 8.3 ms | **8x faster** |
| Symbol lookup | 54 µs | N/A | - |
| BM25 cached | 47 ns | 1.4 µs | **30x faster** |

## Architecture

OCI uses a layered architecture:

```
┌─────────────────────────────────────────────────────────────┐
│                    Layer 5: Intervention                     │
│         Duplicate detection, naming conflict warnings        │
├─────────────────────────────────────────────────────────────┤
│                     Layer 4: Analysis                        │
│            Dead code detection, coverage correlation         │
├─────────────────────────────────────────────────────────────┤
│                     Layer 3: Search                          │
│     Hybrid search: BM25 + Semantic embeddings + RRF fusion   │
├─────────────────────────────────────────────────────────────┤
│                     Layer 2: Symbols                         │
│       Function/struct/trait definitions with call graph      │
├─────────────────────────────────────────────────────────────┤
│                     Layer 1: Topology                        │
│          Module graph with PageRank relevance scoring        │
└─────────────────────────────────────────────────────────────┘
```

### Hybrid Search Pipeline

```
Query
  │
  ├──► [Semantic Search] ──► Top-50 candidates (ANN on embeddings)
  │                                │
  │                                ▼
  └──► [BM25 Search] ─────► Top-50 candidates
                                   │
                                   ▼
                          [RRF Fusion]
                                   │
                                   ▼
                          Final Top-10 results
```

The pipeline:
1. **Embeddings for broad recall** - Fixes BM25's synonym blindness
2. **BM25 for precision** - Protects against junk semantic matches
3. **RRF fusion** - Combines rankings robustly (k=60, weights: semantic=0.4, bm25=0.6)

## Integration with AG1337

OCI is designed to replace `code-index` in the AG1337 agent framework. To benchmark:

```bash
# From AG1337 directory
cargo run -p benchmarks -- --omni
```

This runs the OCI MCP server through the standard benchmarking harness to measure performance in the brain → MCP → tools flow.

## License

MIT
