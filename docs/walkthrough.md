# Compressor Project Walkthrough

## Completed Objectives

1.  **Phase 0: Manifest Generation**
    - Walks a directory recursively using `jwalk`.
    - Creates a reproducible sequential `Manifest` mapping original files to a 1D contiguous byte stream.
2.  **Phase 1: Streaming Byte Assembly**
    - Iterates the directory reading files lazily and yielding exact `fragment_size` chunks of bytes crossing contiguous file boundaries.
    - Automatically assigns checksums to the manifest entries on the fly.
3.  **Phase 2: Parallel Compression**
    - Integrates `zstd` compression algorithm.
    - Appends `FragmentMeta` holding resulting metadata post-compression.
4.  **Phase 3: Archive Output Writer**
    - Uses `rayon::par_bridge()` to consume the byte stream and compress chunks simultaneously across CPU cores dynamically.
    - Writes `.zst` fragment blobs securely to an output archive folder.
5.  **Phase 4: CLI Interface**
    - Simple `clap` CLI to `compress` and `decompress` safely.
6.  **Phase 5: Decompression and Reassembly**
    - Reads `manifest.json`.
    - Streams each `.zst` sequentially, decompresses, and slices the chunk back into individual specific directories and files flawlessly.

## Usage Guide

The binary gives you two powerful commands:

### Compressing a Directory

To squish your target directory down securely into highly optimized block chunks:

```bash
cargo run -- compress ./my_source_code ./my_archive_output --fragment-size 1048576
```

Inside `./my_archive_output/`, you will see:

- `manifest.json` — A lightweight text blueprint holding checksums, original sizes, offsets, and chunk routing rules.
- `fragment_000000.zst` — Zstandard compressed streaming blocks sequentially ordered for the unarchiver.

### Decompressing the Archive

Extract the folder securely back out retaining its explicit structural properties:

```bash
cargo run -- decompress ./my_archive_output ./my_restored_code
```

## Testing Structure

Currently, 11 tests exist ensuring data integrity:

- `manifest::walker::tests::*` guarantees sequential byte mapping parsing.
- `compressor::tests::*` verifies `zstd` encoding/decoding behavior.
- `assembler::tests::test_multiple_fragments_boundary_spanning` proves chunks accurately map bytes across boundaries correctly.
- `writer::tests::test_write_archive_end_to_end` sets up an entire stream and dynamically writes metadata concurrently tracking `manifest.segments`.
- `reassembler::tests::test_extract_archive_end_to_end` compresses mock directories randomly, then decompresses mapping bytes properly resulting in 100% data preservation verification.
