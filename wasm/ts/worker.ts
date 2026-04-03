// ════════════════════════════════════════════════
// WASM Compression Worker — Compress Only
// ════════════════════════════════════════════════
//
// This worker ONLY handles compression. It receives raw
// file data, compresses it via WASM LZ4, and returns the
// compressed bytes back to the main thread. No upload logic.

// @ts-ignore: wasm-pack doesn't always provide perfectly resolved types for raw JS imports
import init, { compress_streaming_chunk, passthrough_chunk } from '../pkg/wasm.js';

let wasmReady = init();

// ════════════════════════════════════════════════
// Extension-Based Skip Detection
// ════════════════════════════════════════════════

const SKIP_EXTENSIONS = new Set([
    // Video
    'mp4', 'webm', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'm4v',
    // Audio
    'mp3', 'aac', 'ogg', 'flac', 'wma', 'm4a', 'opus',
    // Images (lossy)
    'jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'heic',
    // Archives (already compressed)
    'zip', 'gz', 'bz2', 'xz', 'zst', 'rar', '7z', 'tar.gz', 'tgz',
    // Documents (internally compressed)
    'pdf', 'docx', 'xlsx', 'pptx',
]);

function shouldSkipCompression(filename: string): boolean {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return SKIP_EXTENSIONS.has(ext);
}

// ════════════════════════════════════════════════
// Main Message Handler
// ════════════════════════════════════════════════

self.onmessage = async (e: MessageEvent) => {
    const { id, file, start, end } = e.data;

    const skipCompression = shouldSkipCompression(file.name);

    try {
        await wasmReady;

        // 1. Read file slice
        const chunkBuffer = await file.slice(start, end).arrayBuffer();
        const ui8Data = new Uint8Array(chunkBuffer);
        const originalSize = ui8Data.length;

        // 2. Compress or passthrough
        let outputData: Uint8Array;
        if (skipCompression) {
            const result = passthrough_chunk(ui8Data);
            outputData = result.data;
        } else {
            const result = compress_streaming_chunk(ui8Data, false);
            outputData = result.data;
        }

        const compressedSize = outputData.length;

        // 3. Return compressed data to main thread
        self.postMessage({
            id,
            type: 'compressed',
            data: outputData,
            originalSize,
            compressedSize,
            skipped: skipCompression,
        }, [outputData.buffer] as any); // Transfer ownership for zero-copy

    } catch (error) {
        self.postMessage({ id, type: 'error', error: (error as Error).message });
    }
};
