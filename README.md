# Omni Index

Omni is a fast index and query tool for code. It is built for agents and automation.

## What It Does

- Incrementally indexes a repo with sane ignore defaults
- Ranks results with BM25 over symbol spans
- Returns byte offsets and 1-based line and column numbers
- Exposes the same query engine via MCP
- Emits deterministic JSON with `--json`

## Quick Start

```bash
cargo build --release

# Index a repo
./target/release/omni index --root /path/to/repo

# Query the index
./target/release/omni query --root /path/to/repo "parse config"
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

### Query

```bash
omni query --root /path/to/repo "token" --top-k 20
```

Filters:
- `path:src/cli.rs`
- `ext:rs`
- `-path:target`

You can pass filters inline in the query or with `--filters`.

### JSON Output

All commands support `--json`.

Success shape:

```json
{
  "ok": true,
  "type": "query",
  "root": "/path/to/repo",
  "query": "parse config",
  "top_k": 10,
  "results": [
    {
      "doc_id": 12,
      "symbol": "crate::config::load",
      "file": "src/config.rs",
      "start_byte": 120,
      "end_byte": 402,
      "start_line": 9,
      "end_line": 24,
      "start_col": 1,
      "end_col": 3,
      "score": 6.42,
      "preview": "fn load_config(...)"
    }
  ]
}
```

Error shape:

```json
{
  "ok": false,
  "error": {
    "code": "invalid_query",
    "message": "Query must include search terms",
    "details": null
  }
}
```

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

## MCP Server

Run the MCP server:

```bash
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

The MCP search returns the same JSON schema as the CLI query.

## Tests

```bash
cargo test
```

## Roadmap

- Semantic search and hybrid ranking
- Chunk indexing for non symbol text
- Highlight extraction
