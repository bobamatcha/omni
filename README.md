# Omniscient Code Index (OCI)

A semantic, interventionist code indexer for AI coding agents. OCI provides deep code understanding through a three-layer hybrid graph architecture, going beyond what traditional language servers offer.

Note: The current CLI and MCP `search` use BM25 only. Semantic embeddings and hybrid fusion are roadmap items.

## Target Use Cases

This project is designed around two primary use cases (neither included in this repo, but they drive design decisions):

1. **AI Coding Agents**: OCI serves as the "code intelligence backend" for autonomous coding agents. It provides symbol lookup, call graph traversal, dead code detection, and semantic search. Agents use OCI to understand existing code before generating new code, preventing duplication and maintaining architectural consistency.

2. **GitHub PR Reviewer**: A lightweight PR review tool (similar to CodeRabbit but with a smaller binary). OCI provides the code understanding layer that enables reviewers to detect potential duplicates, understand change context, and suggest improvements.

## Features

| Feature | OCI | Traditional LSP |
|---------|-----|-----------------|
| Symbol extraction | Yes | Yes |
| Call graph tracking | Yes | Partial |
| Scoped name resolution | Yes | Yes |
| Dead code analysis | Yes | No |
| Intervention engine | Yes | No |
| PageRank relevance | Yes | No |
| Semantic embeddings | Roadmap | No |
| BM25 text search | Yes | No |
| Incremental updates | Yes | Yes |
| Test coverage correlation | Yes | No |
| Git churn analysis | Yes | No |

## Quick Start

### Installation

```bash
# Clone and build
git clone <repo>
cd omni
cargo build --release
```

### CLI (Recommended for AI Agents)

The `omni` CLI is the simplest way to use OCI:

```bash
# Index a repo
omni index --root /path/to/repo

# Query for code (BM25)
omni query --root /path/to/repo "parse configuration"

# Find a symbol
omni symbol --root /path/to/repo HybridSearch

# Find callers of a function
omni calls --root /path/to/repo my_function

# Analyze dead code
omni analyze --root /path/to/repo dead-code

# Get JSON output (for automation)
omni query --root /path/to/repo --json "parse configuration"
```

The CLI outputs human-readable text by default, or JSON with `--json` for easy parsing by AI agents.

### Default Excludes

By default, omni skips common build and dependency directories plus lockfiles and binary assets. Examples:

- Directories: `target/`, `node_modules/`, `.git/`, `dist/`, `build/`, `out/`, `coverage/`, `vendor/`, `.venv/`, `.next/`
- Lockfiles: `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`

Use `--include`/`--exclude` or `--no-default-excludes` to override.

### Performance Notes

- Incremental indexing uses a persisted manifest to skip unchanged files.
- Cache lives under `.omni/` in the repo root.
- Use `omni index --force` to rebuild from scratch.

### Running the MCP Server

For advanced integration, OCI also provides an MCP server:

```bash
# Set the workspace directory to index
export OCI_WORKSPACE=/path/to/your/rust/repo

# Run the MCP server (communicates via JSON-RPC 2.0 over stdio)
./target/release/omni-server
```

### Example MCP Tool Calls

**Build the index:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "index",
    "arguments": {"op": "build"}
  }
}
```

**Find a symbol:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "find_symbol",
    "arguments": {"name": "my_function", "scoped": false, "max_results": 10}
  }
}
```

**Search with BM25:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "search",
    "arguments": {"query": "parse configuration file", "top_k": 10}
  }
}
```

**Check for dead code:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "analyze",
    "arguments": {"analysis": "dead_code"}
  }
}
```

## MCP Tools Reference

| Tool | Description | Key Arguments |
|------|-------------|---------------|
| `index` | Build, rebuild, or check index status | `op`: "build", "rebuild", "status" |
| `find_symbol` | Find symbol definitions by name | `name`, `scoped`, `max_results` |
| `find_calls` | Query call graph | `symbol`, `direction`: "callers" or "callees" |
| `analyze` | Run analysis | `analysis`: "dead_code", "coverage", "churn" |
| `search` | BM25 search | `query`, `top_k` |
| `context` | Generate context for a location | `file`, `line`, `token_budget` |
| `intervention` | Check for naming conflicts/duplicates | `proposed_name`, `file` |
| `topology` | Query module graph and PageRank | `query_type`: "modules", "pagerank", "imports" |

## Using as a Library

```rust
use omni_index::{create_state, IncrementalIndexer, DeadCodeAnalyzer};
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create state for a repository
    let state = create_state("/path/to/repo".into());
    let indexer = IncrementalIndexer::new();

    // Index the repository
    indexer.full_index(&state, Path::new("/path/to/repo")).await?;

    // Query symbols
    let symbols = state.find_by_name("my_function");
    for sym in &symbols {
        println!("{}: {:?}", state.resolve(sym.scoped_name), sym.kind);
    }

    // Find callers of a function
    let callers = state.find_callers("process_data");
    println!("Found {} callers", callers.len());

    // Analyze dead code
    let analyzer = DeadCodeAnalyzer::new();
    let report = analyzer.analyze(&state);
    println!("Found {} potentially dead symbols", report.dead_symbols.len());

    Ok(())
}
```

## Architecture

OCI uses a layered architecture with three interconnected graph layers:

```
┌─────────────────────────────────────────────────────────────┐
│                    Layer 5: Intervention                     │
│         Duplicate detection, naming conflict warnings        │
├─────────────────────────────────────────────────────────────┤
│                     Layer 4: Analysis                        │
│     Dead code detection, coverage correlation, git churn     │
├─────────────────────────────────────────────────────────────┤
│                     Layer 3: Search                          │
│     BM25 search (semantic/hybrid on roadmap)                 │
├─────────────────────────────────────────────────────────────┤
│                     Layer 2: Symbols                         │
│       Function/struct/trait definitions with call graph      │
├─────────────────────────────────────────────────────────────┤
│                     Layer 1: Topology                        │
│          Module graph with PageRank relevance scoring        │
└─────────────────────────────────────────────────────────────┘
```

### Roadmap: Hybrid Search Pipeline

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

The planned pipeline:
1. **Embeddings for broad recall** - Fixes BM25's synonym blindness
2. **BM25 for precision** - Protects against junk semantic matches
3. **RRF fusion** - Combines rankings robustly (k=60, weights: semantic=0.4, bm25=0.6)

## Testing

```bash
# Run all tests (78 total)
cargo test

# Run specific test suites
cargo test --lib                     # Unit tests (46)
cargo test --test property_tests     # Property-based tests (19)
cargo test --test comparative_test   # Comparative tests (13)

# With verbose output
cargo test -- --nocapture
```

## Benchmarks

```bash
# Run all benchmarks
cargo bench

# Individual benchmark suites
cargo bench --bench indexing        # Indexing performance
cargo bench --bench comparative     # OCI vs code-index comparison
cargo bench --bench search_quality  # legacy hybrid benchmark (roadmap)
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
| Find by name (simple) | 4.7 us |
| Find by name (common) | 3.9 us |
| Find by prefix scan | 3.1 us |
| Find callers | 1.8 us |
| Find callers (method) | 3.3 us |

### Roadmap: Search Quality

Planned hybrid search combines BM25 (keyword matching) with semantic embeddings:

| Query | BM25 MRR | Semantic MRR | Hybrid MRR | Found by Both |
|-------|----------|--------------|------------|---------------|
| "addition" (synonym) | 0.000 | 1.000 | 1.000 | 0 |
| "compute total" (synonym) | 0.500 | 1.000 | 1.000 | 1 |
| "make network call" | 0.000 | 1.000 | 1.000 | 0 |
| "add_numbers" (exact) | 1.000 | 0.000 | 1.000 | 0 |
| "arithmetic" (partial) | 1.000 | 1.000 | 1.000 | 4 |
| "read config file" | 1.000 | 1.000 | 1.000 | 2 |

**Key Findings:**

1. **Semantic fixes BM25's synonym blindness**: For queries like "addition" (finds `add_numbers`, `sum_values`, `calculate_total`), BM25 alone scores 0.0 but semantic search finds all relevant results.

2. **BM25 protects exact matches**: For the exact query "add_numbers", BM25 has perfect recall while semantic may miss it. Hybrid still gets 100% by incorporating BM25 results.

3. **Items found by BOTH methods rank highest**: When both BM25 and semantic agree, confidence is highest.

4. **RRF fusion works robustly**: Reciprocal Rank Fusion combines rankings without needing score normalization.

**Projected Search Latency (roadmap):**

| Method | Latency |
|--------|---------|
| BM25 only | 47-257 ns |
| Hybrid (BM25 + Semantic fusion) | 563-800 ns |

### Comparison with code-index

| Metric | OCI | code-index | Improvement |
|--------|-----|------------|-------------|
| Index build (medium) | ~1 ms | 8.3 ms | **8x faster** |
| Symbol lookup | 54 us | N/A | - |
| BM25 cached | 47 ns | 1.4 us | **30x faster** |

## Project Structure

```
omni/
├── Cargo.toml              # Project manifest
├── RESEARCH.md             # Architectural specification
├── PLAN.md                 # Implementation plan
├── CLAUDE.md               # Context for AI agents
├── README.md               # This file
├── src/
│   ├── lib.rs              # Library entry
│   ├── main.rs             # MCP server binary
│   ├── types.rs            # Core type definitions
│   ├── state.rs            # Global state management
│   ├── discovery.rs        # File discovery
│   ├── parsing/
│   │   ├── mod.rs          # Parser trait
│   │   └── rust.rs         # Rust parser (1,110 lines)
│   ├── topology.rs         # Module topology + PageRank
│   ├── incremental.rs      # Incremental indexing
│   ├── fold.rs             # Code folding utilities
│   ├── search/
│   │   ├── mod.rs          # Search (BM25, hybrid roadmap)
│   │   └── bm25.rs         # BM25 text search
│   ├── semantic/
│   │   └── mod.rs          # Embedding layer
│   ├── analysis/
│   │   ├── dead_code.rs    # Dead code analysis
│   │   ├── coverage.rs     # Coverage integration
│   │   └── churn.rs        # Git churn analysis
│   ├── intervention/
│   │   └── mod.rs          # Duplicate detection
│   ├── context/
│   │   └── mod.rs          # Context synthesis
│   └── mcp/
│       └── mod.rs          # MCP server
├── tests/
│   ├── comparative_test.rs
│   └── property_tests.rs
└── benches/
    ├── indexing.rs
    ├── comparative.rs
    └── search_quality.rs
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OCI_WORKSPACE` | Root directory to index | Current directory |
| `RUST_LOG` | Log level (trace, debug, info, warn, error) | warn |

### Embedding Model

OCI uses AllMiniLM-L6-v2 via fastembed. The model is downloaded on first use and cached in `.fastembed_cache/`.

## Known Limitations

- **Rust-only**: Only Rust parser implemented (TypeScript/Python support planned)
- **No file watching**: External process must trigger re-indexing via MCP calls
- **No virtual resources**: MCP resources interface not yet implemented
- **No binary quantization**: Embeddings use full float32 (could be optimized for memory)

## Documentation

- **RESEARCH.md** - Detailed architectural specification and theory
- **PLAN.md** - Implementation plan with phase completion status
- **CLAUDE.md** - Context document for AI agents working with this codebase

## License

MIT
