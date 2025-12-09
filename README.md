# Omniscient Code Index (OCI)

A semantic, interventionist code indexer for AI agents. OCI provides deep code understanding with features beyond traditional code indexers.

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
| Incremental updates | Yes | Partial |

## Installation

```bash
cargo build --release
```

## Usage

### As MCP Server

```bash
./target/release/omni-server
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

# Run comparative benchmarks only
cargo bench --bench comparative

# Run indexing benchmarks only
cargo bench --bench indexing
```

### Benchmark Results

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

### Key Observations

1. **Linear scaling** - Indexing time scales linearly with file count
2. **Fast queries** - All symbol lookups complete in microseconds
3. **Consistent throughput** - ~5 MiB/s regardless of codebase size

## Architecture

OCI uses a layered architecture:

1. **Layer 1: Topology** - Module graph with PageRank relevance scoring
2. **Layer 2: Symbols** - Function/struct/trait definitions with call graph
3. **Layer 3: Semantics** - Embedding-based similarity search (lazy)
4. **Layer 4: Analysis** - Dead code detection, coverage correlation
5. **Layer 5: Intervention** - Duplicate detection, naming conflict warnings

## Comparison with code-index

OCI provides several advantages over the existing code-index tool:

- **Dead code detection** - Identifies unused functions and types
- **Intervention engine** - Warns about duplicates and naming conflicts
- **PageRank scoring** - Ranks files by importance in the dependency graph
- **Semantic search** - Find similar code using embeddings (when enabled)
- **Better incremental updates** - Efficient single-file re-indexing

## License

MIT
