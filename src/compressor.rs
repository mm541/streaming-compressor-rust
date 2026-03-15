use std::io::{Read, Write};
use anyhow::Result;
use zstd::stream::{Encoder, Decoder};

/// Compress a chunk of data using zstd's streaming encoder.
pub fn compress_stream<R: Read, W: Write>(mut reader: R, writer: W, level: i32) -> Result<()> {
    let mut encoder = Encoder::new(writer, level)?;
    std::io::copy(&mut reader, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

/// Decompress a chunk of data using zstd's streaming decoder.
pub fn decompress_stream<R: Read, W: Write>(reader: R, mut writer: W) -> Result<()> {
    let mut decoder = Decoder::new(reader)?;
    std::io::copy(&mut decoder, &mut writer)?;
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
