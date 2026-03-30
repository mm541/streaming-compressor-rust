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

    /// Whether this fragment was actually compressed.
    /// If false, the fragment was stored raw (content-aware skipping).
    #[serde(default = "default_true")]
    pub is_compressed: bool,
}

fn default_true() -> bool { true }

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

    /// If this entry is a symlink, the target identifier. None for regular streams.
    pub symlink_target: Option<String>,
}
