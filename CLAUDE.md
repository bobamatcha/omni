# CLAUDE.md - Omniscient Code Index (OCI)

This file provides context for AI agents working with the OCI codebase.

## Project Overview

**OCI** is a semantic, interventionist code indexer designed specifically for AI coding agents. It provides deep code understanding through a three-layer hybrid graph architecture, going beyond what traditional LSPs offer.

### Target Use Cases

This project is designed around two primary use cases (neither of which are included in this repo, but drive its design):

1. **AI Coding Agents**: The primary use case. OCI serves as the "code intelligence backend" for autonomous coding agents like Claude Code, providing symbol lookup, call graph traversal, dead code detection, and semantic search. The agent uses OCI to understand existing code before generating new code.

2. **GitHub PR Reviewer**: A WIP use case similar to CodeRabbit, but with a smaller binary footprint. OCI provides the code understanding layer that enables a PR reviewer to understand the context of changes, detect potential duplicates, and suggest improvements.

## Architecture

OCI maintains three interconnected graph layers:

```
Layer 3: Semantic Embeddings  - Vector similarity for duplicate detection
Layer 2: Symbol Resolution    - Functions, structs, call graph
Layer 1: Module Topology      - Crates, modules, files with PageRank
```

## Key Files

| File | Purpose | Lines |
|------|---------|-------|
| `src/parsing/rust.rs` | Rust AST parsing with Tree-sitter | 1,110 |
| `src/mcp/mod.rs` | MCP server implementation | 740 |
| `src/context/mod.rs` | Context synthesis for LLMs | 615 |
| `src/intervention/mod.rs` | Duplicate detection | 570 |
| `src/search/bm25.rs` | BM25 text search | 521 |
| `src/topology.rs` | Module graph + PageRank | 505 |
| `src/analysis/coverage.rs` | Test coverage correlation | 460 |
| `src/fold.rs` | Code folding utilities | 438 |
| `src/analysis/dead_code.rs` | Dead code analysis | 404 |
| `src/search/mod.rs` | Hybrid search orchestration | 401 |

## How to Work With This Codebase

### Building
```bash
cargo build --release
```

### Testing
```bash
cargo test                      # All 78 tests
cargo test --lib                # Unit tests only (46)
cargo test --test property_tests    # Property tests (19)
cargo test --test comparative_test  # Comparative tests (13)
```

### Running the MCP Server
```bash
export OCI_WORKSPACE=/path/to/repo
./target/release/omni-server
```

The server communicates via JSON-RPC 2.0 over stdio.

### Benchmarking
```bash
cargo bench --bench indexing        # Performance benchmarks
cargo bench --bench comparative     # OCI vs code-index
cargo bench --bench search_quality  # BM25 vs Semantic vs Hybrid
```

## Design Patterns Used

### String Interning
All symbol names use `lasso::ThreadedRodeo` for memory efficiency. Look up by `Spur` key, resolve back to `&str` when needed.

### Thread-Safe State
- `DashMap` for concurrent symbol lookup
- `RwLock` on topology graph (read-heavy)
- `Arc` for shared state across async tasks

### Lazy Initialization
Both BM25 and semantic indexes use `OnceLock` - only built on first search, not during indexing.

### Incremental Parsing
Tree-sitter enables partial re-parsing. Only changed files are re-indexed.

## MCP Tools

| Tool | Description |
|------|-------------|
| `index` | Build/rebuild/status index |
| `find_symbol` | Find symbol definitions by name |
| `find_calls` | Query callers/callees |
| `analyze` | Dead code, coverage, churn |
| `search` | Hybrid BM25 + semantic search |
| `context` | Generate context for a location |
| `intervention` | Check for duplicates |
| `topology` | Query module graph |

## Common Tasks

### Adding a New Symbol Type
1. Add variant to `SymbolKind` in `src/types.rs`
2. Update extraction in `src/parsing/rust.rs`
3. Update any relevant analysis in `src/analysis/`

### Adding a New Language Parser
1. Create `src/parsing/<lang>.rs`
2. Implement `LanguageParser` trait
3. Register in `IncrementalIndexer::new()`

### Adding a New MCP Tool
1. Add request/response types in `src/mcp/mod.rs`
2. Implement handler function
3. Add to `call_tool()` match statement
4. Add to `list_tools()` return value

## Known Limitations

- **Rust-only**: Only Rust parser implemented (TypeScript/Python deferred)
- **No file watching**: External process must trigger re-indexing
- **No virtual resources**: MCP resources interface not implemented
- **No binary quantization**: Embeddings use full float32 (32x memory vs quantized)

## Performance Characteristics

| Operation | Time |
|-----------|------|
| Full index (100 files) | 29 ms |
| Indexing throughput | 5.3 MiB/s |
| Symbol lookup | 4-5 us |
| BM25 search (cached) | 47 ns |
| Hybrid search | 500-800 ns |
| Dead code analysis | <100ms on 50 files |

## Error Handling

The codebase uses:
- `anyhow::Result` for operations that can fail
- `thiserror` for custom error types
- Panics only for truly unrecoverable situations (invariant violations)

## Testing Philosophy

- **Unit tests**: In each module, test individual functions
- **Property tests**: Verify invariants hold for random inputs (proptest)
- **Comparative tests**: Ensure feature parity with code-index

## Related Documentation

- `RESEARCH.md` - Architectural specification and theory
- `PLAN.md` - Implementation plan with phase status
- `README.md` - User-facing documentation
