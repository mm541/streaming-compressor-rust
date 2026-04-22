use std::io::{Read, Write};
use anyhow::Result;

/// Trait for pluggable compression engines.
///
/// Core ships a `ZstdEngine` implementation, but consumers can provide
/// their own (Lz4, Brotli, etc.) by implementing this trait.
pub trait CompressionEngine: Send + Sync {
    /// Compress all data from `reader` into `writer`.
    fn compress(&self, reader: &mut dyn Read, writer: &mut dyn Write) -> Result<()>;
    /// Decompress all data from `reader` into `writer`.
    fn decompress(&self, reader: &mut dyn Read, writer: &mut dyn Write) -> Result<()>;
    /// Return a reader that decompresses data on the fly from the given source.
    /// This is needed for streaming decompression where the consumer reads
    /// chunks incrementally (e.g., the reassembler with file boundary tracking).
    fn decompressing_reader<'a>(&self, reader: Box<dyn Read + 'a>) -> Result<Box<dyn Read + 'a>>;
}

// ── Zstd engine (native only) ──────────────────────────────────────────────
mod zstd_engine {
    use super::*;
    use zstd::stream::{Encoder, Decoder};

    const COPY_BUF_SIZE: usize = 128 * 1024;

    thread_local! {
        /// Pooled copy buffer — reused across invocations to avoid repeated allocations.
    static COPY_BUF_POOL: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; COPY_BUF_SIZE]);

        /// Pooled Zstd compression context (~128 KB internal state).
        static ZSTD_CCTX: std::cell::RefCell<zstd_safe::CCtx<'static>> =
            std::cell::RefCell::new(zstd_safe::CCtx::create());

        /// Pooled Zstd decompression context (~128 KB internal state).
        static ZSTD_DCTX: std::cell::RefCell<zstd_safe::DCtx<'static>> =
            std::cell::RefCell::new(zstd_safe::DCtx::create());
    }

    /// High-performance Zstd compression engine with thread-local context pooling.
    ///
    /// Zero heap allocations per fragment after the first call on each thread.
    pub struct ZstdEngine {
        pub level: i32,
    }

    impl ZstdEngine {
        pub fn new(level: i32) -> Self {
            Self { level }
        }
    }

    impl CompressionEngine for ZstdEngine {
        fn compress(&self, reader: &mut dyn Read, writer: &mut dyn Write) -> Result<()> {
            let level = self.level;
            ZSTD_CCTX.with(|cctx_cell| {
                let mut cctx = cctx_cell.borrow_mut();
                cctx.set_parameter(zstd_safe::CParameter::CompressionLevel(level))
                    .map_err(|code| anyhow::anyhow!("failed to set zstd compression level: {}", code))?;

                let mut encoder = Encoder::with_context(writer, &mut cctx);

                COPY_BUF_POOL.with(|buf_cell| -> Result<()> {
                    let mut buf = buf_cell.borrow_mut();
                    loop {
                        let n = reader.read(&mut buf)?;
                        if n == 0 { break; }
                        encoder.write_all(&buf[..n])?;
                    }
                    Ok(())
                })?;

                encoder.finish()?;
                Ok(())
            })
        }

        fn decompress(&self, reader: &mut dyn Read, writer: &mut dyn Write) -> Result<()> {
            ZSTD_DCTX.with(|dctx_cell| {
                let mut dctx = dctx_cell.borrow_mut();
                let buf_reader = std::io::BufReader::new(reader);
                let mut decoder = Decoder::with_context(buf_reader, &mut dctx);

                COPY_BUF_POOL.with(|buf_cell| -> Result<()> {
                    let mut buf = buf_cell.borrow_mut();
                    loop {
                        let n = decoder.read(&mut buf)?;
                        if n == 0 { break; }
                        writer.write_all(&buf[..n])?;
                    }
                    Ok(())
                })
            })
        }

        fn decompressing_reader<'a>(&self, reader: Box<dyn Read + 'a>) -> Result<Box<dyn Read + 'a>> {
            let decoder = zstd::Decoder::new(reader)?;
            Ok(Box::new(decoder))
        }
    }
}

pub use zstd_engine::ZstdEngine;

// ── Legacy convenience functions (kept for backward compat) ─────────────────

/// Compress a stream using Zstd (convenience wrapper).
pub fn compress_stream<R: Read, W: Write>(mut reader: R, mut writer: W, level: i32) -> Result<()> {
    let engine = ZstdEngine::new(level);
    engine.compress(&mut reader, &mut writer)
}

/// Decompress a stream using Zstd (convenience wrapper).
pub fn decompress_stream<R: Read, W: Write>(mut reader: R, mut writer: W) -> Result<()> {
    let engine = ZstdEngine::new(3);
    engine.decompress(&mut reader, &mut writer)
}

/// Compress a byte slice (routes to zstd on native, lz4 on WASM).
pub fn compress_chunk(data: &[u8], level: i32) -> Result<Vec<u8>> {
    let mut compressed = Vec::new();
    compress_stream(data, &mut compressed, level)?;
    Ok(compressed)
}

// ── Tests ──────────────────────────────────────────────────────────────────

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

    #[test]
    fn test_engine_trait_roundtrip() {
        let engine = ZstdEngine::new(3);
        let original = b"trait-based compression engine test data for roundtrip verification";
        
        let mut compressed = Vec::new();
        engine.compress(&mut &original[..], &mut compressed).unwrap();
        
        let mut decompressed = Vec::new();
        engine.decompress(&mut &compressed[..], &mut decompressed).unwrap();
        
        assert_eq!(decompressed, original);
    }
}
