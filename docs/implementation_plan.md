# Compressor Library — Streaming Implementation Plan

## Pipeline Overview (Streaming Architecture)

```
  [Manifest]         [Assembly Stream]        [Parallel Compression]        [Output Writer]
  (DONE ✅)

  walk dir ───→ lazy reader yields    ───→ par_bridge() compresses ───→ writes directly
               fragments Iterator          each fragment in a thread    to .frag files
               (Never all in memory)                                    as they finish
```

Instead of loading the entire archive into memory, we will build a **streaming pipeline**. This bounds memory usage to just the active fragments being compressed (e.g., `< 50 MB` even for a 100 GB directory).

---

## Phase 1: Streaming Byte Stream Assembly

> Lazily read files in manifest order, yielding fragment-sized chunks via an `Iterator`.

### [MODIFY] `src/assembler.rs`

Replace the current in-memory `assemble_fragments` with a stateful `Iterator`:

**`pub struct FragmentStream<'a>`**

- Holds a reference to `&'a mut Manifest` and the `root` path.
- State tracks: `current_entry_index`, an open `File` handle, and a `blake3::Hasher`.
- **`impl<'a> Iterator for FragmentStream<'a>`**
  - **`Item = Result<Vec<u8>>`**
  - Upon calling `next()`, reads exactly `fragment_size` bytes (or until EOF of the entire archive).
  - Crosses file boundaries transparently.
  - When a file is fully read, finalizes the blake3 hash and updates `manifest.entries[i].checksum`.

This drops memory usage per fragment to exactly `fragment_size`.

---

## Phase 2: Parallel Streaming Compression

> Compress fragments as they stream in, using Rayon's parallel iterators.

### [NEW] `src/compressor.rs`

**`pub fn compress_fragment(data: &[u8], level: i32) -> Result<Vec<u8>>`**

- Simple wrapper around `zstd` to compress a single chunk.

**`pub fn decompress_fragment(data: &[u8]) -> Result<Vec<u8>>`**

- Counterpart for reassembly.

### Compression Metadata

After compression, we record the compressed size and hash of the fragment payload to ensure archive integrity:

```rust
pub struct FragmentMeta {
    pub compressed_size: u64,
    pub original_size: u64,
    pub checksum: String,  // hash of compressed bytes
}
```

Add `pub fragments: Vec<FragmentMeta>` to `Manifest`.

---

## Phase 3: Archive Output Writer

> Consume the compressed stream and write to disk immediately.

### [NEW] `src/writer.rs`

**`pub fn write_archive(output_dir: &Path, assembler: FragmentStream) -> Result<()>`**

Using `rayon`, we can make the pipeline concurrent:

```rust
// Stream fragments -> parallelize -> compress -> collect metadata
let mut fragment_metas: Vec<_> = assembler
    .enumerate()
    .par_bridge() // <--- Rayon magic: processes items concurrently
    .map(|(index, fragment_res)| {
        let fragment = fragment_res?;
        let compressed = compress_fragment(&fragment, 3)?;

        let meta = FragmentMeta { /* ... */ };

        // Write to output_dir/fragment_{index}.zst
        crate::writer::write_fragment_file(output_dir, index, &compressed)?;

        Ok((index, meta))
    })
    .collect::<Result<Vec<_>>>()?;

// Sort metadata by index (since par_bridge finishes out of order)
fragment_metas.sort_by_key(|(i, _)| *i);
```

At the end, attach `fragment_metas` to the `Manifest` (which now has all file checksums populated) and write `manifest.json`.

---

## Phase 4: CLI

**`src/main.rs`**
Use `clap` to provide the top-level commands:

- `compress <INPUT> [...]`
- `decompress <ARCHIVE_DIR> [...]`

---

## Phase 5: Decompression & Reassembly

> Reverse the pipeline: Load manifest → generate iterator mapping fragment indices to decompressed bytes → write out files.

### [NEW] `src/reassembler.rs`

- Read manifest to get file boundaries.
- Stream fragments from disk lazy-style.
- Provide a `Read` object that acts as a contiguous uncompressed byte stream across all fragments.
- Slice bytes and write them to `output_dir/file/path` based on `byte_offset` and `original_size`.
