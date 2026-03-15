use std::io::{Read, Write};
use anyhow::Result;
use zstd::stream::{Encoder, Decoder};

/// Size of internal copy buffer. 128 KB is 16× the std::io::copy default,
/// drastically cutting syscall overhead for many-fragment workloads.
const COPY_BUF_SIZE: usize = 128 * 1024;

/// Compress a chunk of data using zstd's streaming encoder.
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
}
