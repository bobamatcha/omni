# Contributing to Omni

Thanks for your interest in contributing to omni! This project is designed to help AI coding assistants understand codebases better.

## Getting Started

```bash
# Clone the repo
git clone https://github.com/bobamatcha/omni.git
cd omni

# Build
cargo build --release

# Run tests
cargo test

# Try it out
./target/release/omni index --workspace .
./target/release/omni search --workspace . "search"
```

## Project Structure

```
omni/
├── src/
│   ├── cli.rs          # CLI binary (omni)
│   ├── main.rs         # MCP server binary (omni-server)
│   ├── lib.rs          # Library entry
│   ├── state.rs        # Global state
│   ├── types.rs        # Core types
│   ├── parsing/        # Language parsers (Rust only for now)
│   ├── search/         # BM25 and hybrid search
│   ├── semantic/       # Embedding-based search
│   ├── analysis/       # Dead code, coverage, churn
│   └── mcp/            # MCP server
├── tests/              # Integration tests
└── benches/            # Performance benchmarks
```

## How to Contribute

### Good First Issues

- Add TypeScript/Python parser support
- Improve search ranking
- Add more analysis types (complexity, duplication)
- Better error messages

### Development Flow

1. Fork the repo
2. Create a feature branch
3. Write tests first (TDD encouraged)
4. Implement the feature
5. Run `cargo test` and `cargo clippy`
6. Submit a PR

### Code Style

- Run `cargo fmt` before committing
- No warnings from `cargo clippy`
- Tests for new functionality
- Update docs if adding commands

## Questions?

Open an issue. We're friendly.

---

*This project was built by an AI (Patch) with guidance from a human (Amar). Contributions from both humans and AIs welcome.*
