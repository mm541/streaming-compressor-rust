# Compressor CLI Reference

The Rust Streaming Compression Engine comes with a compiled CLI interface that automatically harnesses all your CPU cores for massive multi-threaded streaming archiving.

## Installation / Building
To build an optimized native release binary:
```bash
cargo build --release
# The binary will be available at ./target/release/cli
```

---

## 1. Compress an Archive

Compresses a single file or a massive nested directory perfectly into our custom mapped `.zst` fragment layout.

```bash
cli compress <INPUT> <OUTPUT_DIR> [OPTIONS]
```

### Arguments
*   `<INPUT>`: The physical file or giant nested directory you want to compress (e.g. `/home/user/personal`).
*   `<OUTPUT_DIR>`: The target directory where the engine will place the `manifest.json` and its stream of `.zst` fragments.

### Options
*   `-l, --level <LEVEL>`: The Zstandard compression density. Values range from `1` (blistering fast) to `22` (maximum space savings). Default is `3`.
*   `-j, --threads <THREADS>`: Manually restrict how many CPU worker threads to use. By default, it auto-detects your system CPU structure and uses all of them.
*   `-f, --fragment-size <BYTES>`: Control the exact byte size of the individual compressed fragments. By default, the engine mathematically auto-computes optimal slicing windows to feed your CPU cores precisely without starving threads!

### Example
```bash
cli compress /home/azam/personal ./my_archive -l 5 -j 8
```

---

## 2. Decompress an Archive

Extracts a generated stream format completely back out onto your physical disk with perfectly preserved relative folder tree structures, permissions, and sub-paths.

```bash
cli decompress <ARCHIVE_DIR> <OUTPUT_DIR> [OPTIONS]
```

### Arguments
*   `<ARCHIVE_DIR>`: Your generated compressed archive directory (this folder must contain your `manifest.json` and `.zst` fragments).
*   `<OUTPUT_DIR>`: The target extraction directory where your files will magically reappear exactly how they were.

### Options
*   `-j, --threads <THREADS>`: Manually restrict the extraction CPU workers. Defaults to matching your physical core count.

### Example
```bash
cli decompress ./my_archive ./extracted_files
```

---

## 3. Benchmarking Framework

If you wish to test your raw physical maximum I/O limit or visualize the exact scaling properties of your CPU, you can run the integrated bench suite:

```bash
# Standard 50 MB memory Benchmark
cargo run --release --bin benchmark

# Custom Dataset Memory/Filesystem Benchmark (Massively rigorous!)
cargo run --release --bin benchmark /home/azam/personal
```
*Note: The built-in benchmark automatically parses your system's `VmHWM` footprint to report exact peak RAM footprints, calculates compression ratios, and compares everything automatically.*
