use wasm_bindgen::prelude::*;
use core::compressor::compress_chunk;

/// Result of compressing a single chunk, returned to JavaScript.
#[wasm_bindgen]
pub struct ChunkResult {
    compressed: Vec<u8>,
    skipped: bool,
}

#[wasm_bindgen]
impl ChunkResult {
    /// Get the output bytes as a Uint8Array
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.compressed.clone()
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
#[wasm_bindgen]
pub fn compress_streaming_chunk(data: &[u8], detect_skip: bool) -> Result<ChunkResult, JsValue> {
    let compressed = compress_chunk(data, 3)
        .map_err(|e| JsValue::from_str(&format!("Compression error: {}", e)))?;

    let ratio = compressed.len() as f64 / data.len() as f64;
    let skipped = detect_skip && data.len() > 1024 && ratio > SKIP_RATIO_THRESHOLD;

    if skipped {
        Ok(ChunkResult {
            compressed: data.to_vec(),
            skipped: true,
        })
    } else {
        Ok(ChunkResult {
            compressed,
            skipped: false,
        })
    }
}

/// Raw passthrough: no compression at all.
#[wasm_bindgen]
pub fn passthrough_chunk(data: &[u8]) -> Result<ChunkResult, JsValue> {
    Ok(ChunkResult {
        compressed: data.to_vec(),
        skipped: true,
    })
}
