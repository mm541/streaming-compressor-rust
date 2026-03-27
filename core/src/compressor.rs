#[cfg(not(target_arch = "wasm32"))]
use std::io::{Read, Write};
use anyhow::Result;

#[cfg(not(target_arch = "wasm32"))]
use zstd::stream::{Encoder, Decoder};

/// Size of internal copy buffer. 128 KB is 16× the std::io::copy default,
/// drastically cutting syscall overhead for many-fragment workloads.
#[cfg(not(target_arch = "wasm32"))]
const COPY_BUF_SIZE: usize = 128 * 1024;

/// Compress a chunk of data using zstd's streaming encoder.
#[cfg(not(target_arch = "wasm32"))]
pub fn compress_stream<R: Read, W: Write>(mut reader: R, writer: W, level: i32) -> Result<()> {
    let mut encoder = Encoder::new(writer, level)?;
    let mut buf = vec![0u8; COPY_BUF_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        encoder.write_all(&buf[..n])?;
    }
    encoder.finish()?;
    Ok(())
}

/// Decompress a chunk of data using zstd's streaming decoder.
#[cfg(not(target_arch = "wasm32"))]
pub fn decompress_stream<R: Read, W: Write>(reader: R, mut writer: W) -> Result<()> {
    let mut decoder = Decoder::new(reader)?;
    let mut buf = vec![0u8; COPY_BUF_SIZE];
    loop {
        let n = decoder.read(&mut buf)?;
        if n == 0 { break; }
        writer.write_all(&buf[..n])?;
    }
    Ok(())
}

/// A unified function that routes to zstd for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn compress_chunk(data: &[u8], level: i32) -> Result<Vec<u8>> {
    let mut compressed = Vec::new();
    let mut encoder = Encoder::new(&mut compressed, level)?;
    std::io::Write::write_all(&mut encoder, data)?;
    encoder.finish()?;
    Ok(compressed)
}

/// A unified function that routes to lz4_flex for WASM targets.
#[cfg(target_arch = "wasm32")]
pub fn compress_chunk(data: &[u8], _level: i32) -> Result<Vec<u8>> {
    // lz4_flex compress_prepend_size includes the uncompressed size
    // to allow for easy decompression with decompress_size_prepended
    Ok(lz4_flex::compress_prepend_size(data))
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_roundtrip_stream() {
        let original_data = b"hello world, this is a test of the streaming zstd compressor module.";
        let mut compressed = Vec::new();
        
        compress_stream(&original_data[..], &mut compressed, 3).unwrap();
        
        assert!(compressed.len() > 0);
        assert_ne!(compressed.as_slice(), original_data);

        let mut decompressed = Vec::new();
        decompress_stream(&compressed[..], &mut decompressed).unwrap();
        
        assert_eq!(decompressed, original_data);
    }

    #[test]
    fn test_compress_chunk_native() {
        let original_data = b"hello world from native";
        let compressed = compress_chunk(original_data, 3).unwrap();
        assert!(compressed.len() > 0);
        assert_ne!(compressed.as_slice(), original_data);
    }
}

// In a real scenario we'd write to the DOM or handle WASM tests differently,
// but for the sake of compiling and verifying, a mock tests block could exist here.
#[cfg(target_arch = "wasm32")]
#[cfg(test)]
mod wasm_tests {
    use super::*;

    #[test]
    fn test_compress_chunk_wasm() {
        let original_data = b"hello world from wasm";
        let compressed = compress_chunk(original_data, 3).unwrap();
        assert!(compressed.len() > 0);
        assert_ne!(compressed.as_slice(), original_data);
        
        let decompressed = lz4_flex::decompress_size_prepended(&compressed).unwrap();
        assert_eq!(decompressed.as_slice(), original_data);
    }
}
