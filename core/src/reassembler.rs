use std::io::{Read, Write};
use std::sync::mpsc::Sender;

use anyhow::Result;
use rayon::prelude::*;
use crate::compressor::CompressionEngine;
use crate::manifest::Manifest;
use crate::progress::ProgressEvent;

const CHUNK_BUF_SIZE: usize = 64 * 1024;

thread_local! {
    /// Pooled 64 KB chunk buffer for ferrying decompressed data.
    static REASSEMBLER_CHUNK_BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; CHUNK_BUF_SIZE]);
}

/// Reads fragments sequentially, decompresses them via the provided engine,
/// and reconstructs original files keeping absolute minimal memory bounds.
///
/// This function is fully I/O-agnostic. The caller provides:
/// - `fragment_reader_factory`: Opens a readable stream for each compressed fragment.
/// - `file_writer_factory`: Creates a writable sink for each output file entry.
/// - `engine`: A `CompressionEngine` implementation for decompression.
///
/// Core never touches the filesystem directly.
pub fn extract_archive<RF, WF>(
    manifest: &Manifest,
    fragment_reader_factory: RF,
    file_writer_factory: WF,
    progress_tx: Option<Sender<ProgressEvent>>,
    engine: &dyn CompressionEngine,
) -> Result<()>
where
    RF: Fn(usize) -> Result<Box<dyn Read>>,
    WF: Fn(&str) -> Result<Box<dyn Write>>,
{
    let mut current_entry_idx = 0;
    let mut bytes_written_to_current_file = 0u64;
    let mut current_writer: Option<Box<dyn Write>> = None;

    for (frag_idx, frag_meta) in manifest.fragments.iter().enumerate() {
        let raw_reader = fragment_reader_factory(frag_idx)?;
        
        // Only decompress if the fragment was actually compressed
        let mut decoder: Box<dyn Read> = if frag_meta.is_compressed {
            engine.decompressing_reader(raw_reader)?
        } else {
            raw_reader
        };

        let mut data_offset = 0;
        let data_len = frag_meta.original_size;

        REASSEMBLER_CHUNK_BUF.with(|buf_cell| -> Result<()> {
            let mut chunk_buf = buf_cell.borrow_mut();

            while data_offset < data_len {
                if current_entry_idx >= manifest.entries.len() {
                    anyhow::bail!("Reached end of manifest entries but still have data to extract");
                }

                let entry = &manifest.entries[current_entry_idx];

                if entry.original_size == 0 {
                    let _ = file_writer_factory(&entry.identifier)?;
                    current_entry_idx += 1;
                    continue;
                }

                if current_writer.is_none() {
                    current_writer = Some(file_writer_factory(&entry.identifier)?);
                    bytes_written_to_current_file = 0;
                }

                let file_remaining = entry.original_size - bytes_written_to_current_file;
                let fragment_remaining = data_len - data_offset;

                let bytes_to_read = file_remaining.min(fragment_remaining).min(chunk_buf.len() as u64) as usize;

                let n = decoder.read(&mut chunk_buf[..bytes_to_read])?;
                if n == 0 {
                    anyhow::bail!("Unexpected EOF while streaming decompressor fragment {}", frag_idx);
                }

                current_writer.as_mut().unwrap().write_all(&chunk_buf[..n])?;

                data_offset += n as u64;
                bytes_written_to_current_file += n as u64;

                if bytes_written_to_current_file == entry.original_size {
                    current_writer = None;
                    current_entry_idx += 1;
                }
            }

            // Decoder should hit EOF clean
            let mut sink = [0u8; 1];
            if decoder.read(&mut sink)? != 0 {
                anyhow::bail!("Decompressor produced more bytes than registered in Manifest metadata.");
            }

            Ok(())
        })?;

        if let Some(tx) = &progress_tx {
            let _ = tx.send(ProgressEvent::FragmentCompleted {
                idx: frag_idx,
                original_size: frag_meta.original_size,
                compressed_size: frag_meta.compressed_size,
            });
        }
    }

    Ok(())
}

/// Reads fragments sequentially in chunks, decompresses them in parallel via Rayon,
/// and reconstructs original files keeping memory bounds strictly to O(N * fragment_size).
pub fn parallel_extract_archive<RF, WF>(
    manifest: &Manifest,
    fragment_reader_factory: RF,
    file_writer_factory: WF,
    progress_tx: Option<Sender<ProgressEvent>>,
    engine: &(dyn CompressionEngine + Sync),
) -> Result<()>
where
    RF: Fn(usize) -> Result<Box<dyn Read>> + Send + Sync,
    WF: Fn(&str) -> Result<Box<dyn Write>>,
{
    let chunk_size = rayon::current_num_threads();

    let mut current_entry_idx = 0;
    let mut bytes_written_to_current_file = 0u64;
    let mut current_writer: Option<Box<dyn Write>> = None;

    for (chunk_idx, fragment_chunk) in manifest.fragments.chunks(chunk_size).enumerate() {
        let base_idx = chunk_idx * chunk_size;

        // Step 1: Parallel decompress this chunk
        let decompressed_chunk: Result<Vec<Vec<u8>>> = fragment_chunk
            .into_par_iter()
            .enumerate()
            .map(|(iter_idx, frag_meta)| {
                let frag_idx = base_idx + iter_idx;
                let raw_reader = fragment_reader_factory(frag_idx)?;
                let mut decoder: Box<dyn Read> = if frag_meta.is_compressed {
                    engine.decompressing_reader(raw_reader)?
                } else {
                    raw_reader
                };

                // Read exactly frag_meta.original_size bytes
                let len = frag_meta.original_size as usize;
                let mut buf = vec![0u8; len];
                decoder.read_exact(&mut buf)?;

                // Ensure it hit EOF cleanly
                let mut sink = [0u8; 1];
                if decoder.read(&mut sink)? != 0 {
                    anyhow::bail!("Decompressor produced more bytes than registered in Manifest metadata.");
                }

                Ok(buf)
            })
            .collect();

        let decompressed_chunk = decompressed_chunk?;

        // Step 2: Sequentially write out the chunk
        for (iter_idx, frag_meta) in fragment_chunk.iter().enumerate() {
            let frag_idx = base_idx + iter_idx;
            let data = &decompressed_chunk[iter_idx]; // perfectly aligned!

            let mut data_offset = 0;
            let data_len = frag_meta.original_size;

            while data_offset < data_len {
                if current_entry_idx >= manifest.entries.len() {
                    anyhow::bail!("Reached end of manifest entries but still have data to extract");
                }

                let entry = &manifest.entries[current_entry_idx];

                if entry.original_size == 0 {
                    let _ = file_writer_factory(&entry.identifier)?;
                    current_entry_idx += 1;
                    continue;
                }

                if current_writer.is_none() {
                    current_writer = Some(file_writer_factory(&entry.identifier)?);
                    bytes_written_to_current_file = 0;
                }

                let file_remaining = entry.original_size - bytes_written_to_current_file;
                let fragment_remaining = data_len - data_offset;
                let bytes_to_write = file_remaining.min(fragment_remaining) as usize;

                let start_idx = data_offset as usize;
                let end_idx = start_idx + bytes_to_write;

                current_writer.as_mut().unwrap().write_all(&data[start_idx..end_idx])?;

                data_offset += bytes_to_write as u64;
                bytes_written_to_current_file += bytes_to_write as u64;

                if bytes_written_to_current_file == entry.original_size {
                    current_writer = None;
                    current_entry_idx += 1;
                }
            }

            if let Some(tx) = &progress_tx {
                let _ = tx.send(ProgressEvent::FragmentCompleted {
                    idx: frag_idx,
                    original_size: frag_meta.original_size,
                    compressed_size: frag_meta.compressed_size,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use crate::compressor::ZstdEngine;
    use crate::manifest::build_manifest;
    use crate::publisher::compress_archive;
    use crate::stream::StreamProvider;
    use tempfile::TempDir;

    /// Filesystem provider used only in tests
    #[derive(Clone)]
    struct TestFsProvider { root: std::path::PathBuf }

    impl StreamProvider<File> for TestFsProvider {
        fn provide_stream(&self, identifier: &str) -> anyhow::Result<File> {
            Ok(File::open(self.root.join(identifier))?)
        }
    }

    #[test]
    fn test_extract_archive_end_to_end_streaming() {
        let input_dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        fs::write(input_dir.path().join("f1.txt"), b"first file data").unwrap();
        fs::write(input_dir.path().join("f2.txt"), b"second somewhat longer file").unwrap();
        fs::create_dir_all(input_dir.path().join("sub")).unwrap();
        fs::write(input_dir.path().join("sub/f3.txt"), b"in a subdir").unwrap();
        fs::write(input_dir.path().join("empty.txt"), b"").unwrap();

        let mut manifest = build_manifest(input_dir.path(), Some(1024 * 1024)).unwrap(); 
        manifest.fragment_size = 16;
        manifest.fragment_start_indices = crate::manifest::builder::compute_offsets_and_indices(&mut manifest.entries, 16);

        let provider = TestFsProvider { root: input_dir.path().to_path_buf() };
        let archive_path = archive_dir.path().to_path_buf();
        let writer_factory = {
            let p = archive_path.clone();
            move |idx: usize| -> anyhow::Result<File> {
                Ok(File::create(p.join(format!("fragment_{:06}.zst", idx)))?)
            }
        };

        let engine = ZstdEngine::new(3);

        let manifest = compress_archive(
            provider,
            manifest,
            writer_factory,
            None,
            &engine,
            None,
        ).unwrap();

        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(archive_dir.path().join("manifest.json"), manifest_json).unwrap();

        // Decompress
        let manifest_content = fs::read_to_string(archive_dir.path().join("manifest.json")).unwrap();
        let saved_manifest: Manifest = serde_json::from_str(&manifest_content).unwrap();
        
        let archive_p = archive_dir.path().to_path_buf();
        let output_p = output_dir.path().to_path_buf();

        extract_archive(
            &saved_manifest,
            |idx| -> anyhow::Result<Box<dyn Read>> {
                let path = archive_p.join(format!("fragment_{:06}.zst", idx));
                Ok(Box::new(File::open(&path)?))
            },
            |identifier| -> anyhow::Result<Box<dyn Write>> {
                let path = output_p.join(identifier);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                Ok(Box::new(File::create(&path)?))
            },
            None,
            &engine,
        ).unwrap();

        assert_eq!(fs::read_to_string(output_dir.path().join("f1.txt")).unwrap(), "first file data");
        assert_eq!(fs::read_to_string(output_dir.path().join("f2.txt")).unwrap(), "second somewhat longer file");
        assert_eq!(fs::read_to_string(output_dir.path().join("sub/f3.txt")).unwrap(), "in a subdir");
        assert_eq!(fs::read_to_string(output_dir.path().join("empty.txt")).unwrap(), "");
    }

    #[test]
    fn test_parallel_extract_archive_end_to_end_streaming() {
        let input_dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        fs::write(input_dir.path().join("f1.txt"), b"first file data").unwrap();
        fs::write(input_dir.path().join("f2.txt"), b"second somewhat longer file").unwrap();
        fs::create_dir_all(input_dir.path().join("sub")).unwrap();
        fs::write(input_dir.path().join("sub/f3.txt"), b"in a subdir").unwrap();
        fs::write(input_dir.path().join("empty.txt"), b"").unwrap();

        let mut manifest = build_manifest(input_dir.path(), Some(1024 * 1024)).unwrap(); 
        manifest.fragment_size = 16;
        manifest.fragment_start_indices = crate::manifest::builder::compute_offsets_and_indices(&mut manifest.entries, 16);

        let provider = TestFsProvider { root: input_dir.path().to_path_buf() };
        let archive_path = archive_dir.path().to_path_buf();
        let writer_factory = {
            let p = archive_path.clone();
            move |idx: usize| -> anyhow::Result<File> {
                Ok(File::create(p.join(format!("fragment_{:06}.zst", idx)))?)
            }
        };

        let engine = ZstdEngine::new(3);

        let manifest = compress_archive(
            provider,
            manifest,
            writer_factory,
            None,
            &engine,
            None,
        ).unwrap();

        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(archive_dir.path().join("manifest.json"), manifest_json).unwrap();

        // Decompress Parallel
        let manifest_content = fs::read_to_string(archive_dir.path().join("manifest.json")).unwrap();
        let saved_manifest: Manifest = serde_json::from_str(&manifest_content).unwrap();
        
        let archive_p = archive_dir.path().to_path_buf();
        let output_p = output_dir.path().to_path_buf();

        crate::reassembler::parallel_extract_archive(
            &saved_manifest,
            |idx| -> anyhow::Result<Box<dyn Read>> {
                let path = archive_p.join(format!("fragment_{:06}.zst", idx));
                Ok(Box::new(File::open(&path)?))
            },
            |identifier| -> anyhow::Result<Box<dyn Write>> {
                let path = output_p.join(identifier);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                Ok(Box::new(File::create(&path)?))
            },
            None,
            &engine,
        ).unwrap();

        assert_eq!(fs::read_to_string(output_dir.path().join("f1.txt")).unwrap(), "first file data");
        assert_eq!(fs::read_to_string(output_dir.path().join("f2.txt")).unwrap(), "second somewhat longer file");
        assert_eq!(fs::read_to_string(output_dir.path().join("sub/f3.txt")).unwrap(), "in a subdir");
        assert_eq!(fs::read_to_string(output_dir.path().join("empty.txt")).unwrap(), "");
    }
}
