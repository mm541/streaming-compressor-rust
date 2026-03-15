use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::manifest::Manifest;

/// `FragmentReader` acts as a seamless byte-stream reader over a portion of the fully concatenated archive.
/// It is responsible for exactly `fragment_size` bytes (or less if at the end of the archive).
///
/// Uses `Arc<Manifest>` internally so that all readers across all threads share
/// a **single** copy of the manifest in memory — zero cloning overhead.
pub struct FragmentReader {
    root: PathBuf,
    manifest: Arc<Manifest>,
    current_entry_idx: usize,
    current_file: Option<File>,
    bytes_remaining_in_fragment: u64,
    bytes_remaining_in_current_file: u64,
}

impl FragmentReader {
    pub fn new(root: &Path, manifest: &Arc<Manifest>, fragment_idx: usize) -> Result<Self> {
        let virtual_start = (fragment_idx as u64) * manifest.fragment_size;
        let virtual_end = std::cmp::min(virtual_start + manifest.fragment_size, manifest.total_original_size);
        let bytes_remaining_in_fragment = virtual_end.saturating_sub(virtual_start);
        
        let mut current_entry_idx = 0;
        let mut current_file = None;
        let mut bytes_remaining_in_current_file = 0;

        if bytes_remaining_in_fragment > 0 {
            // Find the entry that covers virtual_start
            for (i, entry) in manifest.entries.iter().enumerate() {
                if virtual_start >= entry.byte_offset && virtual_start < entry.byte_offset + entry.original_size {
                    current_entry_idx = i;
                    let file_path = root.join(&entry.relative_path);
                    let file_offset = virtual_start - entry.byte_offset;
                    bytes_remaining_in_current_file = entry.original_size - file_offset;
                    
                    match File::open(&file_path) {
                        Ok(mut f) => {
                            if let Err(e) = f.seek(SeekFrom::Start(file_offset)) {
                                eprintln!("Warning: failed to seek in {} during compression: {}", file_path.display(), e);
                            } else {
                                current_file = Some(f);
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: failed to open {} during compression (file deleted/locked?): {}", file_path.display(), e);
                            // We leave current_file = None, which triggers zero-padding for its length
                        }
                    }
                    break;
                }
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            manifest: Arc::clone(manifest),
            current_entry_idx,
            current_file,
            bytes_remaining_in_fragment,
            bytes_remaining_in_current_file,
        })
    }
}

impl Read for FragmentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.bytes_remaining_in_fragment == 0 || buf.is_empty() {
            return Ok(0);
        }

        while self.current_entry_idx < self.manifest.entries.len() {
            if self.bytes_remaining_in_current_file == 0 {
                let entry = &self.manifest.entries[self.current_entry_idx];
                self.bytes_remaining_in_current_file = entry.original_size;
                
                // Skip empty files and symlinks
                if entry.original_size == 0 {
                    self.current_entry_idx += 1;
                    continue;
                }

                let file_path = self.root.join(&entry.relative_path);
                match File::open(&file_path) {
                    Ok(f) => self.current_file = Some(f),
                    Err(e) => {
                        eprintln!("Warning: could not open {} during compression: {}", file_path.display(), e);
                        self.current_file = None; // fallback to zero padding
                    }
                }
            }

            let limit = std::cmp::min(
                buf.len() as u64,
                std::cmp::min(self.bytes_remaining_in_fragment, self.bytes_remaining_in_current_file)
            ) as usize;

            let n = match self.current_file.as_mut() {
                Some(file) => {
                    match file.read(&mut buf[..limit]) {
                        Ok(0) => {
                            // File truncated or grew unexpectedly.
                            // We MUST pad with zeros to fulfill the manifest contract!
                            buf[..limit].fill(0);
                            limit
                        }
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("Warning: read error during compression: {}", e);
                            self.current_file = None; // fallback to zero padding for remaining bytes
                            buf[..limit].fill(0);
                            limit
                        }
                    }
                }
                None => {
                    // File missing or unreadable, pad with zeros
                    buf[..limit].fill(0);
                    limit
                }
            };

            self.bytes_remaining_in_fragment -= n as u64;
            self.bytes_remaining_in_current_file -= n as u64;
            
            if self.bytes_remaining_in_current_file == 0 {
                self.current_file = None;
                self.current_entry_idx += 1;
            }
            
            return Ok(n);
        }

        Ok(0)
    }
}
