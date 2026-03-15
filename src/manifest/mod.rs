//! Manifest module — responsible for building the archive blueprint
//! before any compression happens.

pub mod walker;

use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The top-level manifest describing an entire archive.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    /// Format version for future compatibility.
    pub version: u8,

    /// Unix timestamp when the archive was created.
    pub created_at: u64,

    /// Total size of all files before compression (bytes).
    pub total_original_size: u64,

    /// Configured fragment size in bytes.
    pub fragment_size: u64,

    /// Whether the input was a directory (true) or a single file (false).
    pub is_directory: bool,

    /// All file entries in byte-stream order.
    /// Directory structure is implicit from file paths.
    pub entries: Vec<Entry>,

    /// Ordered list of compressed fragments representing the data in the archive.
    #[serde(default)]
    pub fragments: Vec<FragmentMeta>,
}

/// Metadata about a single compressed chunk of the archive.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FragmentMeta {
    /// Compressed size in bytes.
    pub compressed_size: u64,

    /// Original (uncompressed) size in bytes. 
    /// Will match Manifest.fragment_size except for the last fragment.
    pub original_size: u64,

    /// Hex-encoded hash of the *compressed* fragment for fast disk corruption checks.
    pub checksum: String,
}

/// A single file entry in the archive.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entry {
    /// Path relative to the input root directory.
    pub relative_path: PathBuf,

    /// Original file size in bytes.
    pub original_size: u64,

    /// Unix permissions (mode bits).
    pub permissions: u32,

    /// Last modified time as unix timestamp.
    pub modified_at: u64,

    /// Byte offset where this file's data starts in the contiguous byte stream.
    pub byte_offset: u64,

    /// Hex-encoded hash of the file contents for integrity verification.
    /// Populated after reading file data, not during the initial directory walk.
    pub checksum: Option<String>,

    /// If this entry is a symlink, the path it points to. None for regular files.
    pub symlink_target: Option<PathBuf>,
}

/// Assign sequential byte offsets to each entry.
///
/// Each file's `byte_offset` is the cumulative sum of all preceding files' sizes.
/// Entries must already be in their final order (sorted by path).
pub fn compute_byte_offsets(entries: &mut [Entry]) {
    let mut offset: u64 = 0;
    for entry in entries.iter_mut() {
        entry.byte_offset = offset;
        offset += entry.original_size;
    }
}

/// Create an `Entry` from a file's metadata and relative path.
///
/// Shared helper used by both the directory walker and single-file handling.
pub fn entry_from_metadata(relative_path: PathBuf, metadata: &std::fs::Metadata) -> Entry {
    let modified_at = metadata
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    };

    #[cfg(not(unix))]
    let permissions = 0o644;

    Entry {
        relative_path,
        original_size: metadata.len(),
        permissions,
        modified_at,
        byte_offset: 0,
        checksum: None,
        symlink_target: None,
    }
}

/// Minimum allowed fragment size: 1 MB
pub const MIN_FRAGMENT_SIZE: u64 = 1024 * 1024;

/// Maximum allowed fragment size: 4 GB
pub const MAX_FRAGMENT_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Build a complete manifest from a directory or single file path.
///
/// Steps:
/// 1. Validate fragment size
/// 2. Detect if root is a file or directory
/// 3. Collect file entries (walk directory, or create single entry)
/// 4. Assign sequential byte offsets
/// 5. Compute total size
/// 6. Return the manifest
pub fn build_manifest(root: &Path, fragment_size: u64) -> Result<Manifest> {
    anyhow::ensure!(
        fragment_size >= MIN_FRAGMENT_SIZE && fragment_size <= MAX_FRAGMENT_SIZE,
        "fragment_size must be between {} bytes (1 MB) and {} bytes (4 GB), got {}",
        MIN_FRAGMENT_SIZE,
        MAX_FRAGMENT_SIZE,
        fragment_size
    );

    let root = root
        .canonicalize()
        .with_context(|| format!("path does not exist: {}", root.display()))?;

    let is_directory = root.is_dir();

    let mut entries = if is_directory {
        walker::walk_directory(&root)
            .with_context(|| format!("failed to walk directory: {}", root.display()))?
    } else {
        // Single file — create one entry directly
        let metadata = root
            .metadata()
            .with_context(|| format!("failed to read metadata: {}", root.display()))?;

        let file_name = root
            .file_name()
            .with_context(|| format!("path has no filename: {}", root.display()))?;

        vec![entry_from_metadata(PathBuf::from(file_name), &metadata)]
    };

    compute_byte_offsets(&mut entries);

    let total_original_size = entries.iter().map(|e| e.original_size).sum();

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(Manifest {
        version: 1,
        created_at,
        total_original_size,
        fragment_size,
        is_directory,
        entries,
        fragments: Vec::new(),
    })
}

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
