//! File churn analysis via git history.
//!
//! Analyzes git commit history to identify:
//! - Files with high change frequency (hotspots)
//! - Per-file churn metrics (commits, lines changed, authors)
//! - Code stability patterns

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A churn analyzer that uses git history to identify code hotspots.
pub struct ChurnAnalyzer;

impl ChurnAnalyzer {
    /// Analyze git history for the given repository root.
    ///
    /// # Arguments
    /// * `root` - The root directory of the git repository
    /// * `days` - Number of days of history to analyze
    ///
    /// # Returns
    /// A `ChurnReport` containing file churn metrics and hotspots.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The directory is not a git repository
    /// - Git commands fail to execute
    pub fn analyze(root: &Path, days: u32) -> Result<ChurnReport> {
        // Verify this is a git repository
        if !Self::is_git_repo(root)? {
            anyhow::bail!("Not a git repository: {}", root.display());
        }

        // Get list of all files changed in the time period
        let changed_files = Self::get_changed_files(root, days)?;

        // Build churn metrics for each file
        let mut file_churn = Vec::new();
        for file_path in changed_files {
            if let Ok(churn) = Self::analyze_file(root, &file_path, days) {
                file_churn.push(churn);
            }
            // Silently skip files that no longer exist or can't be analyzed
        }

        // Identify hotspots: files with high change frequency
        let mut hotspots: Vec<(PathBuf, u32)> = file_churn
            .iter()
            .filter(|fc| fc.commits > 3) // More than 3 commits = hotspot
            .map(|fc| (fc.path.clone(), fc.commits))
            .collect();

        // Sort hotspots by change frequency (descending)
        hotspots.sort_by(|a, b| b.1.cmp(&a.1));

        Ok(ChurnReport {
            file_churn,
            hotspots,
        })
    }

    /// Check if a directory is a git repository.
    fn is_git_repo(root: &Path) -> Result<bool> {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--git-dir")
            .current_dir(root)
            .output()
            .context("Failed to execute git command")?;

        Ok(output.status.success())
    }

    /// Get list of all files changed in the time period.
    fn get_changed_files(root: &Path, days: u32) -> Result<HashSet<PathBuf>> {
        let since = format!("{} days ago", days);

        let output = Command::new("git")
            .args(["log", "--since", &since, "--pretty=format:", "--name-only"])
            .current_dir(root)
            .output()
            .context("Failed to get changed files from git")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git log failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: HashSet<PathBuf> = stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| PathBuf::from(line.trim()))
            .collect();

        Ok(files)
    }

    /// Analyze a single file's churn metrics.
    fn analyze_file(root: &Path, file_path: &Path, days: u32) -> Result<FileChurn> {
        let since = format!("{} days ago", days);

        // Get commit hashes, authors, and dates for this file
        let log_output = Command::new("git")
            .args([
                "log",
                "--since",
                &since,
                "--pretty=format:%H|%an|%ad",
                "--date=iso",
                "--",
                file_path.to_str().unwrap_or(""),
            ])
            .current_dir(root)
            .output()
            .context("Failed to get git log for file")?;

        if !log_output.status.success() {
            anyhow::bail!("Git log failed for file: {}", file_path.display());
        }

        let log_stdout = String::from_utf8_lossy(&log_output.stdout);
        let commits: Vec<&str> = log_stdout.lines().filter(|line| !line.is_empty()).collect();

        let commit_count = commits.len() as u32;

        // Extract unique authors
        let mut authors = HashSet::new();
        let mut last_modified = String::new();

        for (idx, commit_line) in commits.iter().enumerate() {
            let parts: Vec<&str> = commit_line.split('|').collect();
            if parts.len() >= 3 {
                authors.insert(parts[1].to_string());
                // First entry is the most recent (git log is reverse chronological)
                if idx == 0 {
                    last_modified = parts[2].to_string();
                }
            }
        }

        let authors_vec: Vec<String> = authors.into_iter().collect();

        // Get line change statistics
        let (lines_added, lines_removed) = Self::get_line_stats(root, file_path, days)?;

        Ok(FileChurn {
            path: file_path.to_path_buf(),
            commits: commit_count,
            lines_added,
            lines_removed,
            authors: authors_vec,
            last_modified,
        })
    }

    /// Get line addition/deletion statistics for a file.
    fn get_line_stats(root: &Path, file_path: &Path, days: u32) -> Result<(u32, u32)> {
        let since = format!("{} days ago", days);

        let output = Command::new("git")
            .args([
                "log",
                "--since",
                &since,
                "--numstat",
                "--pretty=format:",
                "--",
                file_path.to_str().unwrap_or(""),
            ])
            .current_dir(root)
            .output()
            .context("Failed to get numstat for file")?;

        if !output.status.success() {
            // If numstat fails, return zeros rather than error
            return Ok((0, 0));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut total_added = 0u32;
        let mut total_removed = 0u32;

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                // Format: added removed filename
                // Handle binary files (shown as "-")
                if let Ok(added) = parts[0].parse::<u32>() {
                    total_added += added;
                }
                if let Ok(removed) = parts[1].parse::<u32>() {
                    total_removed += removed;
                }
            }
        }

        Ok((total_added, total_removed))
    }
}

/// Report containing churn analysis results.
#[derive(Debug, Clone)]
pub struct ChurnReport {
    /// Per-file churn metrics
    pub file_churn: Vec<FileChurn>,
    /// Files with high change frequency (more than 3 commits)
    pub hotspots: Vec<(PathBuf, u32)>,
}

/// Churn metrics for a single file.
#[derive(Debug, Clone)]
pub struct FileChurn {
    /// Path to the file (relative to repository root)
    pub path: PathBuf,
    /// Number of commits that modified this file
    pub commits: u32,
    /// Total lines added across all commits
    pub lines_added: u32,
    /// Total lines removed across all commits
    pub lines_removed: u32,
    /// Unique authors who modified this file
    pub authors: Vec<String>,
    /// ISO date of last modification
    pub last_modified: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a test git repository with some history.
    fn create_test_repo() -> Result<TempDir> {
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()?;

        // Configure git
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()?;
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()?;

        // Create and commit a file
        let test_file = repo_path.join("test.rs");
        fs::write(&test_file, "fn main() {}\n")?;

        Command::new("git")
            .args(["add", "test.rs"])
            .current_dir(repo_path)
            .output()?;
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()?;

        // Modify the file
        fs::write(&test_file, "fn main() {\n    println!(\"Hello\");\n}\n")?;
        Command::new("git")
            .args(["add", "test.rs"])
            .current_dir(repo_path)
            .output()?;
        Command::new("git")
            .args(["commit", "-m", "Add println"])
            .current_dir(repo_path)
            .output()?;

        Ok(temp_dir)
    }

    #[test]
    fn test_is_git_repo() {
        let temp_dir = TempDir::new().unwrap();

        // Not a git repo
        assert!(!ChurnAnalyzer::is_git_repo(temp_dir.path()).unwrap());

        // Initialize git
        Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .output()
            .unwrap();

        // Now it is a git repo
        assert!(ChurnAnalyzer::is_git_repo(temp_dir.path()).unwrap());
    }

    #[test]
    fn test_analyze_basic() {
        let temp_repo = create_test_repo().unwrap();
        let report = ChurnAnalyzer::analyze(temp_repo.path(), 365).unwrap();

        // Should have churn data for test.rs
        assert!(!report.file_churn.is_empty());

        let test_file_churn = report
            .file_churn
            .iter()
            .find(|fc| fc.path == PathBuf::from("test.rs"));

        assert!(test_file_churn.is_some());
        let churn = test_file_churn.unwrap();

        // Should have 2 commits
        assert_eq!(churn.commits, 2);

        // Should have 1 author
        assert_eq!(churn.authors.len(), 1);
        assert_eq!(churn.authors[0], "Test User");

        // Should have some lines added
        assert!(churn.lines_added > 0);
    }

    #[test]
    fn test_non_git_repo_error() {
        let temp_dir = TempDir::new().unwrap();
        let result = ChurnAnalyzer::analyze(temp_dir.path(), 30);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Not a git repository")
        );
    }

    #[test]
    fn test_hotspot_detection() {
        let temp_repo = create_test_repo().unwrap();
        let repo_path = temp_repo.path();

        // Create a hotspot by making multiple commits
        let hotspot_file = repo_path.join("hotspot.rs");
        for i in 1..=5 {
            fs::write(&hotspot_file, format!("// Version {}\n", i)).unwrap();
            Command::new("git")
                .args(["add", "hotspot.rs"])
                .current_dir(repo_path)
                .output()
                .unwrap();
            Command::new("git")
                .args(["commit", "-m", &format!("Update {}", i)])
                .current_dir(repo_path)
                .output()
                .unwrap();
        }

        let report = ChurnAnalyzer::analyze(repo_path, 365).unwrap();

        // hotspot.rs should be in the hotspots list (>3 commits)
        assert!(!report.hotspots.is_empty());
        let hotspot = report
            .hotspots
            .iter()
            .find(|(path, _)| path == &PathBuf::from("hotspot.rs"));

        assert!(hotspot.is_some());
        let (_, commit_count) = hotspot.unwrap();
        assert_eq!(*commit_count, 5);
    }
}
