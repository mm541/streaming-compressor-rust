use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result};
use rayon::prelude::*;

thread_local! {
    /// 128 KB zero-allocation thread pool globally reserved specifically for uncompressed passthrough bypassing.
    static PASSTHROUGH_BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; 128 * 1024]);
}

use crate::stream::{ReadSeek, FragmentReader, StreamProvider};
use crate::compressor::CompressionEngine;
use crate::detection::is_compressible;
use crate::manifest::{FragmentMeta, Manifest};
use crate::progress::ProgressEvent;

/// Custom reader to track original size flowing through
struct TrackingReader<R> {
    inner: R,
    size: u64,
}

impl<R: Read> Read for TrackingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.size += n as u64;
        Ok(n)
    }
}

/// Custom writer to track compressed output size.
struct TrackingWriter<W> {
    inner: W,
    size: u64,
}

impl<W: Write> Write for TrackingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }
    
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Determine if a fragment's content is worth compressing.
///
/// Checks the entries that fall within this fragment's byte range.
/// If all entries are known pre-compressed formats, skip compression.
fn should_compress_fragment(manifest: &Manifest, fragment_idx: usize) -> bool {
    let frag_start = (fragment_idx as u64) * manifest.fragment_size;
    let frag_end = std::cmp::min(
        frag_start + manifest.fragment_size,
        manifest.total_original_size,
    );

    if frag_start >= frag_end {
        return true; // empty fragment — compress anyway (fast)
    }

    // Find all entries overlapping this fragment
    let start_idx = manifest
        .fragment_start_indices
        .get(fragment_idx)
        .copied()
        .unwrap_or(0);

    let mut all_incompressible = true;
    let mut has_data = false;

    for entry in &manifest.entries[start_idx..] {
        let entry_end = entry.byte_offset + entry.original_size;

        // Skip entries entirely before this fragment
        if entry_end <= frag_start {
            continue;
        }
        // Stop if entry starts after this fragment
        if entry.byte_offset >= frag_end {
            break;
        }
        // Skip zero-byte entries
        if entry.original_size == 0 {
            continue;
        }

        has_data = true;

        // Check extension-based detection (no header available at this stage)
        if is_compressible(&entry.identifier, &[]) {
            all_incompressible = false;
            break;
        }
    }

    // If no data entries found, compress (it's tiny/empty)
    if !has_data {
        return true;
    }

    // If ALL entries in this fragment are pre-compressed, skip
    !all_incompressible
}

/// Compress an archive in parallel using Rayon.
///
/// This function is fully I/O-agnostic. The caller provides:
/// - `provider`: A `StreamProvider` that yields readable streams for source files.
/// - `writer_factory`: A closure that creates a writable sink for each compressed fragment.
/// - `engine`: A `CompressionEngine` implementation (e.g., `ZstdEngine`).
///
/// Content-aware skipping: fragments containing only pre-compressed files
/// (JPEG, MP4, ZIP, etc.) are stored raw without re-compression.
///
/// Core never touches the filesystem directly.
pub fn compress_archive<R, W, SP, WF>(
    provider: SP,
    manifest: Manifest,
    writer_factory: WF,
    progress_tx: Option<Sender<ProgressEvent>>,
    engine: &(dyn CompressionEngine + Sync),
    skip_map: Option<std::collections::HashMap<usize, u64>>,
) -> Result<Manifest>
where
    R: ReadSeek,
    W: Write + Send,
    SP: StreamProvider<R> + Clone + Send + Sync,
    WF: Fn(usize) -> Result<W> + Send + Sync,
{
    let num_fragments = if manifest.total_original_size == 0 {
        0
    } else {
        manifest.total_original_size.div_ceil(manifest.fragment_size) as usize
    };

    let manifest_arc = Arc::new(manifest.clone());
    let skip_map = Arc::new(skip_map.unwrap_or_default());

    // Optimization: Asymmetric Thread Stragglers
    // Pre-calculate complexity (number of file boundaries overlapping each fragment)
    let mut fragment_indices: Vec<usize> = (0..num_fragments).collect();
    let mut fragment_complexities = vec![0_usize; num_fragments];
    
    for idx in 0..num_fragments {
        let frag_start = (idx as u64) * manifest_arc.fragment_size;
        let frag_end = frag_start + manifest_arc.fragment_size;
        let start_idx = manifest_arc.fragment_start_indices.get(idx).copied().unwrap_or(0);
        
        let mut count = 0;
        for entry in &manifest_arc.entries[start_idx..] {
            if entry.byte_offset >= frag_end { break; }
            if entry.byte_offset + entry.original_size <= frag_start { continue; }
            count += 1;
        }
        fragment_complexities[idx] = count;
    }
    
    // Sort descending so Rayon pulls the fragments with the most file metadata/seeking bounds FIRST.
    // This perfectly masks the long-tail blocking delay of heavily fragmented chunks.
    fragment_indices.sort_by_cached_key(|&idx| std::cmp::Reverse(fragment_complexities[idx]));

    let metas: Result<Vec<(usize, FragmentMeta)>> = fragment_indices
        .into_par_iter()
        .map_with(progress_tx, |tx, idx| {
            if let Some(chan) = tx.as_ref() {
                let _ = chan.send(ProgressEvent::FragmentStarted {
                    idx,
                    total_fragments: num_fragments,
                });
            }

            // Check if fragment is already on disk and we should skip
            if let Some(compressed_size) = skip_map.get(&idx).copied() {
                let is_compressed = should_compress_fragment(&manifest_arc, idx);
                let original_size = if idx == num_fragments - 1 {
                    let m = &*manifest_arc;
                    let rem = m.total_original_size % m.fragment_size;
                    if rem == 0 { m.fragment_size } else { rem }
                } else {
                    manifest_arc.fragment_size
                };

                if let Some(chan) = tx.as_ref() {
                    let _ = chan.send(ProgressEvent::FragmentCompleted {
                        idx,
                        original_size,
                        compressed_size,
                    });
                }

                return Ok((idx, FragmentMeta {
                    compressed_size,
                    original_size,
                    is_compressed,
                }));
            }

            let reader = FragmentReader::new(provider.clone(), &manifest_arc, idx)
                .with_context(|| format!("failed to create reader for fragment {}", idx))?;
            
            let mut tracking_reader = TrackingReader { inner: reader, size: 0 };
            
            let output = writer_factory(idx)
                .with_context(|| format!("failed to create writer for fragment {}", idx))?;
            
            let mut tracking_writer = TrackingWriter { inner: output, size: 0 };

            let compress = should_compress_fragment(&manifest_arc, idx);

            if compress {
                engine.compress(&mut tracking_reader, &mut tracking_writer)
                    .map_err(|e| anyhow::anyhow!("Compression failed for fragment {}: {}", idx, e))?;
            } else {
                // Optimization: Passthrough Loop Buffers
                // Inject the massive 128KB thread-local slice natively to override std::io::copy's tiny 8KB buffer allocations.
                PASSTHROUGH_BUF.with(|buf_cell| -> Result<()> {
                    let mut buf = buf_cell.borrow_mut();
                    loop {
                        let n = tracking_reader.read(&mut buf)
                            .map_err(|e| anyhow::anyhow!("Passthrough read failed: {}", e))?;
                        if n == 0 { break; }
                        tracking_writer.write_all(&buf[..n])
                            .map_err(|e| anyhow::anyhow!("Passthrough write failed: {}", e))?;
                    }
                    Ok(())
                })
                .map_err(|e| anyhow::anyhow!("Passthrough copy failed for fragment {}: {}", idx, e))?;
            }

            let original_size = tracking_reader.size;
            let compressed_size = tracking_writer.size;

            if let Some(chan) = tx {
                let _ = chan.send(ProgressEvent::FragmentCompleted {
                    idx,
                    original_size,
                    compressed_size,
                });
            }

            Ok((idx, FragmentMeta {
                compressed_size,
                original_size,
                is_compressed: compress,
            }))
        })
        .collect();

    let mut result_metas = metas?;
    result_metas.sort_by_key(|(i, _)| *i);

    let mut final_manifest = manifest;
    final_manifest.fragments = result_metas.into_iter().map(|(_, m)| m).collect();
    
    Ok(final_manifest)
}
