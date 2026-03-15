# Compressor Library — Architecture & Implementation

## Pipeline Overview

```
  [Manifest]         [Assembly]        [Compression]       [Output]
  (Phase 0 ✅)       (Phase 1 ✅)      (Phase 2 ✅)        (Phase 3 ✅)

  walk dir ──→ entries ──→ fragment ──→ compress each ──→ write archive
               + offsets    chunks      fragment           (manifest.json
                                        (parallel)          + .frag files)
```

---

## Phase 1: Byte Stream Assembly & Fragmentation

> Reads files in manifest order and splits them lazily into fragment-sized chunks.

### `src/assembler.rs`

**`FragmentStream` (Iterator)**

- Iterates entries in order, reads file bytes lazily.
- Fills a buffer of `fragment_size` bytes; when full, yields the chunk.
- A single file may span multiple fragments seamlessly.
- Computes checksums (blake3) per file during reading, updating `manifest.entries[i].checksum`.

_(See [Assembler Diagram](./assembler_diagram.md) for detailed flowchart logic)_

---

## Phase 2: Compression

> Compresses each fragment independently, in parallel.

### `src/compressor.rs`

**`compress_fragment(data: &[u8], level: i32) -> Result<Vec<u8>>`**

- Encodes fragment using `zstd`.

**`decompress_fragment(data: &[u8]) -> Result<Vec<u8>>`**

- Single fragment decompression (used during reassembly).

### Compression metadata

After compression, each fragment's output size and hash is stored in:

```rust
pub struct FragmentMeta {
    pub compressed_size: u64,
    pub original_size: u64,
    pub checksum: String,  // hash of compressed bytes
}
```

Stored in `manifest.fragments`.

---

## Phase 3: Archive Output Format

> Writes the final archive to disk.

### `src/writer.rs`

**Output structure** (directory-based format):

```
output_dir/
├── manifest.json         # The manifest with all metadata
├── fragment_000000.zst   # Compressed fragment 0
├── fragment_000001.zst   # Compressed fragment 1
└── ...
```

**`write_archive(output_dir: &Path, assembler: FragmentStream, manifest: Manifest) -> Result<()>`**

- Consumes the iterator using `rayon::par_bridge()` to chunk and compress across all CPU cores dynamically.
- Writes each compressed fragment directly to disk.
- Finally serializes and writes `manifest.json`.

---

## Phase 4: CLI

> User-facing binary using `clap`.

### `src/main.rs`

**Subcommands**:

```
compressor compress <INPUT> <OUTPUT_DIR> [OPTIONS]
  -f, --fragment-size <SIZE>  Fragment size (default: 1048576)

compressor decompress <ARCHIVE_DIR> <OUTPUT_DIR>
```

---

## Phase 5: Decompression & Reassembly

> Reverses the process: reads archive → decompresses → writes files.

### `src/reassembler.rs`

**`extract_archive(archive_dir: &Path, output_dir: &Path, manifest: &Manifest) -> Result<()>`**

1. Parses `manifest.json`.
2. Iterates over `.zst` fragments in order.
3. Decompresses the block.
4. Uses `bytes_written` trackers combined with `manifest.entries` sizes to dynamically split the chunk into exactly the right sized bytes for each specific file path.
5. Handles empty files naturally and creates nested directories as needed.
