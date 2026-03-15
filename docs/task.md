# Compressor Library — Progress

## Phase 0: Manifest Generation ✅

- [x] Project setup (cargo init, deps, module structure)
- [x] Manifest types (`Manifest`, `Entry`)
- [x] Directory walker (`walk_directory` with jwalk)
- [x] Shared `entry_from_metadata` helper
- [x] `compute_byte_offsets` + `build_manifest`
- [x] File vs directory detection (`is_directory`)
- [x] Fragment size validation (1MB–4GB)
- [x] Cross-platform permissions
- [x] CLI main.rs (save manifest JSON to /tmp)
- [x] Tests (5 passing)
- [x] Documentation (WALKTHROUGH.md, IMPLEMENTATION_PLAN.md)

## Phase 1: Streaming Byte Stream Assembly

- [ ] Create `FragmentStream` iterator in `assembler.rs`
- [ ] Lazily read files into `fragment_size` chunks
- [ ] Maintain file checksum state across boundaries
- [ ] Tests for the streaming assembler

## Phase 2: Compression

- [x] Create `compressor.rs` module
- [x] Parallel compression with rayon + zstd
- [x] Decompression support

## Phase 3: Archive Output

- [x] Create `writer.rs` module
- [x] Write fragments + manifest to disk

## Phase 4: CLI

- [x] clap-based CLI with compress/decompress subcommands

## Phase 5: Decompression & Reassembly

- [x] Create `reassembler.rs` module
