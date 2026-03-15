use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use crate::manifest::Manifest;

/// Computes a Blake3 checksum while reading from an inner reader.
pub struct HashingReader<R: Read> {
    inner: R,
    hasher: blake3::Hasher,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R) -> Self {
        Self { 
            inner, 
            hasher: blake3::Hasher::new() 
        }
    }
    
    pub fn finalize(self) -> String {
        self.hasher.finalize().to_hex().to_string()
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// Reads fragments sequentially, decompresses them natively via streams, 
/// and reconstructs original files keeping absolute minimal memory bounds.
pub fn extract_archive(archive_dir: &Path, output_dir: &Path, manifest: &Manifest) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create extract directory: {}", output_dir.display()))?;

    let mut current_entry_idx = 0;
    let mut bytes_written_to_current_file = 0u64;
    let mut current_file: Option<File> = None;

    // Iterate through all fragments defined in the manifest in order
    for (frag_idx, frag_meta) in manifest.fragments.iter().enumerate() {
        let frag_path = archive_dir.join(format!("fragment_{:06}.zst", frag_idx));
        
        let f = File::open(&frag_path)
            .with_context(|| format!("failed to open fragment: {}", frag_path.display()))?;
            
        // We pass the hashing reader into the Decoder by mutable reference 
        // so we can access its state immediately after the stream drains.
        let mut hashing_reader = HashingReader::new(f);
        let mut decoder = zstd::stream::Decoder::new(&mut hashing_reader)?;

        let mut data_offset = 0;
        let data_len = frag_meta.original_size;
        
        // Use a fixed 64KB buffer strictly to ferry chunks along 
        let mut chunk_buf = vec![0u8; 64 * 1024];

        while data_offset < data_len {
            if current_entry_idx >= manifest.entries.len() {
                anyhow::bail!("Reached end of manifest entries but still have data to extract");
            }

            let entry = &manifest.entries[current_entry_idx];
            let file_path = output_dir.join(&entry.relative_path);

            if entry.original_size == 0 {
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                File::create(&file_path)?;
                current_entry_idx += 1;
                continue;
            }

            if current_file.is_none() {
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                current_file = Some(File::create(&file_path)?);
                bytes_written_to_current_file = 0;
            }

            let file_remaining = entry.original_size - bytes_written_to_current_file;
            let fragment_remaining = data_len - data_offset;
            
            // Limit ferrying exactly up to file vs fragment boundaries
            let bytes_to_read = file_remaining.min(fragment_remaining).min(chunk_buf.len() as u64) as usize;

            let n = decoder.read(&mut chunk_buf[..bytes_to_read])?;
            if n == 0 {
                anyhow::bail!("Unexpected EOF while streaming decompressor fragment {}", frag_idx);
            }
            
            current_file.as_mut().unwrap().write_all(&chunk_buf[..n])?;

            data_offset += n as u64;
            bytes_written_to_current_file += n as u64;

            if bytes_written_to_current_file == entry.original_size {
                current_file = None;
                current_entry_idx += 1;
            }
        }
        
        // ZSTD decoder should hit EOF clean
        let mut sink = [0u8; 1];
        if decoder.read(&mut sink)? != 0 {
             anyhow::bail!("ZSTD stream produced more exact original_size bytes than registered in Manifest metadata.");
        }

        // Validate corruption signature natively 
        let checksum = hashing_reader.finalize();
        anyhow::ensure!(
            checksum == frag_meta.checksum,
            "Checksum mismatch for fragment {}; archive may be corrupted",
            frag_idx
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::build_manifest;
    use crate::publisher::{start_pipeline, CompressEvent};
    use tempfile::TempDir;

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

        let mut manifest = build_manifest(input_dir.path(), 1024 * 1024).unwrap(); 
        manifest.fragment_size = 16;

        // 2 workers, channel bound of 10
        let pipeline = start_pipeline(input_dir.path().to_path_buf(), manifest, Some(2), 10);

        // Drain each receiver in its own consumer thread, writing fragments to archive_dir
        let mut consumer_handles = Vec::new();
        for rx in pipeline.receivers {
            let archive = archive_dir.path().to_path_buf();
            consumer_handles.push(std::thread::spawn(move || {
                let mut current_file: Option<std::fs::File> = None;
                for event in rx {
                    match event.unwrap() {
                        CompressEvent::Start { fragment_idx } => {
                            let path = archive.join(format!("fragment_{:06}.zst", fragment_idx));
                            current_file = Some(std::fs::File::create(&path).unwrap());
                        }
                        CompressEvent::Chunk { data } => {
                            std::io::Write::write_all(current_file.as_mut().unwrap(), &data).unwrap();
                        }
                        CompressEvent::Complete { .. } => {
                            current_file = None;
                        }
                    }
                }
            }));
        }

        // Wait for consumers
        for h in consumer_handles {
            h.join().unwrap();
        }

        // Get the final manifest from the coordinator
        let manifest = pipeline.handle.join().unwrap().unwrap();
        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(archive_dir.path().join("manifest.json"), manifest_json).unwrap();

        // Now decompress
        let manifest_content = fs::read_to_string(archive_dir.path().join("manifest.json")).unwrap();
        let saved_manifest: Manifest = serde_json::from_str(&manifest_content).unwrap();
        
        extract_archive(archive_dir.path(), output_dir.path(), &saved_manifest).unwrap();

        assert_eq!(fs::read_to_string(output_dir.path().join("f1.txt")).unwrap(), "first file data");
        assert_eq!(fs::read_to_string(output_dir.path().join("f2.txt")).unwrap(), "second somewhat longer file");
        assert_eq!(fs::read_to_string(output_dir.path().join("sub/f3.txt")).unwrap(), "in a subdir");
        assert_eq!(fs::read_to_string(output_dir.path().join("empty.txt")).unwrap(), "");
    }
}
