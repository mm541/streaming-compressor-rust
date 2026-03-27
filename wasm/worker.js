import init, { compress_streaming_chunk, passthrough_chunk } from './pkg/wasm.js';

let wasmReady = init(); 

// ════════════════════════════════════════════════
// Extension-Based Skip Detection
// ════════════════════════════════════════════════

// Files with these extensions are already compressed by their codecs.
// Attempting LZ4 compression wastes CPU for ~0% size reduction.
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

function shouldSkipCompression(filename) {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return SKIP_EXTENSIONS.has(ext);
}

// ════════════════════════════════════════════════
// Content-Type Detection
// ════════════════════════════════════════════════

const MIME_TYPES = {
    mp4: 'video/mp4', webm: 'video/webm', mkv: 'video/x-matroska',
    avi: 'video/x-msvideo', mov: 'video/quicktime',
    mp3: 'audio/mpeg', wav: 'audio/wav', flac: 'audio/flac',
    jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png',
    gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
    pdf: 'application/pdf', zip: 'application/zip',
    gz: 'application/gzip', tar: 'application/x-tar',
    txt: 'text/plain', json: 'application/json',
    csv: 'text/csv', xml: 'application/xml',
};

function detectMimeType(filename) {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return MIME_TYPES[ext] || 'application/octet-stream';
}

// ════════════════════════════════════════════════
// Retry Logic
// ════════════════════════════════════════════════

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 500;

async function uploadWithRetry(url, body, headers, retries = MAX_RETRIES) {
    for (let attempt = 0; attempt <= retries; attempt++) {
        try {
            const response = await fetch(url, { method: 'POST', headers, body });
            if (response.ok) return response;
            if (response.status < 500) throw new Error(`Upload failed: ${response.status}`);
        } catch (error) {
            if (attempt === retries) throw error;
        }
        const delay = BASE_DELAY_MS * Math.pow(2, attempt);
        await new Promise(resolve => setTimeout(resolve, delay));
    }
}

// ════════════════════════════════════════════════
// Main Message Handler
// ════════════════════════════════════════════════

self.onmessage = async (e) => {
    const { id, file, start, end, index, uploadUrl, relativePath } = e.data;

    // Auto-detect: should we compress this file?
    const skipCompression = shouldSkipCompression(file.name);

    try {
        await wasmReady;

        // 1. Read file slice on this background thread
        const chunkBuffer = await file.slice(start, end).arrayBuffer();
        const ui8Data = new Uint8Array(chunkBuffer);
        const originalSize = ui8Data.length;

        // 2. Compress or passthrough based on file type
        let outputData, checksum;
        if (skipCompression) {
            // Pre-compressed file: just checksum, no LZ4
            const result = passthrough_chunk(ui8Data);
            outputData = result.data;
            checksum = result.checksum;
        } else {
            // Compressible file: run through LZ4
            const result = compress_streaming_chunk(ui8Data, false);
            outputData = result.data;
            checksum = result.checksum;
        }

        const compressedSize = outputData.length;

        // 3. Report compression progress
        self.postMessage({
            id, type: 'compressed',
            originalSize, compressedSize, checksum,
            skipped: skipCompression
        });

        // 4. Build headers
        const mimeType = detectMimeType(file.name);
        const pathToSend = relativePath || file.name;
        const headers = {
            'Content-Type': 'application/octet-stream',
            'X-Original-Content-Type': mimeType,
            'X-Chunk-Checksum': checksum,
            'X-Compression-Skipped': skipCompression ? 'true' : 'false',
            'X-Relative-Path': encodeURIComponent(pathToSend),
        };

        // 5. Upload with retry
        const url = uploadUrl ||
            `/api/upload-chunk?file=${encodeURIComponent(pathToSend)}&index=${index}&checksum=${checksum}`;
        await uploadWithRetry(url, outputData, headers);

        // 6. Report upload success
        self.postMessage({ id, type: 'uploaded', success: true });

    } catch (error) {
        self.postMessage({ id, type: 'error', success: false, error: error.message });
    }
};
