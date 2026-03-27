//! Manifest module — responsible for building the archive blueprint
//! before any compression happens.

#[cfg(not(target_arch = "wasm32"))]
pub mod walker;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
// Removed PathBuf to avoid OS-specific types in the model
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Supported compression algorithms
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgo {
    Zstd,
    Lz4,
}

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

    /// Compression algorithm used for fragments.
    pub algo: CompressionAlgo,

    /// Whether the input was a directory (true) or a single file (false).
    pub is_directory: bool,

    /// All stream entries in byte-stream order.
    pub entries: Vec<StreamEntry>,

    /// Ordered list of compressed fragments representing the data in the archive.
    #[serde(default)]
    pub fragments: Vec<FragmentMeta>,

    /// O(1) fragment to entry lookup.
    /// `fragment_start_indices[f]` gives the index of the first `StreamEntry`
    /// containing data for fragment `f`.
    #[serde(default)]
    pub fragment_start_indices: Vec<usize>,
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

/// A generic logical stream entry (e.g., a file) in the archive.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StreamEntry {
    /// Generic identifier (e.g., relative path converted to standard string).
    pub identifier: String,

    /// Original stream size in bytes.
    pub original_size: u64,

    /// Unix permissions (mode bits).
    pub permissions: u32,

    /// Last modified time as unix timestamp.
    pub modified_at: u64,

    /// Byte offset where this entry's data starts in the contiguous byte stream.
    pub byte_offset: u64,

    /// Hex-encoded hash of the stream contents for integrity verification.
    pub checksum: Option<String>,

    /// If this entry is a symlink, the target identifier. None for regular streams.
    pub symlink_target: Option<String>,
}

/// Assign sequential byte offsets to each entry, and compute fragment start indices.
pub fn compute_offsets_and_indices(entries: &mut [StreamEntry], fragment_size: u64) -> Vec<usize> {
    let mut offset: u64 = 0;
    let mut fragment_start_indices = Vec::new();

    for (i, entry) in entries.iter_mut().enumerate() {
        entry.byte_offset = offset;
        
        if entry.original_size > 0 {
            let end_frag = ((offset + entry.original_size - 1) / fragment_size) as usize;

            while fragment_start_indices.len() <= end_frag {
                fragment_start_indices.push(i);
            }
        } else if fragment_start_indices.is_empty() {
            fragment_start_indices.push(i);
        }

        offset += entry.original_size;
    }

    fragment_start_indices
}

/// Create a `StreamEntry` from a file's metadata and an identifier.
#[cfg(not(target_arch = "wasm32"))]
pub fn entry_from_metadata(identifier: String, metadata: &std::fs::Metadata) -> StreamEntry {
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

    StreamEntry {
        identifier,
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
#[cfg(not(target_arch = "wasm32"))]
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
        let metadata = root
            .metadata()
            .with_context(|| format!("failed to read metadata: {}", root.display()))?;

        let file_name = root
            .file_name()
            .with_context(|| format!("path has no filename: {}", root.display()))?
            .to_string_lossy()
            .into_owned();

        vec![entry_from_metadata(file_name, &metadata)]
    };

    let fragment_start_indices = compute_offsets_and_indices(&mut entries, fragment_size);

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
        algo: CompressionAlgo::Zstd, // Defaulting to native target choice
        is_directory,
        entries,
        fragments: Vec::new(),
        fragment_start_indices,
    })
}

/// Serialize a manifest to a JSON file at the given path.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_manifest(manifest: &Manifest, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest)
        .context("failed to serialize manifest")?;
    std::fs::write(path, json)
        .with_context(|| format!("failed to write manifest to {}", path.display()))?;
    Ok(())
}

/// Load a manifest from a JSON file at the given path.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_manifest(path: &Path) -> Result<Manifest> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest from {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .context("failed to parse manifest JSON")?;
    Ok(manifest)
}
