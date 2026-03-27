use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use core::stream::{FragmentReader, StreamProvider};
use core::compressor::compress_chunk;
use core::manifest::{FragmentMeta, Manifest};

/// A local disk provider for FragmentReader when compiling for native environments.
#[derive(Clone)]
pub struct FileSystemProvider {
    root: PathBuf,
}

impl FileSystemProvider {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }
}

impl StreamProvider<File> for FileSystemProvider {
    fn provide_stream(&self, identifier: &str) -> anyhow::Result<File> {
        let path = self.root.join(identifier);
        let file = File::open(&path)?;
        Ok(file)
    }
}

/// Compress an archive sequentially fetching fragments via `FragmentReader`
/// and processing them in parallel using Rayon.
pub fn compress_archive(
    input_dir: PathBuf,
    output_dir: PathBuf,
    manifest: Manifest,
) -> Result<Manifest> {
    let num_fragments = if manifest.total_original_size == 0 {
        0
    } else {
        ((manifest.total_original_size + manifest.fragment_size - 1) / manifest.fragment_size) as usize
    };

    let manifest_arc = Arc::new(manifest.clone());

    // Generate indices to iterate in parallel
    let fragment_indices: Vec<usize> = (0..num_fragments).collect();

    // Spawn rayon worker threads natively to process all chunks
    let metas: Result<Vec<(usize, FragmentMeta)>> = fragment_indices.into_par_iter().map(|idx| {
        let provider = FileSystemProvider::new(&input_dir);
        let mut reader = FragmentReader::new(provider, &manifest_arc, idx)
            .with_context(|| format!("failed to create reader for fragment {}", idx))?;
        
        let mut buffer = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut buffer)?;
        let original_size = buffer.len() as u64;

        // Compress entirely in-memory slice using the unified API
        let compressed_data = compress_chunk(&buffer, 3)
            .map_err(|e| anyhow::anyhow!("Compression failed for fragment {}: {}", idx, e))?;

        let compressed_size = compressed_data.len() as u64;

        let mut hasher = blake3::Hasher::new();
        hasher.update(&compressed_data);
        let checksum = hasher.finalize().to_hex().to_string();

        let fragment_filename = format!("fragment_{:06}.zst", idx);
        let fragment_path = output_dir.join(fragment_filename);
        let mut file = File::create(&fragment_path)?;
        file.write_all(&compressed_data)?;

        let meta = FragmentMeta {
            compressed_size,
            original_size,
            checksum,
        };

        Ok((idx, meta))
    }).collect();

    let mut result_metas = metas?;
    result_metas.sort_by_key(|(i, _)| *i);

    let mut final_manifest = manifest;
    final_manifest.fragments = result_metas.into_iter().map(|(_, m)| m).collect();
    
    Ok(final_manifest)
}
