//! Code analysis modules.
//!
//! - Dead code detection
//! - Test coverage integration
//! - Churn analysis

pub mod dead_code;
pub mod coverage;
pub mod churn;

// Re-exports
pub use dead_code::DeadCodeAnalyzer;
pub use coverage::{CoverageAnalyzer, CoverageData, LineCoverage, BranchCoverage};
