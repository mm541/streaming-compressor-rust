# Streaming Compressor

A high-performance streaming compression engine written in Rust. It achieves **17x faster compression** than `tar + zstd` by parallelizing the entire pipeline — filesystem traversal, chunking, and Zstandard compression — across all available CPU cores using Rayon.

## Architecture

```
┌─────────────────────────────────┐     ┌──────────────────────────────┐
│           cli/                  │     │           core/              │
│                                 │     │                              │
│  walker.rs    (jwalk traversal) │────▶│  manifest/  (pure math)      │
│  manifest_io  (JSON read/write) │     │  compressor (Zstd engine)    │
│  fs_provider  (file I/O)        │     │  publisher  (parallel comp)  │
│  main.rs      (clap CLI)        │     │  reassembler(parallel dec)   │
│                                 │     │  detection  (content-aware)  │
└─────────────────────────────────┘     └──────────────────────────────┘
         OS Boundary                         I/O-Agnostic Core
```

The project is structured as a Cargo workspace with a strict separation of concerns:

- **`core/`** — A pure algorithmic library with **zero filesystem dependencies**. It accepts data structures (not file paths) and exposes compression, decompression, manifest building, and content detection. It can be consumed by any I/O frontend.
- **`cli/`** — The native CLI binary. Handles all OS-specific operations: directory walking (`jwalk`), manifest persistence (JSON), and file I/O. It feeds data into the `core` engine.

See [ARCHITECTURE.md](./ARCHITECTURE.md) for a deep dive with detailed diagrams covering the compression pipeline, fragment layout, and extensibility model.

## Performance

Benchmarked on a real-world 7.69 GB mixed-content directory (35,633 files) with Zstandard level 3 on 20 CPU cores:

| Metric | tar + zstd | streaming-compressor |
|--------|-----------|---------------------|
| **Compress Time** | 2 min 32s | **8.92s** (17x faster) |
| **Decompress Time** | 10.61s | **8.38s** (1.3x faster) |
| **Archive Size** | 3,876 MB | 3,852 MB |
| **Compression Ratio** | 2.03x | 2.04x |
| **Compress RAM** | 101 MB | 103 MB |

The speedup comes from eliminating `tar`'s single-threaded pipe serialization. Our engine parallelizes filesystem traversal, fragmentation, and compression simultaneously, pushing the bottleneck to the SSD's physical read bandwidth.

See [BENCHMARK_RESULTS.md](./BENCHMARK_RESULTS.md) for the full methodology and environment details.

## Quick Start

```bash
# Build
cargo build --release

# Compress a directory
./target/release/cli compress ./my_data ./archive -l 3

# Decompress
./target/release/cli decompress ./archive ./restored
```

See [CLI_USAGE.md](./CLI_USAGE.md) for the full reference.

## Key Features

- **Parallel compression** — Rayon-based work-stealing across all CPU cores
- **Content-aware skipping** — Detects pre-compressed formats (JPEG, MP4, ZIP, etc.) via magic bytes and file extensions; stores them raw to avoid wasting CPU cycles
- **Adaptive fragment sizing** — Automatically computes optimal fragment sizes based on dataset size and core count to prevent thread starvation
- **Resumable compression** — Re-running compress on an existing archive skips already-completed fragments
- **Random-access extraction** — Fragment-based layout enables extracting individual files without decompressing the entire archive
- **I/O-agnostic core** — The `core` library has zero filesystem imports, making it embeddable in any context

## How It Works

1. **Walk** — The CLI traverses the input directory using `jwalk` (parallel directory walker) and collects file metadata
2. **Manifest** — The core engine builds a byte-offset manifest that maps every file into a flat virtual byte stream, then slices it into equal-sized fragments
3. **Compress** — Each fragment is compressed independently via Zstd using Rayon's thread pool. Pre-compressed content is detected and stored raw
4. **Reassemble** — On decompression, fragments are read and decompressed in parallel. Each file is reconstructed by seeking to its byte offset and writing its data directly

## Roadmap

See [FUTURE_ENHANCEMENTS.md](./FUTURE_ENHANCEMENTS.md) for planned features including archive integrity verification, encryption, dry-run mode, and stdout streaming.

## Run Benchmarks

```bash
# Built-in synthetic benchmark (50 MB in-memory)
cargo run --release --bin benchmark

# Head-to-head vs tar+zstd on real data
./benchmark.sh ./path/to/dataset
```

## License

MIT
