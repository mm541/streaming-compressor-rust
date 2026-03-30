use super::types::StreamEntry;
#[cfg(not(target_arch = "wasm32"))]
use super::types::{Manifest, CompressionAlgo};

#[cfg(not(target_arch = "wasm32"))]
use super::walker;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};

/// Minimum allowed fragment size: 1 MB
pub const MIN_FRAGMENT_SIZE: u64 = 1024 * 1024;

/// Maximum allowed fragment size: 4 GB
pub const MAX_FRAGMENT_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Helper to compute an optimal fragment size based on total data and CPU cores.
/// 
/// Heuristic: Target ~4 fragments per CPU thread for good Rayon load balancing,
/// clamped between 1 MB and 64 MB.
pub fn optimal_fragment_size(total_size: u64, num_cpus: usize) -> u64 {
    if total_size == 0 || num_cpus == 0 {
        return MIN_FRAGMENT_SIZE;
    }
    let target_fragments = (num_cpus as u64) * 4;
    let computed_size = total_size / target_fragments;
    computed_size.clamp(MIN_FRAGMENT_SIZE, 64 * 1024 * 1024)
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
        symlink_target: None,
    }
}

/// Build a complete manifest from a directory or single file path.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_manifest(root: &Path, fragment_size: Option<u64>) -> Result<Manifest> {
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

    let total_original_size = entries.iter().map(|e| e.original_size).sum();

    let resolved_fragment_size = fragment_size.unwrap_or_else(|| {
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        optimal_fragment_size(total_original_size, cpus)
    });

    anyhow::ensure!(
        (MIN_FRAGMENT_SIZE..=MAX_FRAGMENT_SIZE).contains(&resolved_fragment_size),
        "fragment_size must be between {} bytes (1 MB) and {} bytes (4 GB), got {}",
        MIN_FRAGMENT_SIZE,
        MAX_FRAGMENT_SIZE,
        resolved_fragment_size
    );

    let fragment_start_indices = compute_offsets_and_indices(&mut entries, resolved_fragment_size);

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(Manifest {
        version: 1,
        created_at,
        total_original_size,
        fragment_size: resolved_fragment_size,
        algo: CompressionAlgo::Zstd, // Defaulting to native target choice
        is_directory,
        entries,
        fragments: Vec::new(),
        fragment_start_indices,
    })
}
