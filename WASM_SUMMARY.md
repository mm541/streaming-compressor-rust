# WASM Streaming Compression Module — Full Summary

## What It Is

A browser-native, multi-core file upload engine built in Rust (compiled to WebAssembly) + JavaScript. It intelligently compresses files that benefit from compression, skips already-compressed formats (videos, images, archives), and streams everything to a backend via parallel HTTP uploads — all while using only ~5MB of RAM regardless of file size.

---

## Architecture

```
User drags file/folder into browser
            │
            ▼
   ┌─────────────────────────┐
   │  example_streaming_     │  JavaScript (Main Thread)
   │  upload.js              │  • Reads File using Blob.slice()
   │                         │  • Manages worker pool
   │  uploadFile()           │  • Resumable state (localStorage)
   │  uploadDirectory()      │  • Backpressure control
   └────────┬────────────────┘
            │ postMessage(File reference + byte range)
            ▼
   ┌─────────────────────────┐
   │  worker.js (×8 threads) │  JavaScript (Web Workers)
   │                         │  • Reads 1MB slice from disk
   │  Auto-detects file type │  • Calls WASM or passthrough
   │  Retry with backoff     │  • Uploads via fetch()
   │  Content-type headers   │  • Reports progress
   └────────┬────────────────┘
            │ FFI call
            ▼
   ┌─────────────────────────┐
   │  wasm/src/lib.rs        │  Rust → WebAssembly
   │                         │
   │  compress_streaming_    │  • LZ4 compression (lz4_flex)
   │    chunk()              │  • Blake3 integrity checksum
   │  passthrough_chunk()    │  • Returns ChunkResult {data, checksum, skipped}
   └─────────────────────────┘
```

---

## File Structure

```
wasm/
├── Cargo.toml                     # Dependencies: core, wasm-bindgen, blake3
├── src/
│   └── lib.rs                     # 80 lines — stateless WASM functions
├── worker.js                      # Web Worker: auto-detect, compress, upload
└── example_streaming_upload.js    # Orchestrator: pool, resume, directory support
```

---

## WASM API (Rust → JavaScript)

| Function | Input | Output | Purpose |
|----------|-------|--------|---------|
| `compress_streaming_chunk(data, detect_skip)` | `Uint8Array`, `bool` | `ChunkResult` | LZ4 compress + blake3 hash |
| `passthrough_chunk(data)` | `Uint8Array` | `ChunkResult` | Blake3 hash only, no compression |

**`ChunkResult`** has three getters: `.data` (bytes), `.checksum` (hex string), `.skipped` (bool).

---

## JavaScript API

| Function | Purpose |
|----------|---------|
| `initWorkerPool(count?)` | Spawn persistent Web Workers (call once on app start) |
| `destroyWorkerPool()` | Terminate workers (call on app teardown) |
| `uploadFile(file, options)` | Upload a single file with auto-detection |
| `uploadDirectory(fileList, options)` | Upload entire directory from `<input webkitdirectory>` |

**Options:**
- `chunkSize` — Fragment size in bytes (default 1MB)
- `maxConcurrentUploads` — Backpressure limit (default 3)
- `resumable` — Enable localStorage crash recovery (default true)
- `onProgress({ phase, file, percent, checksum, skipped })`
- `onFileComplete({ file, totalChunks })`
- `onDirectoryComplete({ totalFiles, manifest })`
- `onSkipDetected()` — Fires when compression is pointless

---

## Smart Auto-Detection

The worker checks the file extension before touching WASM:

| File Type | Action | Why |
|-----------|--------|-----|
| CSV, JSON, TXT, XML, logs | **LZ4 compress** | 60-80% size reduction |
| BMP, TIFF, RAW images | **LZ4 compress** | Significant savings |
| MP4, WebM, MKV, AVI | **Raw passthrough** | Already codec-compressed |
| JPEG, PNG, WebP, GIF | **Raw passthrough** | Already compressed |
| ZIP, GZ, RAR, 7z | **Raw passthrough** | Already compressed |
| PDF, DOCX, XLSX | **Raw passthrough** | Internally compressed |

Both paths still get: chunking, parallel upload, blake3 checksums, retry, resume, and progress.

---

## Production Features

| Feature | How It Works |
|---------|-------------|
| **Multi-core** | N Web Workers (matches CPU cores), each with own WASM instance |
| **Zero UI freeze** | Workers read files + compress + upload entirely off main thread |
| **~5MB RAM** | Only 1 chunk per worker in memory at any time |
| **Resumable** | Completed chunk indices stored in localStorage |
| **Retry** | 3 attempts with exponential backoff (500ms → 1s → 2s) |
| **Backpressure** | Max 3 concurrent HTTP uploads to prevent network flooding |
| **Integrity** | Blake3 checksum per chunk, verified by Python backend |
| **Directory support** | Preserves folder structure via `webkitRelativePath` |
| **Content-type** | `X-Original-Content-Type` header for backend routing |
| **Tiny binary** | Release build uses `opt-level=z` + LTO + strip |

---

## Data Format

Each compressed chunk has the format:
```
[4 bytes: uncompressed size (little-endian u32)][LZ4 block payload]
```

Python decompresses with:
```python
size = struct.unpack('<I', chunk[:4])[0]
data = lz4.block.decompress(chunk[4:], uncompressed_size=size)
```

Passthrough (skipped) chunks are raw bytes with no header.

---

## Backend Integration

```
Browser → Spring Boot (saves chunks to shared volume)
                │
                ▼
          RabbitMQ (metadata messages only, not binary data)
                │
                ▼
          Python (reads chunks from shared volume → decompresses → assembles → processes)
```

Full backend code is documented in [`BACKEND_INTEGRATION.md`](file:///home/azam/personal/fun/compressor/BACKEND_INTEGRATION.md).
