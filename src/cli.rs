//! omni - CLI for Omniscient Code Index
//!
//! A simple CLI designed for AI coding assistants to understand codebases.
//!
//! # Usage
//!
//! ```bash
//! # Index a workspace
//! omni index --root /path/to/repo
//!
//! # Query for code
//! omni query --root /path/to/repo "parse configuration"
//!
//! # Find a symbol
//! omni symbol --root /path/to/repo HybridSearch
//!
//! # Analyze dead code
//! omni analyze --root /path/to/repo dead-code
//! ```
//!
//! # Design for AI Agents
//!
//! This CLI is designed to be used by AI coding assistants:
//! - `--json` flag outputs machine-readable JSON
//! - Simple, predictable command structure
//! - Errors go to stderr, results to stdout
//! - Exit codes: 0 = success, 1 = error

use anyhow::Result;
use clap::{Parser, Subcommand};
use omni_index::export::export_engram_memory;
use omni_index::query::{QueryResponse, execute_query, load_search_index, parse_query_filters};
use omni_index::{IncrementalIndexer, IndexOptions, SymbolDef, create_state};
#[cfg(feature = "analysis")]
use omni_index::DeadCodeAnalyzer;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Parser)]
#[command(name = "omni")]
#[command(author = "BobaMatcha Solutions")]
#[command(version)]
#[command(about = "Omniscient Code Index - Semantic code search for AI agents")]
#[command(long_about = r#"
omni helps AI coding assistants understand codebases.

It provides:
  - BM25 search
  - Symbol lookup with call graph
  - Dead code analysis
  - Duplicate detection

Designed for automation: use --json for machine-readable output.
"#)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Root directory to analyze (alias: --workspace)
    #[arg(short, long, global = true, default_value = ".", alias = "workspace")]
    root: PathBuf,

    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Build or rebuild the code index
    Index {
        /// Force full rebuild (ignore cache)
        #[arg(long)]
        force: bool,

        /// Include paths that match this glob (can be used multiple times)
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,

        /// Exclude paths that match this glob (can be used multiple times)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,

        /// Disable default excludes
        #[arg(long)]
        no_default_excludes: bool,

        /// Include hidden files
        #[arg(long)]
        include_hidden: bool,

        /// Include large files
        #[arg(long)]
        include_large: bool,

        /// Max file size in bytes (ignored if --include-large)
        #[arg(long, default_value = "2097152")]
        max_file_size: u64,
    },

    /// Index multiple workspaces in one command
    IndexAll {
        /// Workspaces to analyze
        #[arg(required = true)]
        workspaces: Vec<PathBuf>,
    },

    /// Query the index using BM25 search
    Query {
        /// Search query
        query: String,

        /// Maximum results to return
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,

        /// Additional filters (path:..., ext:..., -path:...)
        #[arg(long, value_name = "FILTER")]
        filters: Vec<String>,
    },

    /// Find symbol definitions by name
    Symbol {
        /// Symbol name to find
        name: String,

        /// Use scoped name matching
        #[arg(long)]
        scoped: bool,

        /// Maximum results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// Find callers or callees of a symbol
    Calls {
        /// Symbol to analyze
        symbol: String,

        /// Direction: callers or callees
        #[arg(short, long, default_value = "callers")]
        direction: String,
    },

    /// Run code analysis
    Analyze {
        /// Analysis type: dead-code, coverage, churn
        analysis_type: String,
    },

    /// Export a context summary for downstream tools (e.g., Engram)
    Export {
        /// Export format: engram
        #[arg(long, default_value = "engram")]
        format: String,

        /// Max files to include in the summary
        #[arg(long, default_value = "20")]
        max_files: usize,

        /// Max symbols to include in the summary
        #[arg(long, default_value = "40")]
        max_symbols: usize,
    },

    /// Search the index (Claudette interface)
    Search {
        /// Search query
        query: String,

        /// Workspace path (overrides --root for this command)
        #[arg(short = 'w')]
        workspace: Option<PathBuf>,

        /// Maximum results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging (only to stderr to keep stdout clean)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();
    let root = cli.root.clone();
    let root = root.canonicalize().unwrap_or(root);

    match run_command(&cli, &root).await {
        Ok(output) => {
            if cli.json {
                let response = SuccessResponse {
                    ok: true,
                    data: output,
                };
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                print_human_readable(&output);
            }
            Ok(())
        }
        Err(e) => {
            if cli.json {
                let response = error_response(&e);
                eprintln!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    }
}

async fn run_command(cli: &Cli, root: &std::path::Path) -> Result<Output> {
    let state = create_state(root.to_path_buf());
    let indexer = IncrementalIndexer::new();

    match &cli.command {
        Commands::Index {
            force,
            include,
            exclude,
            no_default_excludes,
            include_hidden,
            include_large,
            max_file_size,
        } => {
            let options = IndexOptions {
                force: *force,
                include: include.clone(),
                exclude: exclude.clone(),
                no_default_excludes: *no_default_excludes,
                include_hidden: *include_hidden,
                include_large: *include_large,
                max_file_size: *max_file_size,
            };
            let report = indexer.index(&state, root, &options).await?;
            let docs_total = omni_index::query::load_search_state(root)?
                .map(|s| s.docs.len())
                .unwrap_or(0);
            Ok(Output::Index {
                files: report.total_files,
                symbols: docs_total,
                parsed: report.parsed_files,
                skipped: report.skipped_files,
                removed: report.removed_files,
                root: root.display().to_string(),
            })
        }
        Commands::IndexAll { workspaces } => {
            let mut results = Vec::with_capacity(workspaces.len());
            for ws in workspaces {
                let ws_path = ws.canonicalize().unwrap_or_else(|_| ws.clone());
                let ws_state = create_state(ws_path.clone());
                let report = indexer
                    .index(&ws_state, &ws_path, &IndexOptions::default())
                    .await?;
                let docs_total = omni_index::query::load_search_state(&ws_path)?
                    .map(|s| s.docs.len())
                    .unwrap_or(0);
                results.push(IndexAllResult {
                    workspace: ws_path.display().to_string(),
                    files: report.total_files,
                    symbols: docs_total,
                    call_edges: ws_state.stats().call_edge_count as usize,
                });
            }
            Ok(Output::IndexAll { results })
        }

        Commands::Query {
            query,
            top_k,
            filters,
        } => {
            let (query_text, parsed_filters) = parse_query_filters(query, filters);
            if query_text.trim().is_empty() {
                return Err(CliError::invalid_query("Query must include search terms").into());
            }

            let mut index = load_search_index(root)?;
            if index.is_none() {
                if !cli.json {
                    eprintln!("Index not found. Building index...");
                }
                indexer
                    .index(&state, root, &IndexOptions::default())
                    .await?;
                index = load_search_index(root)?;
            }

            let Some(index) = index else {
                return Err(CliError::index_missing("Index not found; run `omni index`").into());
            };

            let mut response = execute_query(&index, &query_text, *top_k, &parsed_filters);
            response.query = query.clone();
            Ok(Output::Query { response })
        }

        Commands::Symbol {
            name,
            scoped,
            limit,
        } => {
            indexer.full_index(&state, root).await?;

            let symbols: Vec<SymbolDef> = if *scoped {
                // For scoped lookup, try to find the symbol directly
                let interned = state.intern(name);
                state.get_symbol(interned).into_iter().collect()
            } else {
                state.find_by_name(name)
            };

            Ok(Output::Symbols {
                query: name.clone(),
                results: symbols
                    .into_iter()
                    .take(*limit)
                    .map(|s| SymbolResult {
                        name: state.resolve(s.scoped_name).to_string(),
                        kind: format!("{:?}", s.kind),
                        file: s.location.file.display().to_string(),
                        line: s.location.start_line,
                    })
                    .collect(),
            })
        }

        Commands::Calls { symbol, direction } => {
            indexer.full_index(&state, root).await?;

            let results: Vec<CallResult> = match direction.as_str() {
                "callers" => state
                    .find_callers(symbol)
                    .into_iter()
                    .map(|edge| CallResult {
                        caller: state.resolve(edge.caller).to_string(),
                        callee: edge.callee_name.clone(),
                        file: edge.location.file.display().to_string(),
                        line: edge.location.start_line,
                    })
                    .collect(),
                "callees" => {
                    // For callees, we need the scoped name of the caller
                    let symbols = state.find_by_name(symbol);
                    let mut all_callees = Vec::new();
                    for sym in symbols {
                        let edges = state.find_callees(sym.scoped_name);
                        for edge in edges {
                            all_callees.push(CallResult {
                                caller: state.resolve(edge.caller).to_string(),
                                callee: edge.callee_name.clone(),
                                file: edge.location.file.display().to_string(),
                                line: edge.location.start_line,
                            });
                        }
                    }
                    all_callees
                }
                _ => return Err(anyhow::anyhow!("Direction must be 'callers' or 'callees'")),
            };

            Ok(Output::Calls {
                symbol: symbol.clone(),
                direction: direction.clone(),
                results,
            })
        }

        #[cfg(feature = "analysis")]
        Commands::Analyze { analysis_type } => match analysis_type.as_str() {
            "dead-code" => {
                indexer.full_index(&state, root).await?;
                let analyzer = DeadCodeAnalyzer::new();
                let report = analyzer.analyze(&state);

                Ok(Output::DeadCode {
                    dead_count: report.dead_symbols.len(),
                    symbols: report
                        .dead_symbols
                        .into_iter()
                        .take(50) // Limit output
                        .filter_map(|scoped_name| {
                            state.get_symbol(scoped_name).map(|s| SymbolResult {
                                name: state.resolve(s.scoped_name).to_string(),
                                kind: format!("{:?}", s.kind),
                                file: s.location.file.display().to_string(),
                                line: s.location.start_line,
                            })
                        })
                        .collect(),
                })
            }
            other => Err(anyhow::anyhow!(
                "Unknown analysis type: {}. Use: dead-code",
                other
            )),
        },

        #[cfg(not(feature = "analysis"))]
        Commands::Analyze { .. } => Err(anyhow::anyhow!(
            "Analysis requires the 'analysis' feature.\n\
             Rebuild with: cargo build --features analysis"
        )),
        Commands::Export {
            format,
            max_files,
            max_symbols,
        } => {
            indexer.full_index(&state, root).await?;
            match format.as_str() {
                "engram" => {
                    let export = export_engram_memory(&state, root, *max_files, *max_symbols)?;
                    Ok(Output::ExportEngram { export })
                }
                other => Err(anyhow::anyhow!(
                    "Unknown export format: {}. Use: engram",
                    other
                )),
            }
        }

        Commands::Search {
            query,
            workspace,
            limit,
        } => {
            // Resolve workspace: -w flag overrides global --root
            let search_root = workspace.as_ref().unwrap_or(&cli.root);
            let search_root = search_root.canonicalize().unwrap_or_else(|_| search_root.clone());

            // Delegate to query logic
            let (query_text, parsed_filters) = parse_query_filters(query, &[]);
            if query_text.trim().is_empty() {
                return Err(CliError::invalid_query("Query must include search terms").into());
            }

            let search_state = create_state(search_root.clone());
            let mut index = load_search_index(&search_root)?;
            if index.is_none() {
                if !cli.json {
                    eprintln!("Index not found. Building index...");
                }
                indexer
                    .index(&search_state, &search_root, &IndexOptions::default())
                    .await?;
                index = load_search_index(&search_root)?;
            }

            let Some(index) = index else {
                return Err(CliError::index_missing("Index not found; run `omni index`").into());
            };

            let response = execute_query(&index, &query_text, *limit, &parsed_filters);

            // Return in Search-specific format for backward compat
            Ok(Output::Search {
                results: response
                    .results
                    .into_iter()
                    .map(|r| SearchResult {
                        symbol: r.symbol,
                        kind: "symbol".to_string(), // Default kind since QueryResult doesn't have it
                        file: r.file,
                        line: r.start_line,
                        score: r.score,
                    })
                    .collect(),
            })
        }
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Output {
    Index {
        files: usize,
        symbols: usize,
        parsed: usize,
        skipped: usize,
        removed: usize,
        root: String,
    },
    IndexAll {
        results: Vec<IndexAllResult>,
    },
    Query {
        #[serde(flatten)]
        response: QueryResponse,
    },
    Symbols {
        query: String,
        results: Vec<SymbolResult>,
    },
    Calls {
        symbol: String,
        direction: String,
        results: Vec<CallResult>,
    },
    #[cfg(feature = "analysis")]
    DeadCode {
        dead_count: usize,
        symbols: Vec<SymbolResult>,
    },
    ExportEngram {
        export: omni_index::export::EngramMemoryExport,
    },
    Search {
        results: Vec<SearchResult>,
    },
}

#[derive(serde::Serialize)]
struct SuccessResponse<T> {
    ok: bool,
    #[serde(flatten)]
    data: T,
}

#[derive(serde::Serialize)]
struct ErrorResponse {
    ok: bool,
    error: ErrorInfo,
}

#[derive(serde::Serialize)]
struct ErrorInfo {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    IndexMissing(String),
    #[error("{0}")]
    InvalidQuery(String),
}

impl CliError {
    fn code(&self) -> &'static str {
        match self {
            Self::IndexMissing(_) => "index_missing",
            Self::InvalidQuery(_) => "invalid_query",
        }
    }

    fn index_missing(message: &str) -> Self {
        Self::IndexMissing(message.to_string())
    }

    fn invalid_query(message: &str) -> Self {
        Self::InvalidQuery(message.to_string())
    }
}

fn error_response(err: &anyhow::Error) -> ErrorResponse {
    if let Some(cli_err) = err.downcast_ref::<CliError>() {
        return ErrorResponse {
            ok: false,
            error: ErrorInfo {
                code: cli_err.code().to_string(),
                message: cli_err.to_string(),
                details: None,
            },
        };
    }

    ErrorResponse {
        ok: false,
        error: ErrorInfo {
            code: "internal".to_string(),
            message: err.to_string(),
            details: None,
        },
    }
}

#[derive(serde::Serialize)]
struct IndexAllResult {
    workspace: String,
    files: usize,
    symbols: usize,
    call_edges: usize,
}

#[derive(serde::Serialize)]
struct SymbolResult {
    name: String,
    kind: String,
    file: String,
    line: usize,
}

#[derive(serde::Serialize)]
struct CallResult {
    caller: String,
    callee: String,
    file: String,
    line: usize,
}

#[derive(serde::Serialize)]
struct SearchResult {
    symbol: String,
    kind: String,
    file: String,
    line: usize,
    score: f32,
}

fn print_human_readable(output: &Output) {
    match output {
        Output::Index {
            files,
            symbols,
            parsed,
            skipped,
            removed,
            root,
        } => {
            println!("Indexed {} files, {} symbols", files, symbols);
            println!(
                "Parsed: {}, skipped: {}, removed: {}",
                parsed, skipped, removed
            );
            println!("Root: {}", root);
        }
        Output::IndexAll { results } => {
            println!("Indexed {} workspaces:", results.len());
            for result in results {
                println!(
                    "  {}: {} files, {} symbols, {} call edges",
                    result.workspace, result.files, result.symbols, result.call_edges
                );
            }
        }
        Output::Query { response } => {
            println!("Query: \"{}\"", response.query);
            println!("Found {} results:", response.results.len());
            for r in &response.results {
                println!(
                    "  {:.2} {} at {}:{}",
                    r.score, r.symbol, r.file, r.start_line
                );
            }
        }
        Output::Symbols { query, results } => {
            println!("Symbol: \"{}\"", query);
            println!("Found {} matches:", results.len());
            for s in results {
                println!("  {} ({}) at {}:{}", s.name, s.kind, s.file, s.line);
            }
        }
        Output::Calls {
            symbol,
            direction,
            results,
        } => {
            println!("{} of \"{}\":", direction, symbol);
            println!("Found {} results:", results.len());
            for c in results {
                println!("  {} -> {} at {}:{}", c.caller, c.callee, c.file, c.line);
            }
        }
        #[cfg(feature = "analysis")]
        Output::DeadCode {
            dead_count,
            symbols,
        } => {
            println!("Dead code analysis:");
            println!("Found {} potentially dead symbols", dead_count);
            if !symbols.is_empty() {
                println!("Top results:");
                for s in symbols {
                    println!("  {} ({}) at {}:{}", s.name, s.kind, s.file, s.line);
                }
            }
        }
        Output::ExportEngram { export } => {
            println!("{}", export.content);
        }
        Output::Search { results } => {
            println!("Found {} results:", results.len());
            for r in results {
                println!("  {:.2} {} ({}) at {}:{}", r.score, r.symbol, r.kind, r.file, r.line);
            }
        }
    }
}
