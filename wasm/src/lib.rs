use wasm_bindgen::prelude::*;
use core::compressor::compress_chunk;

/// Result of compressing a single chunk, returned to JavaScript.
/// Contains the compressed bytes, its blake3 integrity checksum,
/// and whether compression was actually applied or skipped.
#[wasm_bindgen]
pub struct ChunkResult {
    compressed: Vec<u8>,
    checksum: String,
    skipped: bool,
}

#[wasm_bindgen]
impl ChunkResult {
    /// Get the output bytes as a Uint8Array
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.compressed.clone()
    }

    /// Get the blake3 hex checksum for integrity verification
    #[wasm_bindgen(getter)]
    pub fn checksum(&self) -> String {
        self.checksum.clone()
    }

    /// Whether compression was skipped (data is already compressed)
    #[wasm_bindgen(getter)]
    pub fn skipped(&self) -> bool {
        self.skipped
    }
}

/// Ratio threshold: if compressed size > 95% of original, the data
/// is already compressed and we skip compression for all future chunks.
const SKIP_RATIO_THRESHOLD: f64 = 0.95;

/// Compress a single chunk. If `detect_skip` is true and the chunk barely
/// shrinks (>95% ratio), the raw data is returned with `skipped = true`.
/// JavaScript should then switch all future chunks to raw passthrough mode.
#[wasm_bindgen]
pub fn compress_streaming_chunk(data: &[u8], detect_skip: bool) -> Result<ChunkResult, JsValue> {
    let compressed = compress_chunk(data, 3)
        .map_err(|e| JsValue::from_str(&format!("Compression error: {}", e)))?;

    // Skip detection: if compressed >= 95% of original, compression is pointless
    let ratio = compressed.len() as f64 / data.len() as f64;
    let skipped = detect_skip && data.len() > 1024 && ratio > SKIP_RATIO_THRESHOLD;

    if skipped {
        // Return raw data instead — saves CPU on all future chunks
        let checksum = blake3::hash(data).to_hex().to_string();
        Ok(ChunkResult {
            compressed: data.to_vec(),
            checksum,
            skipped: true,
        })
    } else {
        let checksum = blake3::hash(&compressed).to_hex().to_string();
        Ok(ChunkResult {
            compressed,
            checksum,
            skipped: false,
        })
    }
}

/// Raw passthrough: just checksum, no compression at all.
/// Used after skip detection triggers on the first chunk.
#[wasm_bindgen]
pub fn passthrough_chunk(data: &[u8]) -> Result<ChunkResult, JsValue> {
    let checksum = blake3::hash(data).to_hex().to_string();
    Ok(ChunkResult {
        compressed: data.to_vec(),
        checksum,
        skipped: true,
    })
}
