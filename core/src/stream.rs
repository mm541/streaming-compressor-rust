use std::io::{Read, Seek};
use anyhow::Result;

/// A trait that combines Read and Seek into a single interface.
pub trait ReadSeek: Read + Seek {}

/// Blanket implementation for anything that implements both Read and Seek.
impl<T: Read + Seek> ReadSeek for T {}

/// A generic provider that turns string identifiers into byte streams.
/// This trait decouples the core compression logic from the OS filesystem.
pub trait StreamProvider<R: ReadSeek> {
    /// Return a readable, seekable stream for the given identifier.
    fn provide_stream(&self, identifier: &str) -> Result<R>;
}

use std::sync::Arc;
use crate::manifest::Manifest;

/// `FragmentReader` acts as a seamless byte-stream reader over a portion of the fully concatenated archive.
/// It uses a generic `StreamProvider` to load entries dynamically.
pub struct FragmentReader<R: ReadSeek, P: StreamProvider<R>> {
    provider: P,
    manifest: Arc<Manifest>,
    current_entry_idx: usize,
    current_stream: Option<R>,
    bytes_remaining_in_fragment: u64,
    bytes_remaining_in_current_file: u64,
}

impl<R: ReadSeek, P: StreamProvider<R>> FragmentReader<R, P> {
    pub fn new(provider: P, manifest: &Arc<Manifest>, fragment_idx: usize) -> Result<Self> {
        let virtual_start = (fragment_idx as u64) * manifest.fragment_size;
        let virtual_end = std::cmp::min(virtual_start + manifest.fragment_size, manifest.total_original_size);
        let bytes_remaining_in_fragment = virtual_end.saturating_sub(virtual_start);
        
        let mut current_entry_idx = 0;
        let mut current_stream = None;
        let mut bytes_remaining_in_current_file = 0;

        if bytes_remaining_in_fragment > 0 && fragment_idx < manifest.fragment_start_indices.len() {
            let i = manifest.fragment_start_indices[fragment_idx];
            current_entry_idx = i;
            let entry = &manifest.entries[i];
            let file_offset = virtual_start - entry.byte_offset;
            bytes_remaining_in_current_file = entry.original_size - file_offset;

            match provider.provide_stream(&entry.identifier) {
                Ok(mut s) => {
                    if let Err(e) = s.seek(std::io::SeekFrom::Start(file_offset)) {
                        eprintln!("Warning: failed to seek in {} during compression: {}", entry.identifier, e);
                    } else {
                        current_stream = Some(s);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to open {} during compression: {}", entry.identifier, e);
                }
            }
        }

        Ok(Self {
            provider,
            manifest: Arc::clone(manifest),
            current_entry_idx,
            current_stream,
            bytes_remaining_in_fragment,
            bytes_remaining_in_current_file,
        })
    }
}

impl<R: ReadSeek, P: StreamProvider<R>> Read for FragmentReader<R, P> {
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

                match self.provider.provide_stream(&entry.identifier) {
                    Ok(s) => self.current_stream = Some(s),
                    Err(e) => {
                        eprintln!("Warning: could not open {} during compression: {}", entry.identifier, e);
                        self.current_stream = None; // fallback to zero padding
                    }
                }
            }

            let limit = std::cmp::min(
                buf.len() as u64,
                std::cmp::min(self.bytes_remaining_in_fragment, self.bytes_remaining_in_current_file)
            ) as usize;

            let n = match self.current_stream.as_mut() {
                Some(stream) => {
                    match stream.read(&mut buf[..limit]) {
                        Ok(0) => {
                            // Stream truncated unexpectedly. Pad with zero.
                            buf[..limit].fill(0);
                            limit
                        }
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("Warning: read error during compression: {}", e);
                            self.current_stream = None;
                            buf[..limit].fill(0);
                            limit
                        }
                    }
                }
                None => {
                    buf[..limit].fill(0);
                    limit
                }
            };

            self.bytes_remaining_in_fragment -= n as u64;
            self.bytes_remaining_in_current_file -= n as u64;
            
            if self.bytes_remaining_in_current_file == 0 {
                self.current_stream = None;
                self.current_entry_idx += 1;
            }
            
            return Ok(n);
        }

        Ok(0)
    }
}
