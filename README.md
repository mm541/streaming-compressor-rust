# Streaming Compressor 🚀

A high-performance, resilient streaming compressor built for massive I/O parallelization and fast browser uploading. Built in Rust (compiled to WebAssembly + Native), it achieves incredible speed by fragmenting files and processing them concurrently.

## Architecture & Integration

This repository is structured as a multi-crate workspace:
- **`core/`**: The backbone of the compressor. Contains the `manifest` generation logic and parallel zstd block compression engines.
- **`cli/`**: The native CLI tool for lightning-fast compression and extraction on your local machine.
- **`wasm/`**: The browser-native library. It supports multi-threaded worker pools, smart auto-detection (skipping already compressed formats like video), and parallel HTTP chunk uploading directly to a backend.

### Developer Guides
- 🌟 **[WASM & Frontend Integration Guide](./WASM_SUMMARY.md)**: Master the `worker.js` and `example_streaming_upload.js` logic for zero-freeze browser uploading.
- ⚙️ **[Backend Integration Guide](./BACKEND_INTEGRATION.md)**: Learn how to set up the Spring Boot Gateway, RabbitMQ broker, and Python assembler to receive and stitch the chunks together flawlessly.

---

## 🚀 Native CLI Benchmarks

We benchmarked the CLI against a massive, real-world directory (`~48.5 GB`) to compare our parallel chunking architecture against the standard `tar + multi-threaded zstd` baseline.

**Dataset:** 48.5 GB Directory
**Fragment Size:** 1 MB chunks

| Tool | Wall Time | CPU Usage | Peak RAM | Output Size | Speedup vs Tar |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tar \| zstd -T0 -3` | 100.28s | 192% | ~359 MB | 19.01 GB | Baseline |
| **Our Streaming CLI** | **43.87s** | **749%** | **~303 MB** | **20.78 GB** | **🔥 2.28x Faster** |

### Why is it so much faster?
Traditional archivers like `tar` read files sequentially, creating an I/O bottleneck before the data even reaches the multi-threaded compressor. Our native engine uses `rayon` to read, chunk, and compress multiple files concurrently from your NVMe drive. 

By compressing data in independent 1MB fragments, we trade a slight penalty in absolute compression ratio for **massive read/write parallelization (>1.1 GB/s throughput)**, while strictly enabling predictable memory limits and WASM-compatible browser streaming for massive video uploads.

---

## Native CLI Usage

To compress a directory and output the `manifest.json` and block fragments to a target folder:
```bash
cargo run --release -p cli -- compress /path/to/source /path/to/archive
```

To extract that archive back to disk exactly as it was:
```bash
cargo run --release -p cli -- decompress /path/to/archive /path/to/restore
```
