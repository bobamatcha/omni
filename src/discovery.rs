//! File discovery module.
//!
//! Discovers source files in a repository while respecting .gitignore rules.

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use std::fs;
use std::path::{Path, PathBuf};

/// Discovers source files in a repository.
pub struct FileDiscovery {
    /// Additional include patterns
    include_patterns: Vec<String>,
    /// Additional ignore patterns
    exclude_patterns: Vec<String>,
    /// Whether to apply default excludes
    default_excludes: bool,
    /// Whether to include hidden files
    include_hidden: bool,
    /// Whether to include large files
    include_large: bool,
    /// Max file size (bytes) unless include_large is set
    max_file_size: u64,
}

impl Default for FileDiscovery {
    fn default() -> Self {
        Self {
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            default_excludes: true,
            include_hidden: false,
            include_large: false,
            max_file_size: 2 * 1024 * 1024,
        }
    }
}

impl FileDiscovery {
    /// Create a new file discovery with default settings (Rust files only).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an include pattern.
    pub fn with_include(mut self, pattern: &str) -> Self {
        self.include_patterns.push(pattern.to_string());
        self
    }

    /// Add an exclude pattern.
    pub fn with_exclude(mut self, pattern: &str) -> Self {
        self.exclude_patterns.push(pattern.to_string());
        self
    }

    /// Disable default excludes.
    pub fn without_default_excludes(mut self) -> Self {
        self.default_excludes = false;
        self
    }

    /// Include hidden files.
    pub fn include_hidden(mut self) -> Self {
        self.include_hidden = true;
        self
    }

    /// Include large files.
    pub fn include_large(mut self) -> Self {
        self.include_large = true;
        self
    }

    /// Override max file size.
    pub fn with_max_file_size(mut self, max_file_size: u64) -> Self {
        self.max_file_size = max_file_size;
        self
    }

    /// Discover all matching files under the given root.
    pub fn discover(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let default_excludes = if self.default_excludes {
            build_globset(default_exclude_patterns())?
        } else {
            GlobSetBuilder::new().build()?
        };

        let user_excludes = build_globset(self.exclude_patterns.iter().map(|s| s.as_str()))?;
        let user_includes = build_globset(self.include_patterns.iter().map(|s| s.as_str()))?;

        // Build walker with .gitignore support
        let walker = WalkBuilder::new(root)
            .hidden(!self.include_hidden) // Skip hidden files by default
            .git_ignore(true) // Respect .gitignore
            .git_global(true) // Respect global gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .require_git(false) // Parse .gitignore even without .git directory
            .build();

        // Collect files matching our extension filter
        let mut files = Vec::<PathBuf>::new();

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if !is_file {
                continue;
            }

            let rel = path.strip_prefix(root).unwrap_or(path);
            if is_excluded(rel, &default_excludes, &user_excludes, &user_includes) {
                continue;
            }

            if self.should_include(path) {
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    /// Check if a file should be included based on extension.
    pub fn should_include(&self, path: &Path) -> bool {
        if self.include_large {
            return true;
        }
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        metadata.len() <= self.max_file_size
    }
}

fn default_exclude_patterns() -> Vec<&'static str> {
    vec![
        "**/.git/**",
        "**/.omni/**",
        "**/target/**",
        "**/node_modules/**",
        "**/dist/**",
        "**/build/**",
        "**/out/**",
        "**/coverage/**",
        "**/vendor/**",
        "**/.venv/**",
        "**/.next/**",
        "**/package-lock.json",
        "**/yarn.lock",
        "**/pnpm-lock.yaml",
        "**/Cargo.lock",
        "**/*.min.js",
        "**/*.min.css",
        "**/*.map",
        "**/*.png",
        "**/*.jpg",
        "**/*.jpeg",
        "**/*.gif",
        "**/*.webp",
        "**/*.pdf",
        "**/*.zip",
        "**/*.gz",
        "**/*.tar",
        "**/*.tgz",
        "**/*.jar",
        "**/*.wasm",
        "**/*.o",
        "**/*.a",
        "**/*.so",
        "**/*.dylib",
        "**/*.dll",
    ]
}

fn build_globset<'a>(patterns: impl IntoIterator<Item = &'a str>) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

fn is_excluded(path: &Path, default: &GlobSet, user: &GlobSet, include: &GlobSet) -> bool {
    let is_included = include.is_match(path);
    let is_excluded = default.is_match(path) || user.is_match(path);
    is_excluded && !is_included
}
