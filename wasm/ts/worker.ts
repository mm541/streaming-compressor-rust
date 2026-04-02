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
// Content-Type Detection
// ════════════════════════════════════════════════

const MIME_TYPES: Record<string, string> = {
    mp4: 'video/mp4', webm: 'video/webm', mkv: 'video/x-matroska',
    avi: 'video/x-msvideo', mov: 'video/quicktime',
    mp3: 'audio/mpeg', wav: 'audio/wav', flac: 'audio/flac',
    ogg: 'audio/ogg',
    jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png',
    gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
    pdf: 'application/pdf', zip: 'application/zip',
    gz: 'application/gzip', tar: 'application/x-tar',
    txt: 'text/plain', json: 'application/json',
    csv: 'text/csv', xml: 'application/xml',
};

function detectMimeType(filename: string): string {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return MIME_TYPES[ext] || 'application/octet-stream';
}

// ════════════════════════════════════════════════
// Retry Logic
// ════════════════════════════════════════════════

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 500;

async function uploadWithRetry(url: string, body: Uint8Array, headers: Record<string, string>, retries = MAX_RETRIES) {
    for (let attempt = 0; attempt <= retries; attempt++) {
        try {
            const response = await fetch(url, { method: 'POST', headers, body: body as any });
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

self.onmessage = async (e: MessageEvent) => {
    const { id, file, start, end, index, uploadUrl, relativePath } = e.data;

    const skipCompression = shouldSkipCompression(file.name);

    try {
        await wasmReady;

        // 1. Read file slice
        const chunkBuffer = await file.slice(start, end).arrayBuffer();
        const ui8Data = new Uint8Array(chunkBuffer);
        const originalSize = ui8Data.length;

        // 2. Compress or passthrough
        let outputData;
        if (skipCompression) {
            const result = passthrough_chunk(ui8Data);
            outputData = result.data;
        } else {
            const result = compress_streaming_chunk(ui8Data, false);
            outputData = result.data;
        }

        const compressedSize = outputData.length;

        // 3. Report compression progress
        self.postMessage({
            id, type: 'compressed',
            originalSize, compressedSize,
            skipped: skipCompression
        });

        // 4. Build headers
        const mimeType = detectMimeType(file.name);
        const pathToSend = relativePath || file.name;
        const headers = {
            'Content-Type': 'application/octet-stream',
            'X-Original-Content-Type': mimeType,
            'X-Compression-Skipped': skipCompression ? 'true' : 'false',
            'X-Relative-Path': encodeURIComponent(pathToSend),
        };

        // 5. Upload with retry
        const url = uploadUrl ||
            `/api/upload-chunk?file=${encodeURIComponent(pathToSend)}&index=${index}`;
        await uploadWithRetry(url, outputData, headers);

        // 6. Report upload success
        self.postMessage({ id, type: 'uploaded', success: true });

    } catch (error) {
        self.postMessage({ id, type: 'error', success: false, error: (error as Error).message });
    }
};
