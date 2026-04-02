/**
 * Production-grade streaming upload engine.
 * Supports: single files, directories, auto compression detection,
 * Web Worker parallelism, backpressure, resumable uploads, progress,
 * retry logic, and blake3 integrity checksums.
 */

// ════════════════════════════════════════════════
// Persistent Worker Warm Pool
// ════════════════════════════════════════════════

let workerPool: Worker[] | null = null;

/**
 * Initialize a persistent pool of Web Workers.
 * Call once on app startup. Workers stay alive across uploads.
 */
export function initWorkerPool(count?: number) {
    const numWorkers = count || navigator.hardwareConcurrency || 4;
    if (workerPool && workerPool.length === numWorkers) return;
    if (workerPool) workerPool.forEach((w: Worker) => w.terminate());

    workerPool = [];
    for (let i = 0; i < numWorkers; i++) {
        workerPool.push(new Worker(new URL('./worker.js', import.meta.url), { type: 'module' }));
    }
    console.log(`[Compressor] Warm pool: ${numWorkers} workers`);
}

/** Destroy the worker pool on app teardown. */
export function destroyWorkerPool() {
    if (workerPool) {
        workerPool.forEach((w: Worker) => w.terminate());
        workerPool = null;
    }
}

// ════════════════════════════════════════════════
// Resumable State (localStorage)
// ════════════════════════════════════════════════

function getUploadKey(file: File) {
    return `compressor_${file.name}_${file.size}_${file.lastModified}`;
}

function getCompletedChunks(file: File) {
    try {
        const stored = localStorage.getItem(getUploadKey(file));
        return stored ? new Set(JSON.parse(stored)) : new Set();
    } catch { return new Set(); }
}

function markChunkCompleted(file: File, idx: number) {
    try {
        const completed = getCompletedChunks(file);
        completed.add(idx);
        localStorage.setItem(getUploadKey(file), JSON.stringify([...completed]));
    } catch {}
}

function clearUploadState(file: File) {
    try { localStorage.removeItem(getUploadKey(file)); } catch {}
}

// ════════════════════════════════════════════════
// Single File Upload (core engine)
// ════════════════════════════════════════════════

/**
 * Upload a single file with chunking, compression auto-detection, and all features.
 * @param {File} file
 * @param {Object} options
 * @param {string} [options.relativePath] - Path within directory (for directory uploads)
 * @param {number} [options.chunkSize=1048576]
 * @param {number} [options.maxConcurrentUploads=3]
 * @param {boolean} [options.resumable=true]
 * @param {string} [options.uploadUrl]
 * @param {function} [options.onProgress]
 * @param {function} [options.onFileComplete]
 * @param {function} [options.onError]
 */
export async function uploadFile(file: File, options: any = {}): Promise<any> {
    const {
        relativePath = null,
        chunkSize = 1024 * 1024,
        maxConcurrentUploads = 3,
        resumable = true,
        uploadUrl = null,
        onProgress = () => {},
        onFileComplete = () => {},
        onError = () => {},
    } = options;

    const totalChunks = Math.ceil(file.size / chunkSize);
    const ownWorkers = !workerPool;
    const workers = workerPool || (() => {
        const n = Math.min(navigator.hardwareConcurrency || 4, totalChunks);
        const pool = [];
        for (let i = 0; i < n; i++) {
            pool.push(new Worker(new URL('./worker.js', import.meta.url), { type: 'module' }));
        }
        return pool;
    })();

    const completedChunks = resumable ? getCompletedChunks(file) : new Set();
    let nextChunk = 0;
    let uploadedCount = completedChunks.size;
    let activeWorkers = 0;

    return new Promise((resolve, reject) => {
        function tryDispatch() {
            if (uploadedCount === totalChunks) {
                if (ownWorkers) workers.forEach((w: Worker) => w.terminate());
                if (resumable) clearUploadState(file);
                onFileComplete({ file: relativePath || file.name, totalChunks });
                return resolve({ file: relativePath || file.name, totalChunks });
            }

            while (nextChunk < totalChunks && activeWorkers < workers.length) {
                const idx = nextChunk++;
                if (completedChunks.has(idx)) continue;

                activeWorkers++;
                const worker = workers[idx % workers.length];
                const start = idx * chunkSize;
                const end = Math.min(start + chunkSize, file.size);

                const handler = (e: MessageEvent) => {
                    const msg = e.data;
                    if (msg.id !== idx) return;

                    if (msg.type === 'compressed') {
                        onProgress({
                            phase: 'compressed',
                            file: relativePath || file.name,
                            chunkIndex: idx, totalChunks,
                            originalSize: msg.originalSize,
                            compressedSize: msg.compressedSize,
                            checksum: msg.checksum,
                            skipped: msg.skipped,
                        });
                    }

                    if (msg.type === 'uploaded') {
                        worker.removeEventListener('message', handler);
                        activeWorkers--;
                        uploadedCount++;
                        if (resumable) markChunkCompleted(file, idx);

                        onProgress({
                            phase: 'uploaded',
                            file: relativePath || file.name,
                            chunkIndex: idx, totalChunks,
                            percent: Math.round((uploadedCount / totalChunks) * 100),
                        });
                        tryDispatch();
                    }

                    if (msg.type === 'error') {
                        worker.removeEventListener('message', handler);
                        activeWorkers--;
                        if (ownWorkers) workers.forEach((w: Worker) => w.terminate());
                        const err = new Error(`${relativePath || file.name} chunk ${idx}: ${msg.error}`);
                        onError(err);
                        reject(err);
                    }
                };

                worker.addEventListener('message', handler);
                worker.postMessage({
                    id: idx, file, start, end,
                    index: idx,
                    uploadUrl,
                    relativePath: relativePath || file.name,
                });
            }
        }
        tryDispatch();
    });
}

// ════════════════════════════════════════════════
// Directory Upload
// ════════════════════════════════════════════════

/**
 * Upload an entire directory. Files are compressed or uploaded raw
 * automatically based on their extension.
 *
 * Usage:
 *   <input type="file" webkitdirectory id="dirInput">
 *   const files = document.getElementById('dirInput').files;
 *   await uploadDirectory(files, { ... });
 *
 * @param {FileList} fileList - From an <input webkitdirectory> element
 * @param {Object} options
 * @param {number} [options.chunkSize=1048576]
 * @param {number} [options.maxConcurrentUploads=3]
 * @param {boolean} [options.resumable=true]
 * @param {string} [options.uploadUrl]
 * @param {function} [options.onProgress] - Per-chunk: { file, chunkIndex, totalChunks, percent }
 * @param {function} [options.onFileComplete] - Per-file: { file, totalChunks }
 * @param {function} [options.onDirectoryComplete] - All files done: { totalFiles, manifest }
 * @param {function} [options.onError]
 */
export async function uploadDirectory(fileList: FileList, options: any = {}): Promise<any> {
    const {
        chunkSize = 1024 * 1024,
        maxConcurrentUploads = 3,
        resumable = true,
        uploadUrl = null,
        onProgress = () => {},
        onFileComplete = () => {},
        onDirectoryComplete = () => {},
        onError = () => {},
    } = options;

    const files = Array.from(fileList);
    const manifest = [];

    console.log(`[Compressor] Uploading directory: ${files.length} files`);

    // Process files sequentially (each file uses parallel chunk workers internally)
    for (const file of files) {
        // Skip hidden files and empty files
        if (file.name.startsWith('.') || file.size === 0) continue;

        const relativePath = file.webkitRelativePath || file.name;

        try {
            const result = await uploadFile(file, {
                relativePath,
                chunkSize,
                maxConcurrentUploads,
                resumable,
                uploadUrl,
                onProgress,
                onFileComplete,
                onError,
            });

            manifest.push({
                relativePath,
                totalChunks: result.totalChunks,
                size: file.size,
            });

        } catch (err) {
            onError(err);
            throw err;
        }
    }

    // Send finalize with the full directory manifest
    const finalizeUrl = uploadUrl
        ? `${uploadUrl.replace('/upload-chunk', '/finalize')}`
        : '/api/finalize';

    await fetch(finalizeUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            type: 'directory',
            totalFiles: manifest.length,
            manifest,
        })
    });

    onDirectoryComplete({ totalFiles: manifest.length, manifest });
    return { totalFiles: manifest.length, manifest };
}

// ════════════════════════════════════════════════
// Usage Examples
// ════════════════════════════════════════════════
//
// // 1. Initialize warm pool once
// initWorkerPool();
//
// // 2a. Single file upload
// const fileInput = document.getElementById('fileInput');
// fileInput.addEventListener('change', async (e) => {
//     await uploadFile(e.target.files[0], {
//         onProgress: (info) => console.log(`${info.percent}%`),
//         onFileComplete: () => console.log('Done!'),
//     });
// });
//
// // 2b. Directory upload
// // <input type="file" webkitdirectory id="dirInput">
// const dirInput = document.getElementById('dirInput');
// dirInput.addEventListener('change', async (e) => {
//     await uploadDirectory(e.target.files, {
//         onProgress: (info) => console.log(`${info.file}: ${info.percent}%`),
//         onFileComplete: (info) => console.log(`${info.file} complete`),
//         onDirectoryComplete: (info) => console.log(`All ${info.totalFiles} files uploaded!`),
//     });
// });
//
// // 3. Cleanup
// destroyWorkerPool();
