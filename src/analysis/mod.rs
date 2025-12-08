//! Code analysis modules.
//!
//! - Dead code detection
//! - Test coverage integration
//! - Churn analysis

pub mod churn;
pub mod coverage;
pub mod dead_code;

// Re-exports
pub use coverage::{BranchCoverage, CoverageAnalyzer, CoverageData, LineCoverage};
pub use dead_code::DeadCodeAnalyzer;
