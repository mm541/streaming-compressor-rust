# Benchmark: streaming-compressor vs tar + zstd

## Environment
| Parameter | Value |
|-----------|-------|
| **Dataset** | `mixed_codebase_and_assets/` |
| **Dataset Size** | 7.69 GB (7,876 MB) |
| **File Count** | 35,633 |
| **CPU** | 20 cores |
| **RAM** | 16 GB |
| **Zstd Level** | 3 |
| **Zstd CLI Version** | v1.5.5 |
| **Mode** | `--no-skip` (force compress all data) |

## Results

| Metric | tar + zstd | streaming-compressor | Winner |
|--------|-----------|---------------------|--------|
| **Compress Time** | 2 min 32.68s | **8.92s** | ⚡ **17x faster** |
| **Compress RAM** | 101 MB | 103 MB | ≈ tie |
| **Archive Size** | 3,876 MB | 3,852 MB | ≈ tie |
| **Compression Ratio** | 2.03x | 2.04x | ≈ tie |
| **Decompress Time** | 10.61s | **8.38s** | ⚡ **1.3x faster** |
| **Decompress RAM** | 6 MB | 89 MB | tar wins |

## Key Takeaways

- **17x faster compression** on a real 7.69 GB mixed-content directory (35K files)
- Achieves virtually **identical compression ratios** — no quality sacrifice
- Uses **comparable peak RAM** (~103 MB) during compression
- Decompression is **1.3x faster** with Rayon-parallelized random-access writes
- Both tools used Zstd level 3 for a fair comparison

## Architecture Advantages

| Feature | tar + zstd | streaming-compressor |
|---------|-----------|---------------------|
| Parallel compression | Single pipeline (tar serial → zstd parallel) | Full Rayon parallel: walk, fragment, compress |
| Random-access extraction | ❌ Must decompress entire archive | ✅ Per-fragment random access |
| Streaming decompression | ❌ Requires full archive on disk | ✅ Fragment-at-a-time |
| Memory model | Full stream buffering | O(fragment_size) bounded |
| Resumable compression | ❌ | ✅ Existing fragments skipped |
