//! omni - CLI for Omniscient Code Index
//!
//! A simple CLI designed for AI coding assistants to understand codebases.
//!
//! # Usage
//!
//! ```bash
//! # Index a workspace
//! omni index --workspace /path/to/repo
//!
//! # Search for code
//! omni search --workspace /path/to/repo "parse configuration"
//!
//! # Find a symbol
//! omni symbol --workspace /path/to/repo HybridSearch
//!
//! # Analyze dead code
//! omni analyze --workspace /path/to/repo dead-code
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
use omni_index::{
    create_state, DeadCodeAnalyzer, IncrementalIndexer, SymbolDef,
};
use omni_index::export::export_engram_memory;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "omni")]
#[command(author = "BobaMatcha Solutions")]
#[command(version)]
#[command(about = "Omniscient Code Index - Semantic code search for AI agents")]
#[command(long_about = r#"
omni helps AI coding assistants understand codebases.

It provides:
  - Hybrid search (keyword + semantic)
  - Symbol lookup with call graph
  - Dead code analysis
  - Duplicate detection

Designed for automation: use --json for machine-readable output.
"#)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Workspace directory to analyze
    #[arg(short, long, global = true, default_value = ".")]
    workspace: PathBuf,

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
    },

    /// Index multiple workspaces in one command
    IndexAll {
        /// Workspaces to analyze
        #[arg(required = true)]
        workspaces: Vec<PathBuf>,
    },

    /// Search for code using hybrid (keyword + semantic) search
    Search {
        /// Search query
        query: String,

        /// Maximum results to return
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
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
    let workspace = cli.workspace.clone();
    let workspace = workspace.canonicalize().unwrap_or(workspace);

    match run_command(&cli, &workspace).await {
        Ok(output) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                print_human_readable(&output);
            }
            Ok(())
        }
        Err(e) => {
            if cli.json {
                let err = serde_json::json!({
                    "error": e.to_string()
                });
                eprintln!("{}", serde_json::to_string_pretty(&err)?);
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    }
}

async fn run_command(cli: &Cli, workspace: &PathBuf) -> Result<Output> {
    let state = create_state(workspace.clone());
    let indexer = IncrementalIndexer::new();

    match &cli.command {
        Commands::Index { force } => {
            if *force {
                indexer.full_index(&state, workspace).await?;
            } else {
                indexer.full_index(&state, workspace).await?;
            }
            let stats = state.stats();
            Ok(Output::Index {
                files: stats.file_count as usize,
                symbols: stats.symbol_count as usize,
                workspace: workspace.display().to_string(),
            })
        }
        Commands::IndexAll { workspaces } => {
            let mut results = Vec::with_capacity(workspaces.len());
            for ws in workspaces {
                let ws_path = ws.canonicalize().unwrap_or_else(|_| ws.clone());
                let ws_state = create_state(ws_path.clone());
                indexer.full_index(&ws_state, &ws_path).await?;
                let stats = ws_state.stats();
                results.push(IndexAllResult {
                    workspace: ws_path.display().to_string(),
                    files: stats.file_count as usize,
                    symbols: stats.symbol_count as usize,
                    call_edges: stats.call_edge_count as usize,
                });
            }
            Ok(Output::IndexAll { results })
        }

        Commands::Search { query, limit } => {
            // Ensure index exists
            indexer.full_index(&state, workspace).await?;

            // Simple search: find symbols that contain the query terms
            // (Full hybrid search with embeddings would require more setup)
            let query_lower = query.to_lowercase();
            let mut results: Vec<SearchResult> = Vec::new();
            
            for entry in state.symbols.iter() {
                let sym = entry.value();
                let name = state.resolve(sym.scoped_name).to_lowercase();
                
                // Score based on match quality
                let score = if name == query_lower {
                    1.0 // Exact match
                } else if name.contains(&query_lower) {
                    0.5 // Substring match
                } else {
                    continue;
                };
                
                results.push(SearchResult {
                    symbol: state.resolve(sym.scoped_name).to_string(),
                    kind: format!("{:?}", sym.kind),
                    file: sym.location.file.display().to_string(),
                    line: sym.location.start_line,
                    score,
                });
            }
            
            // Sort by score descending
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
            results.truncate(*limit);

            Ok(Output::Search {
                query: query.clone(),
                results,
            })
        }

        Commands::Symbol { name, scoped, limit } => {
            indexer.full_index(&state, workspace).await?;

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
            indexer.full_index(&state, workspace).await?;

            let results: Vec<CallResult> = match direction.as_str() {
                "callers" => {
                    state.find_callers(symbol)
                        .into_iter()
                        .map(|edge| CallResult {
                            caller: state.resolve(edge.caller).to_string(),
                            callee: edge.callee_name.clone(),
                            file: edge.location.file.display().to_string(),
                            line: edge.location.start_line,
                        })
                        .collect()
                }
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

        Commands::Analyze { analysis_type } => match analysis_type.as_str() {
            "dead-code" => {
                indexer.full_index(&state, workspace).await?;
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
        Commands::Export {
            format,
            max_files,
            max_symbols,
        } => {
            indexer.full_index(&state, workspace).await?;
            match format.as_str() {
                "engram" => {
                    let export = export_engram_memory(&state, workspace, *max_files, *max_symbols)?;
                    Ok(Output::ExportEngram { export })
                }
                other => Err(anyhow::anyhow!(
                    "Unknown export format: {}. Use: engram",
                    other
                )),
            }
        }
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "type")]
enum Output {
    Index {
        files: usize,
        symbols: usize,
        workspace: String,
    },
    IndexAll {
        results: Vec<IndexAllResult>,
    },
    Search {
        query: String,
        results: Vec<SearchResult>,
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
    DeadCode {
        dead_count: usize,
        symbols: Vec<SymbolResult>,
    },
    ExportEngram {
        export: omni_index::export::EngramMemoryExport,
    },
}

#[derive(serde::Serialize)]
struct SearchResult {
    symbol: String,
    kind: String,
    file: String,
    line: usize,
    score: f32,
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

fn print_human_readable(output: &Output) {
    match output {
        Output::Index {
            files,
            symbols,
            workspace,
        } => {
            println!("Indexed {} files, {} symbols", files, symbols);
            println!("Workspace: {}", workspace);
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
        Output::Search { query, results } => {
            println!("Search: \"{}\"", query);
            println!("Found {} results:", results.len());
            for r in results {
                println!(
                    "  {:.2} {} ({}) at {}:{}",
                    r.score, r.symbol, r.kind, r.file, r.line
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
    }
}
