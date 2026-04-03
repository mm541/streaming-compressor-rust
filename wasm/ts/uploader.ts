// ════════════════════════════════════════════════
// Streaming Upload Engine — Direct-to-S3
// ════════════════════════════════════════════════
//
// The library does NOT call any backend endpoints itself.
// The consumer provides two callbacks:
//   - getPresignedUrls() → how to get S3 URLs (frontend's job)
//   - onUploadComplete() → what to do with metadata (frontend's job)

// ════════════════════════════════════════════════
// Types
// ════════════════════════════════════════════════

export interface PresignedUrl {
    key: string;
    url: string;
}

export interface FileDescriptor {
    name: string;
    size: number;
    mimeType: string;
}

export interface S3FileMetadata {
    s3Key: string;
    originalName: string;
    size: number;
    mimeType: string;
    compressed: boolean;
    originalSize?: number;
}

export interface UploadOptions {
    /**
     * Callback to obtain presigned S3 PUT URLs.
     * The frontend is responsible for calling its own backend.
     */
    getPresignedUrls: (files: FileDescriptor[]) => Promise<PresignedUrl[]>;
    /**
     * Called after all files are uploaded to S3 with the full metadata.
     * The frontend can send this to its backend, save it, etc.
     */
    onUploadComplete?: (info: { totalFiles: number; files: S3FileMetadata[] }) => Promise<void> | void;
    /** Size threshold in bytes above which compression is attempted (default: 1MB) */
    compressThreshold?: number;
    /** Max concurrent S3 uploads (default: 6) */
    concurrency?: number;
    /** Progress callback */
    onProgress?: (info: UploadProgressInfo) => void;
    /** Per-file complete callback */
    onFileComplete?: (info: { file: string; compressed: boolean; s3Key: string }) => void;
    /** Error callback */
    onError?: (error: Error) => void;
}

export interface UploadProgressInfo {
    phase: 'compressing' | 'uploading' | 'complete';
    file: string;
    uploaded: number;
    total: number;
    percent: number;
}

// ════════════════════════════════════════════════
// Extension-Based Compression Skip
// ════════════════════════════════════════════════

const SKIP_EXTENSIONS = new Set([
    'mp4', 'webm', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'm4v',
    'mp3', 'aac', 'ogg', 'flac', 'wma', 'm4a', 'opus',
    'jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'heic',
    'zip', 'gz', 'bz2', 'xz', 'zst', 'rar', '7z', 'tar.gz', 'tgz',
    'pdf', 'docx', 'xlsx', 'pptx',
]);

function shouldSkipCompression(filename: string): boolean {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return SKIP_EXTENSIONS.has(ext);
}

// ════════════════════════════════════════════════
// MIME Type Detection
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
// Worker Pool (for WASM Compression)
// ════════════════════════════════════════════════

let workerPool: Worker[] | null = null;

export function initWorkerPool(count?: number) {
    const numWorkers = count || navigator.hardwareConcurrency || 4;
    if (workerPool && workerPool.length === numWorkers) return;
    if (workerPool) workerPool.forEach((w: Worker) => w.terminate());

    workerPool = [];
    for (let i = 0; i < numWorkers; i++) {
        workerPool.push(new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' }));
    }
    console.log(`[Compressor] Warm pool: ${numWorkers} workers`);
}

export function destroyWorkerPool() {
    if (workerPool) {
        workerPool.forEach((w: Worker) => w.terminate());
        workerPool = null;
    }
}

// ════════════════════════════════════════════════
// WASM Compression via Worker
// ════════════════════════════════════════════════

let nextMsgId = 0;

function compressInWorker(file: File): Promise<{ data: Uint8Array; compressed: boolean; originalSize: number }> {
    return new Promise((resolve, reject) => {
        const workers = workerPool || (() => {
            const pool = [new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })];
            return pool;
        })();

        const id = nextMsgId++;
        const worker = workers[id % workers.length];

        const handler = (e: MessageEvent) => {
            const msg = e.data;
            if (msg.id !== id) return;

            worker.removeEventListener('message', handler);

            if (msg.type === 'compressed') {
                resolve({
                    data: msg.data,
                    compressed: !msg.skipped,
                    originalSize: msg.originalSize,
                });
            } else if (msg.type === 'error') {
                reject(new Error(msg.error));
            }
        };

        worker.addEventListener('message', handler);
        worker.postMessage({ id, file, start: 0, end: file.size });
    });
}

// ════════════════════════════════════════════════
// S3 Upload with Retry
// ════════════════════════════════════════════════

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 500;

async function uploadToS3(url: string, body: Blob | Uint8Array, mimeType: string): Promise<void> {
    for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
        try {
            const response = await fetch(url, {
                method: 'PUT',
                headers: { 'Content-Type': mimeType },
                body: body as any,
            });
            if (response.ok || response.status === 200) return;
            if (response.status < 500) throw new Error(`S3 upload failed: ${response.status}`);
        } catch (error) {
            if (attempt === MAX_RETRIES) throw error;
        }
        const delay = BASE_DELAY_MS * Math.pow(2, attempt);
        await new Promise(resolve => setTimeout(resolve, delay));
    }
}

// ════════════════════════════════════════════════
// Bounded Parallel Execution
// ════════════════════════════════════════════════

async function parallelExec<T>(
    tasks: (() => Promise<T>)[],
    concurrency: number,
): Promise<T[]> {
    const results: T[] = new Array(tasks.length);
    let index = 0;

    async function next(): Promise<void> {
        while (index < tasks.length) {
            const current = index++;
            results[current] = await tasks[current]();
        }
    }

    await Promise.all(
        Array.from({ length: Math.min(concurrency, tasks.length) }, () => next()),
    );

    return results;
}

// ════════════════════════════════════════════════
// Single File Upload
// ════════════════════════════════════════════════

/**
 * Upload a single file directly to S3.
 * Compresses via WASM if beneficial. Consumer provides getPresignedUrls callback.
 */
export async function uploadFile(file: File, options: UploadOptions): Promise<S3FileMetadata> {
    const {
        getPresignedUrls,
        onUploadComplete,
        compressThreshold = 1024 * 1024,
        onProgress,
        onFileComplete,
        onError,
    } = options;

    try {
        const mimeType = detectMimeType(file.name);
        const shouldCompress = !shouldSkipCompression(file.name) && file.size > compressThreshold;

        // 1. Get presigned URL via consumer callback
        const presignedUrls = await getPresignedUrls([{ name: file.name, size: file.size, mimeType }]);
        const presigned = presignedUrls[0];

        // 2. Compress if needed
        let uploadBody: Blob | Uint8Array = file;
        let compressed = false;
        let originalSize = file.size;

        if (shouldCompress) {
            onProgress?.({ phase: 'compressing', file: file.name, uploaded: 0, total: 1, percent: 0 });
            const result = await compressInWorker(file);
            uploadBody = result.data;
            compressed = result.compressed;
            originalSize = result.originalSize;
        }

        // 3. Upload to S3
        onProgress?.({ phase: 'uploading', file: file.name, uploaded: 0, total: 1, percent: 50 });
        await uploadToS3(presigned.url, uploadBody, mimeType);

        const meta: S3FileMetadata = {
            s3Key: presigned.key,
            originalName: file.name,
            size: uploadBody instanceof Uint8Array ? uploadBody.length : (uploadBody as Blob).size,
            mimeType,
            compressed,
            originalSize,
        };

        onProgress?.({ phase: 'complete', file: file.name, uploaded: 1, total: 1, percent: 100 });
        onFileComplete?.({ file: file.name, compressed, s3Key: presigned.key });
        await onUploadComplete?.({ totalFiles: 1, files: [meta] });

        return meta;
    } catch (e: any) {
        const error = e instanceof Error ? e : new Error(String(e));
        onError?.(error);
        throw error;
    }
}

// ════════════════════════════════════════════════
// Directory Upload
// ════════════════════════════════════════════════

/**
 * Upload an entire directory directly to S3.
 * Consumer provides getPresignedUrls callback.
 */
export async function uploadDirectory(fileList: FileList, options: UploadOptions): Promise<{ totalFiles: number; files: S3FileMetadata[] }> {
    const {
        getPresignedUrls,
        onUploadComplete,
        compressThreshold = 1024 * 1024,
        concurrency = 6,
        onProgress,
        onFileComplete,
        onError,
    } = options;

    const files = Array.from(fileList).filter(f => !f.name.startsWith('.') && f.size > 0);
    console.log(`[Compressor] Uploading directory: ${files.length} files directly to S3`);

    try {
        // 1. Request presigned URLs for ALL files via consumer callback
        const descriptors: FileDescriptor[] = files.map(f => ({
            name: f.webkitRelativePath || f.name,
            size: f.size,
            mimeType: detectMimeType(f.name),
        }));

        const presignedUrls = await getPresignedUrls(descriptors);

        // 2. Compress + upload each file in parallel (bounded)
        let uploaded = 0;
        const total = files.length;

        const tasks = files.map((file, i) => async () => {
            const mimeType = detectMimeType(file.name);
            const shouldCompress = !shouldSkipCompression(file.name) && file.size > compressThreshold;

            let uploadBody: Blob | Uint8Array = file;
            let compressed = false;
            let originalSize = file.size;

            if (shouldCompress) {
                const result = await compressInWorker(file);
                uploadBody = result.data;
                compressed = result.compressed;
                originalSize = result.originalSize;
            }

            await uploadToS3(presignedUrls[i].url, uploadBody, mimeType);

            uploaded++;
            onProgress?.({
                phase: 'uploading',
                file: file.webkitRelativePath || file.name,
                uploaded,
                total,
                percent: Math.round((uploaded / total) * 100),
            });
            onFileComplete?.({
                file: file.webkitRelativePath || file.name,
                compressed,
                s3Key: presignedUrls[i].key,
            });

            return {
                s3Key: presignedUrls[i].key,
                originalName: file.webkitRelativePath || file.name,
                size: uploadBody instanceof Uint8Array ? uploadBody.length : (uploadBody as Blob).size,
                mimeType,
                compressed,
                originalSize,
            } as S3FileMetadata;
        });

        const allMeta = await parallelExec(tasks, concurrency);

        // 3. Notify consumer with metadata
        await onUploadComplete?.({ totalFiles: allMeta.length, files: allMeta });

        return { totalFiles: allMeta.length, files: allMeta };
    } catch (e: any) {
        const error = e instanceof Error ? e : new Error(String(e));
        onError?.(error);
        throw error;
    }
}
