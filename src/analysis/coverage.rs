//! Test coverage integration.
//!
//! Supports LLVM coverage (from `cargo llvm-cov --json`) and Tarpaulin JSON formats.
//! Correlates coverage data with symbol definitions to provide per-symbol coverage metrics.

use crate::state::OciState;
use crate::types::SymbolCoverage;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================================
// Coverage Data Structures
// ============================================================================

/// Internal coverage data structure that stores coverage information.
#[derive(Debug, Clone, Default)]
pub struct CoverageData {
    /// Map from file path to line coverage
    pub line_coverage: HashMap<PathBuf, LineCoverage>,
    /// Map from file path to branch coverage
    pub branch_coverage: HashMap<PathBuf, BranchCoverage>,
}

/// Line coverage information for a file.
#[derive(Debug, Clone, Default)]
pub struct LineCoverage {
    /// Set of covered line numbers (1-indexed)
    pub covered_lines: Vec<usize>,
    /// Set of uncovered line numbers (1-indexed)
    pub uncovered_lines: Vec<usize>,
}

/// Branch coverage information for a file.
#[derive(Debug, Clone, Default)]
pub struct BranchCoverage {
    /// Map from line number to (covered, total) branches
    pub branches_per_line: HashMap<usize, (u32, u32)>,
}

// ============================================================================
// LLVM Coverage Format
// ============================================================================

/// LLVM coverage JSON format (from `cargo llvm-cov --json`).
#[derive(Debug, Deserialize)]
struct LlvmCoverageRoot {
    data: Vec<LlvmCoverageData>,
}

#[derive(Debug, Deserialize)]
struct LlvmCoverageData {
    files: Vec<LlvmCoverageFile>,
}

#[derive(Debug, Deserialize)]
struct LlvmCoverageFile {
    filename: String,
    #[serde(default)]
    segments: Vec<LlvmSegment>,
    #[serde(default)]
    branches: Vec<LlvmBranch>,
}

/// LLVM coverage segment: [line, col, count, has_count, is_region_entry, is_gap_region]
#[derive(Debug, Deserialize)]
struct LlvmSegment {
    #[serde(default)]
    line: usize,
    #[serde(default)]
    #[allow(dead_code)]
    col: usize,
    #[serde(default)]
    count: u64,
    #[serde(default)]
    has_count: bool,
    #[serde(default)]
    is_region_entry: bool,
}

/// LLVM branch coverage data
#[derive(Debug, Deserialize)]
struct LlvmBranch {
    #[serde(default)]
    line: usize,
    #[serde(default)]
    #[allow(dead_code)]
    count: u64,
    #[serde(default)]
    covered: bool,
}

// ============================================================================
// Tarpaulin Format
// ============================================================================

/// Tarpaulin JSON format.
#[derive(Debug, Deserialize)]
struct TarpaulinRoot {
    files: HashMap<String, TarpaulinFile>,
}

#[derive(Debug, Deserialize)]
struct TarpaulinFile {
    path: String,
    #[serde(default)]
    covered: Vec<usize>,
    #[serde(default)]
    uncovered: Vec<usize>,
}

// ============================================================================
// Coverage Analyzer
// ============================================================================

/// Coverage analyzer that integrates coverage data with the symbol index.
pub struct CoverageAnalyzer;

impl CoverageAnalyzer {
    /// Load coverage data from LLVM coverage JSON format.
    ///
    /// This parses the JSON output from `cargo llvm-cov --json`.
    pub fn load_llvm_cov(path: &Path) -> Result<CoverageData> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read LLVM coverage file: {:?}", path))?;

        let root: LlvmCoverageRoot = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse LLVM coverage JSON: {:?}", path))?;

        let mut coverage = CoverageData::default();

        for data in root.data {
            for file in data.files {
                let file_path = PathBuf::from(&file.filename);

                // Process segments to determine line coverage
                let mut line_execution_counts: HashMap<usize, u64> = HashMap::new();

                for segment in file.segments {
                    if segment.line > 0 && segment.has_count && segment.is_region_entry {
                        // Track the maximum execution count for each line
                        line_execution_counts
                            .entry(segment.line)
                            .and_modify(|c| *c = (*c).max(segment.count))
                            .or_insert(segment.count);
                    }
                }

                // Build line coverage
                let mut line_cov = LineCoverage::default();
                for (line, count) in line_execution_counts {
                    if count > 0 {
                        line_cov.covered_lines.push(line);
                    } else {
                        line_cov.uncovered_lines.push(line);
                    }
                }

                // Sort for consistent output
                line_cov.covered_lines.sort_unstable();
                line_cov.uncovered_lines.sort_unstable();

                coverage.line_coverage.insert(file_path.clone(), line_cov);

                // Process branches
                let mut branch_cov = BranchCoverage::default();
                for branch in file.branches {
                    if branch.line > 0 {
                        let (covered, total) = branch_cov
                            .branches_per_line
                            .entry(branch.line)
                            .or_insert((0, 0));

                        *total += 1;
                        if branch.covered {
                            *covered += 1;
                        }
                    }
                }

                if !branch_cov.branches_per_line.is_empty() {
                    coverage.branch_coverage.insert(file_path, branch_cov);
                }
            }
        }

        Ok(coverage)
    }

    /// Load coverage data from Tarpaulin JSON format.
    ///
    /// This parses the JSON output from `cargo tarpaulin --out Json`.
    pub fn load_tarpaulin(path: &Path) -> Result<CoverageData> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read Tarpaulin coverage file: {:?}", path))?;

        let root: TarpaulinRoot = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse Tarpaulin coverage JSON: {:?}", path))?;

        let mut coverage = CoverageData::default();

        for (_, file_data) in root.files {
            let file_path = PathBuf::from(&file_data.path);

            let mut line_cov = LineCoverage {
                covered_lines: file_data.covered.clone(),
                uncovered_lines: file_data.uncovered.clone(),
            };

            // Sort for consistent output
            line_cov.covered_lines.sort_unstable();
            line_cov.uncovered_lines.sort_unstable();

            coverage.line_coverage.insert(file_path, line_cov);
        }

        // Tarpaulin doesn't provide branch coverage in basic JSON format
        Ok(coverage)
    }

    /// Correlate coverage data with symbols in the index.
    ///
    /// For each symbol, this finds the file and line range, checks which lines
    /// are covered, and calculates coverage metrics.
    pub fn correlate_symbols(
        state: &OciState,
        coverage: &CoverageData,
    ) -> Vec<SymbolCoverage> {
        let mut results = Vec::new();

        // Iterate over all symbols
        for symbol_entry in state.symbols.iter() {
            let scoped_name = *symbol_entry.key();
            let symbol_def = symbol_entry.value();

            let location = &symbol_def.location;
            let file_path = &location.file;

            // Get coverage data for this file
            let line_cov = match coverage.line_coverage.get(file_path) {
                Some(cov) => cov,
                None => continue, // No coverage data for this file
            };

            let branch_cov = coverage.branch_coverage.get(file_path);

            // Determine the line range for this symbol
            let start_line = location.start_line;
            let end_line = location.end_line;

            if start_line == 0 || end_line == 0 {
                continue; // Invalid location
            }

            // Count covered and total lines
            let mut lines_covered = 0u32;
            let mut lines_total = 0u32;

            // Check each line in the symbol's range
            for line in start_line..=end_line {
                // Check if this line has coverage information
                let is_covered = line_cov.covered_lines.binary_search(&line).is_ok();
                let is_uncovered = line_cov.uncovered_lines.binary_search(&line).is_ok();

                if is_covered || is_uncovered {
                    lines_total += 1;
                    if is_covered {
                        lines_covered += 1;
                    }
                }
            }

            // Count branches in this symbol's range
            let mut branches_covered = 0u32;
            let mut branches_total = 0u32;

            if let Some(branch_data) = branch_cov {
                for line in start_line..=end_line {
                    if let Some(&(covered, total)) = branch_data.branches_per_line.get(&line) {
                        branches_covered += covered;
                        branches_total += total;
                    }
                }
            }

            // Only include symbols that have coverage data
            if lines_total > 0 || branches_total > 0 {
                results.push(SymbolCoverage {
                    symbol: scoped_name,
                    lines_covered,
                    lines_total,
                    branches_covered,
                    branches_total,
                });
            }
        }

        results
    }

    /// Calculate coverage percentage for a symbol.
    pub fn coverage_percentage(coverage: &SymbolCoverage) -> f32 {
        if coverage.lines_total == 0 {
            return 0.0;
        }
        (coverage.lines_covered as f32) / (coverage.lines_total as f32)
    }

    /// Calculate branch coverage percentage for a symbol.
    pub fn branch_coverage_percentage(coverage: &SymbolCoverage) -> f32 {
        if coverage.branches_total == 0 {
            return 0.0;
        }
        (coverage.branches_covered as f32) / (coverage.branches_total as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::create_state;
    use crate::types::{InternedString, Location, SymbolDef, SymbolKind, Visibility};
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_llvm_cov() {
        let llvm_json = r#"{
            "data": [{
                "files": [{
                    "filename": "/path/to/src/main.rs",
                    "segments": [
                        {"line": 10, "col": 1, "count": 5, "has_count": true, "is_region_entry": true},
                        {"line": 11, "col": 1, "count": 0, "has_count": true, "is_region_entry": true},
                        {"line": 12, "col": 1, "count": 3, "has_count": true, "is_region_entry": true}
                    ],
                    "branches": [
                        {"line": 10, "count": 2, "covered": true},
                        {"line": 10, "count": 0, "covered": false}
                    ]
                }]
            }]
        }"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(llvm_json.as_bytes()).unwrap();

        let coverage = CoverageAnalyzer::load_llvm_cov(temp_file.path()).unwrap();

        let file_path = PathBuf::from("/path/to/src/main.rs");
        let line_cov = coverage.line_coverage.get(&file_path).unwrap();

        assert_eq!(line_cov.covered_lines, vec![10, 12]);
        assert_eq!(line_cov.uncovered_lines, vec![11]);

        let branch_cov = coverage.branch_coverage.get(&file_path).unwrap();
        assert_eq!(branch_cov.branches_per_line.get(&10), Some(&(1, 2)));
    }

    #[test]
    fn test_load_tarpaulin() {
        let tarpaulin_json = r#"{
            "files": {
                "main.rs": {
                    "path": "/path/to/src/main.rs",
                    "covered": [10, 12, 15],
                    "uncovered": [11, 13]
                }
            }
        }"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(tarpaulin_json.as_bytes()).unwrap();

        let coverage = CoverageAnalyzer::load_tarpaulin(temp_file.path()).unwrap();

        let file_path = PathBuf::from("/path/to/src/main.rs");
        let line_cov = coverage.line_coverage.get(&file_path).unwrap();

        assert_eq!(line_cov.covered_lines, vec![10, 12, 15]);
        assert_eq!(line_cov.uncovered_lines, vec![11, 13]);
    }

    #[test]
    fn test_correlate_symbols() {
        let state = create_state(PathBuf::from("/test"));

        // Add a test symbol
        let file_path = PathBuf::from("/path/to/src/main.rs");
        let location = Location::new(file_path.clone(), 0, 100)
            .with_positions(10, 0, 15, 0);

        let name = state.intern("test_function");
        let scoped = state.intern("crate::test_function");

        let symbol = SymbolDef {
            name,
            scoped_name: scoped,
            kind: SymbolKind::Function,
            location,
            signature: None,
            visibility: Visibility::Public,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        };

        state.add_symbol(symbol);

        // Create coverage data
        let mut coverage = CoverageData::default();
        coverage.line_coverage.insert(
            file_path,
            LineCoverage {
                covered_lines: vec![10, 11, 12],
                uncovered_lines: vec![13, 14],
            },
        );

        // Correlate
        let results = CoverageAnalyzer::correlate_symbols(&state, &coverage);

        assert_eq!(results.len(), 1);
        let sym_cov = &results[0];
        assert_eq!(sym_cov.symbol, scoped);
        assert_eq!(sym_cov.lines_covered, 3);
        assert_eq!(sym_cov.lines_total, 5);

        let percentage = CoverageAnalyzer::coverage_percentage(sym_cov);
        assert!((percentage - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_coverage_percentage() {
        let coverage = SymbolCoverage {
            symbol: InternedString::default(),
            lines_covered: 7,
            lines_total: 10,
            branches_covered: 3,
            branches_total: 4,
        };

        let line_pct = CoverageAnalyzer::coverage_percentage(&coverage);
        assert!((line_pct - 0.7).abs() < 0.01);

        let branch_pct = CoverageAnalyzer::branch_coverage_percentage(&coverage);
        assert!((branch_pct - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_empty_coverage() {
        let coverage = SymbolCoverage {
            symbol: InternedString::default(),
            lines_covered: 0,
            lines_total: 0,
            branches_covered: 0,
            branches_total: 0,
        };

        assert_eq!(CoverageAnalyzer::coverage_percentage(&coverage), 0.0);
        assert_eq!(CoverageAnalyzer::branch_coverage_percentage(&coverage), 0.0);
    }
}
