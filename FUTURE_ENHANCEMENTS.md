# Roadmap

Future enhancements for the Streaming Compression Engine, roughly ordered by impact.

---

## 1. `cli verify` — Archive Integrity Checks
Add a `verify` subcommand that reads the manifest and fragments, then validates checksums (e.g. XXH3 or CRC32) against stored hashes. Currently there's no way to confirm an archive isn't corrupted without fully extracting it.

**Scope:** Add a `checksum` field to `FragmentMeta`, compute it during compression, and verify during the new subcommand.

---

## 2. `--dry-run` Mode
Print what would be compressed vs. skipped (with file sizes and estimated ratios) without writing any data. Useful for estimating output before committing heavy disk I/O.

**Scope:** Add `--dry-run` flag to `compress`. Run the manifest builder and detection logic, then print a summary table and exit.

---

## 3. `cli info` — Archive Inspection
A quick subcommand that reads `manifest.json` and prints a human-readable summary:
- File count, total original size, compressed size
- Fragment count and average fragment size
- Compression ratio and algorithm used
- Creation date
- Top-N largest files

**Scope:** New subcommand, purely reads and formats manifest data.

---

## 4. Progress Reporting Improvements
Enhance the progress bar to show:
- Real-time throughput (e.g. `~450 MB/s`)
- ETA based on bytes processed rather than fragment count
- Final summary table: total size, ratio, peak RAM, wall time

**Scope:** Modify the `ProgressEvent` enum to include byte-level data and update the CLI renderer.

---

## 5. Adaptive Compression Levels Per File Type
Instead of a global `-l 3`, automatically select higher compression levels for highly compressible text (`.json`, `.log`, `.csv`) and lower levels for binaries. A middle ground between `--no-skip` and the current all-or-nothing detection.

**Scope:** Extend `detection.rs` to return a suggested compression level. Modify `publisher.rs` to vary the engine level per-fragment.

---

## 6. Archive Metadata & Comments
Let users attach a description or tags to an archive:
```bash
cli compress ./data ./archive --comment "weekly backup 2026-03-30"
```
Stored in `manifest.json` for easy identification and searchability later.

**Scope:** Add optional `comment` and `tags` fields to `Manifest`. Wire through CLI.

---

## 7. Encryption Support
Optional `--encrypt` flag that wraps each fragment in AES-256-GCM after compression, with key derivation from a user-supplied passphrase (Argon2). Makes the tool viable for cloud backup and sensitive data archival.

**Scope:** New `crypto` module in `core`. Wrap the fragment writer pipeline with an encrypting layer. Store salt/nonce in manifest.

---

## 8. Streaming to stdout
Support piping the archive to stdout for use in SSH/curl pipelines:
```bash
cli compress ./data - | ssh remote "cat > backup.sca"
```

**Scope:** Detect `-` as output target. Serialize manifest + fragments into a single sequential stream format rather than separate files.
