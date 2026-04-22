# CLI Reference

## Installation

```bash
cargo build --release
# Binary: ./target/release/cli
```

---

## Commands

### `compress` — Compress a directory or file

```bash
cli compress <INPUT> <OUTPUT_DIR> [OPTIONS]
```

**Arguments:**
- `<INPUT>` — The file or directory to compress
- `<OUTPUT_DIR>` — Target directory for the `manifest.json` and `.zst` fragment files

**Options:**

| Flag | Description | Default |
|------|-------------|---------|
| `-l, --level <LEVEL>` | Zstandard compression level (1 = fastest, 22 = best ratio) | `3` |
| `-j, --threads <N>` | Number of worker threads | Auto-detect (all cores) |
| `-f, --fragment-size <SIZE>` | Fragment size in human-readable format (e.g. `500MB`, `1.5GB`) | Auto-computed based on dataset and core count |
| `--no-skip` | Disable content-aware skipping. Forces Zstd on all data, including pre-compressed formats | Off (smart skipping enabled) |

**Examples:**
```bash
# Standard compression with smart detection
cli compress ./my_data ./archive -l 5 -j 8 -f 500MB

# Force compress everything, including pre-compressed media
cli compress ./my_data ./archive --no-skip

# Maximum compression ratio (slower)
cli compress ./my_data ./archive -l 19
```

**Content-aware skipping:** By default, the engine detects pre-compressed files (JPEG, PNG, MP4, ZIP, Zstd, etc.) via magic bytes and file extensions. These files are stored raw without re-compression, avoiding wasted CPU cycles and archive inflation.

**Resumable:** If the output directory already contains fragments from a previous run, the engine detects them and skips re-compression. This makes it safe to interrupt and resume large compression jobs.

---

### `decompress` — Extract a streaming archive

```bash
cli decompress <ARCHIVE_DIR> <OUTPUT_DIR> [OPTIONS]
```

**Arguments:**
- `<ARCHIVE_DIR>` — The compressed archive directory (must contain `manifest.json` and `.zst` fragment files)
- `<OUTPUT_DIR>` — Target directory where files will be extracted with their original directory structure and permissions preserved

**Options:**

| Flag | Description | Default |
|------|-------------|---------|
| `-j, --threads <N>` | Number of worker threads | Auto-detect (all cores) |

**Example:**
```bash
cli decompress ./archive ./extracted
```

---

## Benchmarking

### Built-in synthetic benchmark

Runs an in-memory 50 MB compression/decompression test to measure raw throughput and auto-concurrency scaling:

```bash
cargo run --release --bin benchmark
```

Reports compression/decompression throughput (MB/s), speedup from parallelism, peak RAM, and compression ratio.

### Head-to-head vs tar + zstd

A shell script is included to benchmark against `tar | zstd` on any real dataset:

```bash
./benchmark.sh ./path/to/dataset        # Zstd level 3 (default)
./benchmark.sh ./path/to/dataset 5      # Custom Zstd level
```

The script runs compress and decompress for both tools sequentially, measures wall time, peak RAM, archive size, and compression ratio, then prints a comparison table.

---

## Archive Format

The output directory contains:
- `manifest.json` — A JSON file describing all archived files, their byte offsets, original sizes, permissions, and fragment layout
- `fragment_NNNNNN.zst` — Zstandard-compressed data fragments, each containing a slice of the virtual byte stream

This fragment-based layout enables random-access extraction and parallel decompression.
