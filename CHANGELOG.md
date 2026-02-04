# Changelog

All notable changes to omni-index will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-02-03

### Added
- Initial release of Omniscient Code Index (OCI)
- BM25 text search with symbol-aware ranking
- Incremental indexing with deterministic cache
- Tree-sitter Rust parser with symbol extraction
- CLI tools: `omni index`, `omni search`, `omni query`
- MCP server for AI agent integration (`omni-server`)
- Dead code analysis (feature-gated)
- Semantic duplicate detection (feature-gated)
- Module topology with PageRank scoring
- Symbol lookup and call graph traversal
- JSON output for stable contract with AI agents

### Features
- `core`: BM25 search, indexing, CLI (always available)
- `mcp`: MCP protocol server
- `semantic`: Vector embeddings for semantic search
- `analysis`: Dead code detection
- `context`: Context synthesis for LLMs
- `intervention`: Duplicate detection

[0.1.0]: https://github.com/bobamatcha/omni/releases/tag/v0.1.0
