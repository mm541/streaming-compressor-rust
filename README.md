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

We meticulously benchmarked the CLI against a massive, highly-fragmented real-world directory (`~47 GB`) directly on an NVMe SSD to compare our 128KB passthrough arrays and thread-sorting architecture against the absolute fastest standard `tar + multi-threaded zstd` baseline.

**Dataset:** 47.0 GB Physical Directory
**Compression Algorithm:** Zstandard Level 3

| Tool | Wall Time | CPU Usage | Peak RAM | Speedup vs Tar |
| :--- | :--- | :--- | :--- | :--- |
| `tar \| zstd -T0 -3` | 221.04s | 110% | ~344 MB | Baseline |
| **Our Streaming Engine** | **45.80s** | **1146%** | **~246 MB** | **🔥 4.82x Faster** |

### Why is our engine structurally faster?
Traditional archivers like `tar` walk physical directories sequentially, creating a massive I/O bottleneck before the byte-data even reaches the multi-threaded compressor. Our custom Rust engine does not wait for I/O sweeps. It utilizes asymmetric thread queues and huge zero-allocation `128KB` bypass pools to push dynamically mapped file blocks natively to your NVMe drive over 20 independent CPU threads simultaneously. 

We functionally decouple the disk from the algorithm, fundamentally allowing your CPU cores to completely saturate write-bandwidths perfectly securely resulting in blazing **4.8x absolute speedups** using **28% less memory** natively.

---

## 🛠️ CLI Usage & Quickstart

For full instructions on configuring manual core-limits, dynamic fragment-sizing, or testing the built-in raw machine telemetry suite, check out our **[CLI Usage Manual](./CLI_USAGE.md)**!

```bash
# Squeeze a directory into a highly parallel streaming structure:
cargo run --release --bin cli compress /path/to/source ./archive_output -j 20

# Safely inflate the dynamic fragment stream back into original source files:
cargo run --release --bin cli decompress ./archive_output ./restored_files
```
