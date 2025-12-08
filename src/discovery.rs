//! File discovery module.
//!
//! Discovers source files in a repository while respecting .gitignore rules.

use anyhow::Result;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Discovers source files in a repository.
pub struct FileDiscovery {
    /// File extensions to include (e.g., "rs", "ts", "py")
    extensions: HashSet<String>,
    /// Additional ignore patterns
    ignore_patterns: Vec<String>,
}

impl Default for FileDiscovery {
    fn default() -> Self {
        let mut extensions = HashSet::new();
        extensions.insert("rs".to_string());
        Self {
            extensions,
            ignore_patterns: Vec::new(),
        }
    }
}

impl FileDiscovery {
    /// Create a new file discovery with default settings (Rust files only).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file extension to discover.
    pub fn with_extension(mut self, ext: &str) -> Self {
        self.extensions.insert(ext.to_lowercase());
        self
    }

    /// Add an ignore pattern.
    pub fn with_ignore(mut self, pattern: &str) -> Self {
        self.ignore_patterns.push(pattern.to_string());
        self
    }

    /// Discover all matching files under the given root.
    pub fn discover(&self, root: &Path) -> Result<Vec<PathBuf>> {
        // Build overrides for common build directories that should always be ignored
        let mut overrides = OverrideBuilder::new(root);

        // Always ignore these directories (! prefix means exclude/negate matching)
        overrides.add("!target/")?;
        overrides.add("!node_modules/")?;
        overrides.add("!.git/")?;

        // Add user-specified ignore patterns
        for pattern in &self.ignore_patterns {
            overrides.add(pattern)?;
        }

        let overrides = overrides.build()?;

        // Build walker with .gitignore support
        let walker = WalkBuilder::new(root)
            .hidden(false) // Skip hidden files by default
            .git_ignore(true) // Respect .gitignore
            .git_global(true) // Respect global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .require_git(false) // Parse .gitignore even without .git directory
            .overrides(overrides) // Apply our override patterns
            .build();

        // Collect files matching our extension filter
        let mut files = Vec::<PathBuf>::new();

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();

            // Check if file has one of our target extensions
            if self.should_include(path) {
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    /// Check if a file should be included based on extension.
    pub fn should_include(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| self.extensions.contains(&e.to_lowercase()))
            .unwrap_or(false)
    }
}
