use super::types::{StreamEntry, Manifest, CompressionAlgo};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Result;

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

/// Build a complete manifest from pre-collected entries.
///
/// This is the I/O-agnostic entry point. The caller (e.g., CLI) is responsible
/// for walking the filesystem and building the `Vec<StreamEntry>`. Core only
/// does the math: computing offsets, fragment sizes, and indices.
pub fn build_manifest_from_entries(
    mut entries: Vec<StreamEntry>,
    fragment_size: Option<u64>,
    is_directory: bool,
) -> Result<Manifest> {
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
        algo: CompressionAlgo::Zstd,
        is_directory,
        entries,
        fragments: Vec::new(),
        fragment_start_indices,
    })
}
