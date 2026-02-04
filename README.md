# Omni Index

A fast, deterministic BM25 index and query tool for AI agents.

## What It Does

- Incrementally indexes a repo with sane ignore defaults
- Ranks results with BM25 over symbol spans
- Returns byte offsets and 1-based line and column numbers
- Emits deterministic JSON with `--json`

## Core Contract (Stable)

These commands and their JSON schemas are stable. Claudette depends on this interface.

| Command | Schema |
|---------|--------|
| `omni index --json` | `{ ok, type: "index", files, symbols, ... }` |
| `omni search <query> -w <workspace> -n <limit> --json` | `{ ok, type: "search", results: [...] }` |

Search result schema:
```json
{
  "symbol": "crate::module::function_name",
  "kind": "symbol",
  "file": "src/module.rs",
  "line": 42,
  "score": 6.42
}
```

## Quick Start

```bash
cargo build --release

# Index a repo
./target/release/omni index --root /path/to/repo

# Search the index (Claudette interface)
./target/release/omni search "parse config" -w /path/to/repo -n 10 --json

# Query with filters
./target/release/omni query "token" --root /path/to/repo --top-k 20
```

## CLI Usage

### Index

```bash
omni index --root /path/to/repo
```

Options:
- `--force` rebuilds the cache
- `--include GLOB` re-includes excluded paths
- `--exclude GLOB` adds extra excludes
- `--no-default-excludes` disables defaults
- `--include-hidden` includes dotfiles
- `--include-large` includes large files
- `--max-file-size BYTES` sets the size cap

### Search (Primary Interface)

```bash
omni search <query> -w <workspace> -n <limit> --json
```

This is the primary interface for AI agents like Claudette. The `-w` flag is a short form specific to the search command. Other commands use `--root` or `--workspace` (global alias).

### Query

```bash
omni query <query> --root /path/to/repo --top-k 20
```

Filters:
- `path:src/cli.rs`
- `ext:rs`
- `-path:target`

You can pass filters inline in the query or with `--filters`.

### JSON Output

All commands support `--json` for machine-readable output.

## Other Commands (Non-Core)

These commands may change in future versions:

- `omni query` - BM25 search with filters (similar to search)
- `omni symbol` - Symbol lookup
- `omni calls` - Call graph queries
- `omni analyze dead-code` - Dead code analysis (requires `--features analysis`)
- `omni export` - Engram export
- `omni-server` - MCP server (requires `--features mcp`)

## Building

```bash
# Full build (all features, default)
cargo build --release

# Slim build (CLI only, no MCP/semantic)
cargo build --release --no-default-features --features core
```

| Profile | Features | Use Case |
|---------|----------|----------|
| Slim | `core` | Claudette integration, minimal footprint |
| Standard | `core,analysis` | + dead code analysis |
| Full | default (all) | + MCP server, semantic search |

## Default Excludes

Omni skips these by default:

Directories:
- `target/`, `node_modules/`, `.git/`, `dist/`, `build/`, `out/`, `coverage/`, `vendor/`, `.venv/`, `.next/`, `.omni/`

Lockfiles:
- `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`

Minified and binary assets:
- `**/*.min.js`, `**/*.min.css`, `**/*.map`
- `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.pdf`, `.zip`, `.gz`, `.tar`, `.tgz`, `.jar`, `.wasm`, `.o`, `.a`, `.so`, `.dylib`, `.dll`

## Cache Layout

The index is stored under `.omni/` in the repo root:

- `.omni/manifest.json` file fingerprints and version
- `.omni/state.bin` symbol metadata and spans
- `.omni/bm25.bin` BM25 index

Use `omni index --force` to rebuild.

## MCP Server (Experimental)

Requires building with `--features mcp`:

```bash
cargo build --release --features mcp
export OCI_WORKSPACE=/path/to/repo
./target/release/omni-server
```

Search via MCP:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "search",
    "arguments": {
      "query": "parse config",
      "top_k": 10
    }
  }
}
```

## Tests

```bash
# Contract tests only (Claudette interface, core build)
cargo test --no-default-features --features core --test contract_test

# Full test suite
cargo test
```

Test organization:

| Test File | Runs In | Purpose |
|-----------|---------|---------|
| `contract_test.rs` | core, full | Claudette contract (search + index) |
| `cli_compat_test.rs` | full only | CLI convenience (non-contract) |
| `error_handling_test.rs` | full only | Error behavior (non-contract) |
| `comparative_test.rs`, `property_tests.rs` | full only | Feature tests (analysis/intervention) |

### Pre-commit Hook

Enforce contract stability with a pre-commit hook:

```bash
ln -sf ../../scripts/pre-commit .git/hooks/pre-commit
```

The hook runs contract tests in core build before each commit.

## Architecture

See [docs/vision/ARCHITECTURE.md](docs/vision/ARCHITECTURE.md) for the full architectural vision.
