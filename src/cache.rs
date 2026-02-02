use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CACHE_DIR: &str = ".omni";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const STATE_FILE: &str = "state.bin";
pub const BM25_FILE: &str = "bm25.bin";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileFingerprint {
    pub mtime_ms: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexManifest {
    pub tool_version: String,
    pub root: Option<String>,
    pub files: HashMap<String, FileFingerprint>,
}

pub fn cache_dir(root: &Path) -> PathBuf {
    root.join(CACHE_DIR)
}

pub fn ensure_cache_dir(root: &Path) -> Result<PathBuf> {
    let dir = cache_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create cache dir: {}", dir.display()))?;
    Ok(dir)
}

pub fn manifest_path(root: &Path) -> PathBuf {
    cache_dir(root).join(MANIFEST_FILE)
}

pub fn state_path(root: &Path) -> PathBuf {
    cache_dir(root).join(STATE_FILE)
}

pub fn bm25_path(root: &Path) -> PathBuf {
    cache_dir(root).join(BM25_FILE)
}

pub fn load_manifest(root: &Path) -> Result<Option<IndexManifest>> {
    let path = manifest_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read(&path).with_context(|| format!("Failed to read manifest: {}", path.display()))?;
    let manifest: IndexManifest = serde_json::from_slice(&data)
        .with_context(|| format!("Failed to parse manifest: {}", path.display()))?;
    Ok(Some(manifest))
}

pub fn save_manifest(root: &Path, manifest: &IndexManifest) -> Result<()> {
    ensure_cache_dir(root)?;
    let path = manifest_path(root);
    let data = serde_json::to_vec_pretty(manifest)?;
    fs::write(&path, data)
        .with_context(|| format!("Failed to write manifest: {}", path.display()))?;
    Ok(())
}

pub fn clear_cache(root: &Path) -> Result<()> {
    let dir = cache_dir(root);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to remove cache dir: {}", dir.display()))?;
    }
    Ok(())
}
