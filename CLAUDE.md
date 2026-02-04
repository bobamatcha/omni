# CLAUDE.md - Omniscient Code Index (OCI)

This file provides context for AI agents working with the OCI codebase.

## Project Overview

**OCI** is a fast BM25 code index for AI coding agents. It provides:
- Incremental indexing with deterministic output
- Symbol lookup and call graph traversal
- Dead code detection (optional feature)

### Primary Integration: CLI

The core contract for Claudette and other AI agents:

```bash
omni search <query> --json -w <workspace> -n <limit>
```

Expected JSON schema:
```json
{
  "ok": true,
  "type": "search",
  "results": [{ "symbol": "...", "kind": "...", "file": "...", "line": N, "score": N.N }]
}
```

### Optional: MCP Server

Build with `--features mcp` for MCP protocol support:
```bash
export OCI_WORKSPACE=/path/to/repo
./target/release/omni-server
```

## Architecture

OCI maintains three interconnected graph layers:

```
Layer 3: Semantic Embeddings  - Vector similarity for duplicate detection (optional)
Layer 2: Symbol Resolution    - Functions, structs, call graph
Layer 1: Module Topology      - Crates, modules, files with PageRank
```

## Key Files

| File | Purpose | Lines |
|------|---------|-------|
| `src/parsing/rust.rs` | Rust AST parsing with Tree-sitter | 1,110 |
| `src/mcp/mod.rs` | MCP server implementation (optional) | 740 |
| `src/context/mod.rs` | Context synthesis for LLMs (optional) | 615 |
| `src/intervention/mod.rs` | Duplicate detection (optional) | 570 |
| `src/search/bm25.rs` | BM25 text search | 521 |
| `src/topology.rs` | Module graph + PageRank | 505 |
| `src/analysis/coverage.rs` | Test coverage correlation (optional) | 460 |
| `src/fold.rs` | Code folding utilities | 438 |
| `src/analysis/dead_code.rs` | Dead code analysis (optional) | 404 |
| `src/search/mod.rs` | Hybrid search orchestration | 401 |

## How to Work With This Codebase

### Building

```bash
# Full build (all features)
cargo build --release

# Slim build (core CLI only)
cargo build --release --no-default-features --features core
```

### Testing
```bash
cargo test                          # All tests
cargo test --test contract_test     # Contract tests (stable JSON schema)
cargo test --test property_tests    # Property tests
cargo test --test comparative_test  # Comparative tests
```

### Running the MCP Server
```bash
export OCI_WORKSPACE=/path/to/repo
./target/release/omni-server
```

### Benchmarking
```bash
cargo bench --bench indexing        # Performance benchmarks
cargo bench --bench comparative     # OCI vs code-index
cargo bench --bench search_quality  # BM25 vs Semantic vs Hybrid
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `core` | BM25 search, indexing, CLI | Always available |
| `analysis` | Dead code analysis | - |
| `context` | Context synthesis | - |
| `intervention` | Duplicate detection | strsim |
| `mcp` | MCP server | rmcp, schemars |
| `semantic` | Vector embeddings | fastembed, instant-distance |

Default: All features enabled for backwards compatibility.

## Design Patterns Used

### String Interning
All symbol names use `lasso::ThreadedRodeo` for memory efficiency.

### Thread-Safe State
- `DashMap` for concurrent symbol lookup
- `RwLock` on topology graph (read-heavy)
- `Arc` for shared state across async tasks

### Lazy Initialization
Both BM25 and semantic indexes use `OnceLock` - only built on first search.

### Incremental Parsing
Tree-sitter enables partial re-parsing. Only changed files are re-indexed.

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
- **No binary quantization**: Embeddings use full float32

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
- **Contract tests**: Verify stable JSON schema for Claudette

## Related Documentation

- `docs/vision/ARCHITECTURE.md` - Full architectural vision
- `PLAN.md` - Implementation plan with phase status
- `README.md` - User-facing documentation with core contract

## Installed Skills
- @.claude/skills/suggest-tests/SKILL.md
- @.claude/skills/markdown-writer/SKILL.md
- @.claude/skills/repo-hygiene/SKILL.md
- @.claude/skills/doc-maintenance/SKILL.md
- @.claude/skills/no-workarounds/SKILL.md
- @.claude/skills/dogfood-skills/SKILL.md
- @.claude/skills/tdd/SKILL.md
