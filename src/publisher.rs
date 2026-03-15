use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};

use crate::assembler::FragmentReader;
use crate::compressor::compress_stream;
use crate::manifest::{FragmentMeta, Manifest};

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// Events emitted on a single worker channel.  
/// Each channel processes fragments **one at a time**, so the consumer always
/// knows which fragment the `Chunk` belongs to from the preceding `Start`.
#[derive(Debug)]
pub enum CompressEvent {
    /// A new fragment is about to stream on this channel.
    Start { fragment_idx: usize },
    /// A small slice of compressed bytes (typically 8–64 KB from ZSTD internals).
    Chunk { data: Vec<u8> },
    /// The fragment that was being streamed is now complete.
    Complete { fragment_idx: usize, meta: FragmentMeta },
}

/// Result of calling [`start_pipeline`].  
/// `receivers` has exactly `N` entries (one per worker thread).  
/// `handle` joins when all fragments are done; returns the populated `Manifest`.
pub struct Pipeline {
    pub receivers: Vec<mpsc::Receiver<Result<CompressEvent>>>,
    pub handle: thread::JoinHandle<Result<Manifest>>,
}

// ---------------------------------------------------------------------------
// ChannelWriter — bridges ZSTD's Write to an mpsc channel, hashing on the fly
// ---------------------------------------------------------------------------

struct ChannelWriter {
    tx: mpsc::SyncSender<Result<CompressEvent>>,
    hasher: blake3::Hasher,
    len: u64,
}

impl ChannelWriter {
    fn new(tx: mpsc::SyncSender<Result<CompressEvent>>) -> Self {
        Self { tx, hasher: blake3::Hasher::new(), len: 0 }
    }

    fn finalize(self) -> (String, u64) {
        (self.hasher.finalize().to_hex().to_string(), self.len)
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len();
        self.hasher.update(buf);
        self.len += n as u64;

        self.tx
            .send(Ok(CompressEvent::Chunk { data: buf.to_vec() }))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "receiver dropped"))?;

        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spin up `N` worker threads, each with its own bounded channel.
///
/// * `concurrency` – number of worker threads.  
///   Pass `None` to auto-detect from `std::thread::available_parallelism()`.
/// * `channel_bound` – bounded capacity per channel (backpressure buffer).
///
/// Each worker pulls fragment indices from a shared atomic counter, so
/// fragments are distributed on-demand (work-stealing style).  
/// A single worker fully completes one fragment (`Start` → `Chunk`… → `Complete`)
/// before moving to the next, so the consumer on that channel never sees
/// interleaved data from different fragments.
pub fn start_pipeline(
    input_dir: PathBuf,
    manifest: Manifest,
    concurrency: Option<usize>,
    channel_bound: usize,
) -> Pipeline {
    let n_workers = concurrency.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
    });

    let num_fragments = if manifest.total_original_size == 0 {
        0
    } else {
        ((manifest.total_original_size + manifest.fragment_size - 1) / manifest.fragment_size) as usize
    };

    // Never spawn more threads than fragments — idle threads waste RAM on ZSTD state + channel buffers
    let n_workers = n_workers.min(num_fragments).max(1);

    // Shared atomic counter — each worker grabs the next unprocessed fragment index
    let next_fragment = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let manifest_arc = Arc::new(manifest);

    let mut receivers = Vec::with_capacity(n_workers);
    let mut worker_handles = Vec::with_capacity(n_workers);

    for _ in 0..n_workers {
        let (tx, rx) = mpsc::sync_channel(channel_bound);
        receivers.push(rx);

        let next = Arc::clone(&next_fragment);
        let manifest_ref = Arc::clone(&manifest_arc);
        let root = input_dir.clone();

        let handle = thread::spawn(move || -> Result<Vec<(usize, FragmentMeta)>> {
            let mut metas = Vec::new();

            loop {
                let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if idx >= num_fragments {
                    break;
                }

                // Tell consumer: "a new fragment is starting on this channel"
                if tx.send(Ok(CompressEvent::Start { fragment_idx: idx })).is_err() {
                    break; // consumer dropped
                }

                // FragmentReader seeks to the exact byte offset on disk.
                // It implements Read and only yields whatever the caller asks
                // for (ZSTD asks for ~8 KB at a time). Nothing large in RAM.
                let mut reader = FragmentReader::new(&root, &manifest_ref, idx)
                    .with_context(|| format!("failed to create reader for fragment {}", idx))?;

                let mut writer = ChannelWriter::new(tx.clone());

                // ZSTD pulls 8 KB from reader, compresses, pushes small
                // chunks into the channel via `writer.write()`. O(KB) RAM.
                compress_stream(&mut reader, &mut writer, 3)?;

                let (checksum, compressed_size) = writer.finalize();

                let virtual_start = (idx as u64) * manifest_ref.fragment_size;
                let virtual_end = std::cmp::min(
                    virtual_start + manifest_ref.fragment_size,
                    manifest_ref.total_original_size,
                );

                let meta = FragmentMeta {
                    compressed_size,
                    original_size: virtual_end - virtual_start,
                    checksum,
                };

                let _ = tx.send(Ok(CompressEvent::Complete {
                    fragment_idx: idx,
                    meta: meta.clone(),
                }));

                metas.push((idx, meta));
            }

            Ok(metas)
        });

        worker_handles.push(handle);
    }

    // Coordinator thread: joins all workers, assembles final Manifest
    let handle = thread::spawn(move || -> Result<Manifest> {
        let mut all_metas = Vec::with_capacity(num_fragments);

        for h in worker_handles {
            let worker_metas = h.join().map_err(|_| anyhow::anyhow!("worker panicked"))??;
            all_metas.extend(worker_metas);
        }

        all_metas.sort_by_key(|(i, _)| *i);

        let mut manifest = (*manifest_arc).clone();
        manifest.fragments = all_metas.into_iter().map(|(_, m)| m).collect();
        Ok(manifest)
    });

    Pipeline { receivers, handle }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::build_manifest;
    use tempfile::TempDir;

    #[test]
    fn test_pipeline_n_channels() {
        let input_dir = TempDir::new().unwrap();

        std::fs::write(input_dir.path().join("file1.txt"), b"123456789012345").unwrap();
        std::fs::write(input_dir.path().join("file2.txt"), b"abcdefghijklmnopqrstuvwxy").unwrap();

        let mut manifest = build_manifest(input_dir.path(), 1024 * 1024).unwrap();
        manifest.fragment_size = 16;

        // 2 workers, channel buffer of 10
        let pipeline = start_pipeline(input_dir.path().to_path_buf(), manifest, Some(2), 10);

        assert_eq!(pipeline.receivers.len(), 2);

        // Drain each receiver in its own thread (simulates independent consumers)
        let mut consumer_handles = Vec::new();
        for rx in pipeline.receivers {
            consumer_handles.push(std::thread::spawn(move || {
                let mut starts = 0usize;
                let mut chunks = 0usize;
                let mut completes = 0usize;
                for event in rx {
                    match event.unwrap() {
                        CompressEvent::Start { .. } => starts += 1,
                        CompressEvent::Chunk { .. } => chunks += 1,
                        CompressEvent::Complete { .. } => completes += 1,
                    }
                }
                (starts, chunks, completes)
            }));
        }

        let manifest = pipeline.handle.join().unwrap().unwrap();

        let mut total_starts = 0;
        let mut total_chunks = 0;
        let mut total_completes = 0;
        for h in consumer_handles {
            let (s, c, co) = h.join().unwrap();
            total_starts += s;
            total_chunks += c;
            total_completes += co;
        }

        // 3 fragments total (16 + 16 + 8 = 40 bytes from 15 + 25)
        assert_eq!(total_starts, 3);
        assert_eq!(total_completes, 3);
        assert!(total_chunks >= 3);
        assert_eq!(manifest.fragments.len(), 3);
        assert_eq!(manifest.fragments[0].original_size, 16);
        assert_eq!(manifest.fragments[1].original_size, 16);
        assert_eq!(manifest.fragments[2].original_size, 8);
    }
}
