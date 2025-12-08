# Omniscient Code Index (OCI) Implementation Plan

## Overview

This plan implements the Omniscient Code Index (OCI) as described in `RESEARCH.md`. The OCI is designed to replace the existing `code-index` tool in `AG1337/tools/` with an enhanced MCP server that provides:

1. **Three-Layer Hybrid Graph**: Module Topology, Symbol Resolution (Stack Graphs), Semantic Embeddings
2. **Active Intervention**: Detect semantic duplicates before code is written
3. **Quality Assurance**: Dead code analysis, test coverage integration
4. **Virtual Resources**: Memory-resident context documents via MCP

## Existing Code to Leverage

From `AG1337/tools/`:
- **`code-index/src/lib.rs`**: Tree-sitter parsing, call graph extraction, BM25 integration
- **`scribe/src/bm25.rs`**: Field-weighted BM25 index with KISS tokenization
- **`scribe/src/snippet.rs`**: Snippet extraction and window management
- **`mcp/`**: MCP server skeleton using `rmcp` crate

Key reusable patterns:
- Tree-sitter Rust parsing with incremental CST
- BM25 search with path/ident/doc/string field weighting
- Caller/callee traversal (follow_callers, follow_callees)
- Snippet handle abstraction for lazy loading
- MCP tool router macro pattern

---

## Phase 1: Project Scaffold & Core Types

**Goal**: Set up the Rust project structure and define core data types.

### Tasks (Can run in parallel)

#### 1.1 Initialize Cargo Project
```
Create workspace member: tools/omni-index/
- Cargo.toml with dependencies
- src/lib.rs (library crate)
- src/main.rs (MCP server binary)
```

**Dependencies to include**:
```toml
[dependencies]
# Async
tokio = { version = "1", features = ["full"] }
# Parsing
tree-sitter = "0.25"
tree-sitter-rust = "0.24"
tree-sitter-typescript = "0.24"
tree-sitter-python = "0.24"
# Graph
petgraph = "0.7"
# Embeddings (defer to Phase 3)
fastembed = "0.5"
# MCP
rmcp = { version = "0.3", features = ["server", "transport-io"] }
# File watching
notify = "7"
# Other
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
dashmap = "6"  # Concurrent hashmap
lasso = "0.7"  # String interning
```

#### 1.2 Define Core Types Module (`src/types.rs`)

```rust
// Node types for the Module Topology Graph (Layer 1)
pub enum TopologyNode {
    Crate { name: String, path: PathBuf },
    Module { name: String, path: PathBuf },
    File { path: PathBuf },
}

pub enum TopologyEdge {
    Contains,
    Imports { use_path: String },
    ReExports,
}

// Symbol types for the Symbol Resolution Layer (Layer 2)
pub struct SymbolDef {
    pub name: InternedString,
    pub scoped_name: InternedString,
    pub kind: SymbolKind,
    pub location: Location,
    pub signature: Option<Signature>,
    pub visibility: Visibility,
    pub attributes: Vec<String>,
}

pub enum SymbolKind {
    Function, Method, Struct, Enum, Trait, Impl, Const, Static, Module, TypeAlias,
}

pub struct CallEdge {
    pub caller: InternedString,  // scoped
    pub callee: InternedString,  // unscoped (for dynamic resolution)
    pub location: Location,
}

// Semantic embedding (Layer 3)
pub struct SemanticEntry {
    pub symbol_id: SymbolId,
    pub embedding: Vec<f32>,  // or quantized
}

// Location (same as existing code-index)
pub struct Location {
    pub file: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}
```

#### 1.3 Define Graph State Module (`src/state.rs`)

```rust
pub struct OciState {
    // Layer 1: Module Topology
    pub topology: StableGraph<TopologyNode, TopologyEdge>,
    pub topology_node_map: HashMap<PathBuf, NodeIndex>,

    // Layer 2: Symbol Resolution
    pub symbols: HashMap<InternedString, Vec<SymbolDef>>,
    pub call_graph: Vec<CallEdge>,

    // Layer 3: Semantic (loaded lazily)
    pub semantic_index: Option<SemanticIndex>,

    // File contents cache
    pub file_contents: DashMap<PathBuf, Arc<str>>,

    // BM25 index (lazy)
    pub bm25_cache: OnceLock<Bm25Index>,

    // Metadata
    pub root_path: PathBuf,
    pub git_hash: Option<String>,
    pub last_indexed: Instant,
}
```

---

## Phase 2: Ingestion Pipeline

**Goal**: Build the incremental parsing and graph construction pipeline.

### Tasks (Sequential with some parallelism)

#### 2.1 File Discovery Module (`src/discovery.rs`)

Reuse pattern from existing `code-index`:
- Use `ignore` crate for `.gitignore` respecting walks
- Filter by extension (`.rs`, `.ts`, `.py`, etc.)
- Build `HashSet<PathBuf>` of source files

#### 2.2 Tree-sitter Parsing Module (`src/parsing/mod.rs`)

**Submodules** (can develop in parallel):
- `src/parsing/rust.rs` - Rust-specific extraction
- `src/parsing/typescript.rs` - TypeScript/JavaScript
- `src/parsing/python.rs` - Python

Each module implements:
```rust
pub trait LanguageParser: Send + Sync {
    fn language(&self) -> Language;
    fn extract_symbols(&self, tree: &Tree, source: &str, file: &Path) -> Vec<SymbolDef>;
    fn extract_calls(&self, tree: &Tree, source: &str, file: &Path) -> Vec<CallEdge>;
    fn extract_imports(&self, tree: &Tree, source: &str) -> Vec<ImportInfo>;
}
```

**Rust parser** can largely reuse `walk_rust` from existing code-index with these additions:
- Extract `struct`, `enum`, `trait`, `impl` in addition to functions
- Track module hierarchy from `mod` declarations
- Parse `use` statements to build import edges

#### 2.3 Incremental Update Module (`src/incremental.rs`)

```rust
pub struct IncrementalIndexer {
    parsers: HashMap<&'static str, Box<dyn LanguageParser>>,
    debounce_ms: u64,
}

impl IncrementalIndexer {
    /// Update index for a single changed file
    pub async fn update_file(&self, state: &mut OciState, path: &Path) -> Result<()>;

    /// Remove a deleted file from the index
    pub fn remove_file(&self, state: &mut OciState, path: &Path);

    /// Full rebuild (for cold start)
    pub async fn full_index(&self, state: &mut OciState, root: &Path) -> Result<()>;
}
```

Key: Only re-parse changed files, but re-stitch symbol references across boundaries.

#### 2.4 Module Topology Builder (`src/topology.rs`)

Build the `petgraph::StableGraph` from parsed files:
1. Create `Crate` node for workspace root
2. Create `File` nodes for each source file
3. Create `Module` nodes from `mod` declarations
4. Add `Contains` edges (crate -> module -> file)
5. Add `Imports` edges from `use` statements
6. Compute PageRank scores for relevance ranking

---

## Phase 3: Semantic Layer (Vector Embeddings)

**Goal**: Add semantic search capability using local embeddings.

### Tasks

#### 3.1 Embedding Integration (`src/semantic/mod.rs`)

```rust
pub struct SemanticIndex {
    model: FastEmbed,  // fastembed-rs
    index: HnswIndex,  // instant-distance or hnsw_rs
    id_to_symbol: Vec<InternedString>,
}

impl SemanticIndex {
    pub fn new() -> Result<Self>;
    pub fn add(&mut self, symbol: &SymbolDef, code_snippet: &str) -> Result<()>;
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(f32, InternedString)>;
    pub fn remove(&mut self, symbol_id: &InternedString);
}
```

**Model**: Use `jina-embeddings-v2-base-code` or `all-MiniLM-L6-v2` for initial implementation.

#### 3.2 Binary Quantization (`src/semantic/quantize.rs`)

For memory efficiency on large codebases:
```rust
pub fn quantize_binary(embedding: &[f32]) -> Vec<u8>;
pub fn hamming_distance(a: &[u8], b: &[u8]) -> u32;
```

Use binary quantization for first-pass filtering, then rerank with full vectors.

---

## Phase 4: Quality Assurance Features

**Goal**: Implement dead code analysis and test coverage integration.

### Tasks (Can run in parallel)

#### 4.1 Dead Code Analysis (`src/analysis/dead_code.rs`)

```rust
pub struct DeadCodeAnalyzer;

impl DeadCodeAnalyzer {
    /// Find entry points (main, tests, public lib exports, FFI)
    pub fn find_roots(&self, state: &OciState) -> Vec<InternedString>;

    /// BFS from roots, mark reachable symbols
    pub fn compute_reachability(&self, state: &OciState) -> HashSet<InternedString>;

    /// Return unreachable symbols (potential dead code)
    pub fn find_dead_code(&self, state: &OciState) -> Vec<&SymbolDef>;
}
```

Handle trait implementations conservatively (if trait is reachable, mark all impls).

#### 4.2 Test Coverage Integration (`src/analysis/coverage.rs`)

```rust
pub struct CoverageData {
    // file -> line -> hits
    pub line_coverage: HashMap<PathBuf, HashMap<usize, u64>>,
}

impl CoverageData {
    /// Parse lcov.info format
    pub fn from_lcov(path: &Path) -> Result<Self>;

    /// Map line coverage to symbol coverage
    pub fn symbol_coverage(&self, state: &OciState) -> HashMap<InternedString, f32>;
}
```

#### 4.3 Churn Analysis (`src/analysis/churn.rs`)

```rust
pub struct ChurnAnalyzer;

impl ChurnAnalyzer {
    /// Get file modification frequency from git log
    pub fn compute_churn(&self, root: &Path, days: u32) -> HashMap<PathBuf, u32>;
}
```

---

## Phase 5: Intervention Engine

**Goal**: Implement the "killer feature" - active intervention to prevent duplicates.

### Tasks

#### 5.1 Similarity Detector (`src/intervention/similarity.rs`)

```rust
pub struct SimilarityDetector {
    threshold: f32,  // e.g., 0.85
}

impl SimilarityDetector {
    /// Check if proposed code/signature is similar to existing
    pub fn find_similar(
        &self,
        state: &OciState,
        proposed: &str,
        exclude_file: Option<&Path>,
    ) -> Vec<SimilarityMatch>;
}

pub struct SimilarityMatch {
    pub existing_symbol: InternedString,
    pub location: Location,
    pub score: f32,
    pub suggestion: String,
}
```

#### 5.2 Intervention Controller (`src/intervention/controller.rs`)

```rust
pub struct InterventionController {
    detector: SimilarityDetector,
    enabled: AtomicBool,
}

impl InterventionController {
    /// Analyze proposed code change, return interventions if any
    pub async fn check_proposal(
        &self,
        state: &OciState,
        file: &Path,
        proposed_content: &str,
    ) -> Vec<Intervention>;
}

pub struct Intervention {
    pub severity: Severity,  // Warning, Block
    pub message: String,
    pub existing_location: Location,
    pub recommendation: String,
}
```

---

## Phase 6: Context Synthesis ("Ghost Docs")

**Goal**: Auto-generate architectural context documents.

### Tasks

#### 6.1 Pattern Extractor (`src/context/patterns.rs`)

```rust
pub struct PatternExtractor;

impl PatternExtractor {
    pub fn detect_error_handling(&self, state: &OciState) -> ErrorHandlingPattern;
    pub fn detect_async_runtime(&self, state: &OciState) -> Option<AsyncRuntime>;
    pub fn detect_testing_strategy(&self, state: &OciState) -> TestingStrategy;
    pub fn detect_design_patterns(&self, state: &OciState) -> Vec<DesignPattern>;
}
```

#### 6.2 Context Document Generator (`src/context/generator.rs`)

```rust
pub struct ContextGenerator;

impl ContextGenerator {
    /// Generate virtual CLAUDE.md content
    pub fn generate_context(&self, state: &OciState) -> String;

    /// Merge with existing CLAUDE.md if present
    pub fn merge_with_existing(&self, state: &OciState, existing: &str) -> String;
}
```

---

## Phase 7: MCP Server Implementation

**Goal**: Expose all functionality via MCP protocol.

### Tasks

#### 7.1 Server Skeleton (`src/mcp/server.rs`)

```rust
pub struct OciServer {
    state: Arc<RwLock<OciState>>,
    indexer: Arc<IncrementalIndexer>,
    intervention: Arc<InterventionController>,
    watcher: Arc<FileWatcher>,
}
```

#### 7.2 MCP Tools (`src/mcp/tools.rs`)

Implement these tools:

| Tool Name | Description |
|-----------|-------------|
| `build_index` | Full index rebuild |
| `search` | BM25 + semantic hybrid search |
| `find_symbol` | Find symbol definitions by name |
| `find_callers` | Find call sites of a function |
| `find_callees` | Find functions called by a function |
| `check_dead_code` | Run dead code analysis |
| `get_coverage` | Get coverage data for symbols |
| `check_intervention` | Check proposed code for duplicates |
| `fold_signatures` | Reduce file to signatures only |
| `get_context` | Get generated context document |
| `expand_snippet` | Expand a snippet window |

#### 7.3 MCP Resources (`src/mcp/resources.rs`)

Virtual resources:
- `virtual://context/summary.md` - Generated context
- `virtual://index/plan` - Current inferred plan
- `virtual://index/dead_code` - Dead code report
- `virtual://index/coverage` - Coverage report

#### 7.4 File Watcher Integration (`src/mcp/watcher.rs`)

```rust
pub struct FileWatcher {
    tx: mpsc::Sender<WatchEvent>,
}

impl FileWatcher {
    pub fn start(&self, root: &Path) -> Result<()>;
    pub fn stop(&self);
}
```

On file change:
1. Debounce (200ms)
2. Trigger incremental re-index
3. Push `resources/updated` notification to MCP clients

---

## Phase 8: Integration & Testing

**Goal**: Integrate with AG1337, add tests, benchmark.

### Tasks (Can run in parallel)

#### 8.1 Integration Tests

```
tests/
  indexer_test.rs      - Full indexing tests
  search_test.rs       - BM25 + semantic search
  intervention_test.rs - Duplicate detection
  dead_code_test.rs    - Reachability analysis
  mcp_test.rs          - MCP protocol tests
```

#### 8.2 Benchmarks

```
benches/
  index_build.rs   - Indexing throughput
  search.rs        - Query latency
  incremental.rs   - Incremental update speed
```

#### 8.3 AG1337 Integration

1. Update `AG1337/tools/mcp/Cargo.toml` to depend on `omni-index` OR
2. Run as separate MCP server process
3. Update Claude Code MCP configuration to use new server

---

## Implementation Order & Parallelization Strategy

### Wave 1 (Foundation) - Sequential Start
- **1.1** Initialize project
- **1.2** Define types
- **1.3** Define state

### Wave 2 (Parsing) - Parallel
- **2.1** File discovery (1 agent)
- **2.2** Tree-sitter parsers (3 agents: Rust, TS, Python)
- **2.3** Incremental indexer (1 agent)
- **2.4** Topology builder (1 agent)

### Wave 3 (Features) - Parallel
- **3.1** Embedding integration (1 agent)
- **4.1** Dead code analysis (1 agent)
- **4.2** Coverage integration (1 agent)
- **4.3** Churn analysis (1 agent)

### Wave 4 (Intervention) - Sequential
- **5.1** Similarity detector
- **5.2** Intervention controller

### Wave 5 (Context & MCP) - Parallel
- **6.1** Pattern extractor (1 agent)
- **6.2** Context generator (1 agent)
- **7.1-7.4** MCP server (1 agent)

### Wave 6 (Testing) - Parallel
- **8.1** Integration tests (1 agent)
- **8.2** Benchmarks (1 agent)
- **8.3** AG1337 integration (1 agent)

---

## Execution Commands for Claude Code

When implementing, use these subagent patterns:

```
# Wave 2 example - launch 4 agents in parallel:
Task: "Implement file discovery module in src/discovery.rs using ignore crate"
Task: "Implement Rust parser in src/parsing/rust.rs extracting symbols and calls"
Task: "Implement TypeScript parser in src/parsing/typescript.rs"
Task: "Implement topology builder in src/topology.rs using petgraph"

# Each agent should:
# 1. Read existing code-index implementation for patterns
# 2. Write new implementation
# 3. Add basic unit tests
# 4. Run cargo check to verify
```

---

## Success Criteria

1. **Indexing**: Index 10k+ file repo in <30 seconds
2. **Search**: <100ms query latency with BM25 + semantic hybrid
3. **Incremental**: <500ms update on single file change
4. **Memory**: <500MB for 50k function index
5. **Intervention**: Detect >90% of semantic duplicates at 0.85 threshold
6. **MCP**: Pass all protocol conformance tests

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Embedding model size | Use smaller model (all-MiniLM-L6-v2) or quantization |
| Stack graphs complexity | Start with simpler call graph, add full stack graphs later |
| Tree-sitter multi-language | Prioritize Rust, add others incrementally |
| MCP protocol changes | Pin rmcp version, abstract protocol layer |

---

## File Structure

```
omni/
├── Cargo.toml
├── RESEARCH.md
├── PLAN.md
└── src/
    ├── lib.rs
    ├── main.rs
    ├── types.rs
    ├── state.rs
    ├── discovery.rs
    ├── parsing/
    │   ├── mod.rs
    │   ├── rust.rs
    │   ├── typescript.rs
    │   └── python.rs
    ├── topology.rs
    ├── incremental.rs
    ├── semantic/
    │   ├── mod.rs
    │   └── quantize.rs
    ├── analysis/
    │   ├── mod.rs
    │   ├── dead_code.rs
    │   ├── coverage.rs
    │   └── churn.rs
    ├── intervention/
    │   ├── mod.rs
    │   ├── similarity.rs
    │   └── controller.rs
    ├── context/
    │   ├── mod.rs
    │   ├── patterns.rs
    │   └── generator.rs
    └── mcp/
        ├── mod.rs
        ├── server.rs
        ├── tools.rs
        ├── resources.rs
        └── watcher.rs
```
