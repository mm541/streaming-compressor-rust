use std::path::Path;
use anyhow::{Context, Result};
use super::types::Manifest;

/// Serialize a manifest to a JSON file at the given path.
pub fn save_manifest(manifest: &Manifest, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest)
        .context("failed to serialize manifest")?;
    std::fs::write(path, json)
        .with_context(|| format!("failed to write manifest to {}", path.display()))?;
    Ok(())
}

/// Load a manifest from a JSON file at the given path.
pub fn load_manifest(path: &Path) -> Result<Manifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest from {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .context("failed to parse manifest JSON")?;
    Ok(manifest)
}
