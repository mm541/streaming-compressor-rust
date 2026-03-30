use std::time::Instant;

use core::compressor::ZstdEngine;
use core::manifest::build_manifest;
use core::publisher::compress_archive;

use cli::fs_provider::{FileSystemProvider, fragment_writer_factory, fragment_reader_factory, file_writer_factory_at, file_initializer};

fn generate_test_data(dir: &std::path::Path, total_mb: usize) {
    // Create a mix of file sizes to simulate real workloads
    let mut remaining = total_mb * 1024 * 1024;
    let mut file_idx = 0;

    // Some subdirectories
    std::fs::create_dir_all(dir.join("subdir_a")).unwrap();
    std::fs::create_dir_all(dir.join("subdir_b/nested")).unwrap();

    while remaining > 0 {
        let size = if remaining > 512 * 1024 {
            // Alternate between small and large files
            if file_idx % 3 == 0 { 256 * 1024 }        // 256 KB
            else if file_idx % 3 == 1 { 1024 * 1024 }  // 1 MB
            else { 64 * 1024 }                           // 64 KB
        } else {
            remaining
        };

        let size = size.min(remaining);

        // Generate pseudo-random but compressible data
        let data: Vec<u8> = (0..size)
            .map(|i| {
                let base = (i % 256) as u8;
                let noise = ((i * 7 + 13) % 64) as u8;
                base.wrapping_add(noise)
            })
            .collect();

        let subdir = match file_idx % 4 {
            0 => "".to_string(),
            1 => "subdir_a/".to_string(),
            2 => "subdir_b/".to_string(),
            _ => "subdir_b/nested/".to_string(),
        };

        let path = dir.join(format!("{}file_{:04}.bin", subdir, file_idx));
        std::fs::write(&path, &data).unwrap();

        remaining -= size;
        file_idx += 1;
    }

    println!("  Generated {} files ({} MB)", file_idx, total_mb);
}

fn bench_compress(input_dir: &std::path::Path, archive_dir: &std::path::Path, concurrency: Option<usize>) -> (std::time::Duration, usize) {
    let manifest = build_manifest(input_dir, Some(1024 * 1024)).unwrap(); // 1 MB fragments
    let total_bytes = manifest.total_original_size as usize;

    std::fs::create_dir_all(archive_dir).unwrap();

    let start = Instant::now();

    let provider = FileSystemProvider::new(input_dir);
    let wf = fragment_writer_factory(archive_dir.to_path_buf());
    let engine = ZstdEngine::new(3);

    if let Some(n) = concurrency {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap();
        pool.install(|| {
            let manifest = compress_archive(
                provider,
                manifest,
                wf,
                None,
                &engine,
                None,
                true,
            ).unwrap();
            let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
            std::fs::write(archive_dir.join("manifest.json"), manifest_json).unwrap();
        });
    } else {
        let manifest = compress_archive(
            provider,
            manifest,
            wf,
            None,
            &engine,
            None,
            true,
        ).unwrap();
        let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(archive_dir.join("manifest.json"), manifest_json).unwrap();
    }

    let elapsed = start.elapsed();

    let manifest_content = std::fs::read_to_string(archive_dir.join("manifest.json")).unwrap();
    let manifest: core::manifest::Manifest = serde_json::from_str(&manifest_content).unwrap();

    let compressed_size: u64 = manifest.fragments.iter().map(|f| f.compressed_size).sum();
    let ratio = (compressed_size as f64 / total_bytes as f64) * 100.0;

    let n_workers = concurrency.unwrap_or_else(rayon::current_num_threads);

    println!("  Compression: {:.2?} | {} workers | {:.1} MB/s | ratio: {:.1}%",
        elapsed,
        n_workers,
        (total_bytes as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64(),
        ratio,
    );

    (elapsed, total_bytes)
}

fn bench_decompress(archive_dir: &std::path::Path, output_dir: &std::path::Path, concurrency: Option<usize>) -> std::time::Duration {
    let manifest_content = std::fs::read_to_string(archive_dir.join("manifest.json")).unwrap();
    let manifest: core::manifest::Manifest = serde_json::from_str(&manifest_content).unwrap();
    let total_bytes = manifest.total_original_size;

    std::fs::create_dir_all(output_dir).unwrap();

    if let Some(threads) = concurrency {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| {
            let rf = fragment_reader_factory(archive_dir.to_path_buf());
            let fi = file_initializer(output_dir.to_path_buf());
            let ff = file_writer_factory_at(output_dir.to_path_buf());
            let engine = ZstdEngine::new(3);

            let start = Instant::now();
            if threads == 1 {
                core::reassembler::extract_archive(&manifest, rf, fi, ff, None, &engine).unwrap();
            } else {
                core::reassembler::parallel_extract_archive(&manifest, rf, fi, ff, None, &engine).unwrap();
            }
            let elapsed = start.elapsed();
            return elapsed;
        });
    }
    
    let rf = fragment_reader_factory(archive_dir.to_path_buf());
    let fi = file_initializer(output_dir.to_path_buf());
    let ff = file_writer_factory_at(output_dir.to_path_buf());
    let engine = ZstdEngine::new(3);

    let start = Instant::now();
    core::reassembler::parallel_extract_archive(&manifest, rf, fi, ff, None, &engine).unwrap();
    let elapsed = start.elapsed();

    let n_workers = concurrency.unwrap_or_else(rayon::current_num_threads);

    println!("  Decompression: {:.2?} | {} workers | {:.1} MB/s",
        elapsed,
        n_workers,
        (total_bytes as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64(),
    );

    elapsed
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first_arg = args.next();

    let base_temp = tempfile::TempDir::new().unwrap();
    let base = base_temp.path();
    let input_path;

    if let Some(arg) = first_arg {
        if let Ok(mb) = arg.parse::<usize>() {
            println!("═══════════════════════════════════════════════");
            println!(" Compressor Benchmark — {} MB test data", mb);
            println!("═══════════════════════════════════════════════");
            let gen_dir = base.join("input");
            std::fs::create_dir_all(&gen_dir).unwrap();
            println!("\n[1/5] Generating test data...");
            generate_test_data(&gen_dir, mb);
            input_path = gen_dir;
        } else {
            let path = std::path::PathBuf::from(arg);
            if !path.exists() {
                eprintln!("Error: Path '{}' does not exist.", path.display());
                std::process::exit(1);
            }
            println!("═══════════════════════════════════════════════");
            println!(" Compressor Benchmark — Target: {}", path.display());
            println!("═══════════════════════════════════════════════");
            input_path = path;
            println!("\n[1/5] Using existing data at '{}'. Skipping generation...", input_path.display());
        }
    } else {
        println!("═══════════════════════════════════════════════");
        println!(" Compressor Benchmark — 50 MB test data");
        println!("═══════════════════════════════════════════════");
        let gen_dir = base.join("input");
        std::fs::create_dir_all(&gen_dir).unwrap();
        println!("\n[1/5] Generating test data...");
        generate_test_data(&gen_dir, 50);
        input_path = gen_dir;
    }

    // --- Benchmark: Auto concurrency ---
    println!("\n[2/5] Compress (auto concurrency)...");
    let archive_auto = base.join("archive_auto");
    let (compress_auto, total_bytes) = bench_compress(&input_path, &archive_auto, None);

    let current_peak_ram = if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        status.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| {
            l.split_whitespace().nth(1).and_then(|s| s.parse::<f64>().ok())
        }).map(|kb| kb / 1024.0)
    } else { None };
    if let Some(ram) = current_peak_ram {
        println!("  Peak RAM (Compression): {:.2} MB", ram);
    }

    println!("\n[3/5] Decompress (auto concurrency)...");
    let output_auto = base.join("output_auto");
    let decompress_auto = bench_decompress(&archive_auto, &output_auto, None);

    let mut compressed_size = 0;
    if let Ok(entries) = std::fs::read_dir(&archive_auto) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    compressed_size += meta.len();
                }
            }
        }
    }

    // --- Benchmark: Single thread ---
    println!("\n[4/5] Compress (1 thread, baseline)...");
    let archive_single = base.join("archive_single");
    let (compress_single, _) = bench_compress(&input_path, &archive_single, Some(1));

    println!("\n[5/5] Decompress (1 thread, baseline)...");
    let output_single = base.join("output_single");
    let decompress_single = bench_decompress(&archive_single, &output_single, Some(1));

    let speedup_c = compress_single.as_secs_f64() / compress_auto.as_secs_f64();
    let speedup_d = decompress_single.as_secs_f64() / decompress_auto.as_secs_f64();

    let ratio = if compressed_size > 0 { total_bytes as f64 / compressed_size as f64 } else { 0.0 };
    
    let peak_ram = if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        status.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| {
            l.split_whitespace().nth(1).and_then(|s| s.parse::<f64>().ok())
        }).map(|kb| kb / 1024.0)
    } else { None };

    println!("\n═══════════════════════════════════════════════");
    println!(" RESULTS");
    println!("═══════════════════════════════════════════════");
    println!("  Data size:         {:.2} MB (Original)", total_bytes as f64 / 1024.0 / 1024.0);
    println!("  Archive size:      {:.2} MB (Compressed)", compressed_size as f64 / 1024.0 / 1024.0);
    println!("  Compression ratio: {:.2}x", ratio);
    if let Some(ram) = peak_ram {
        println!("  Peak RAM (VmHWM):  {:.2} MB", ram);
    }
    println!("  ---------------------------------------------");
    println!("  Compress (auto):   {:.2?} ({:.1} MB/s)", compress_auto, (total_bytes as f64 / 1024.0 / 1024.0) / compress_auto.as_secs_f64());
    println!("  Compress (1 thr):  {:.2?} ({:.1} MB/s)", compress_single, (total_bytes as f64 / 1024.0 / 1024.0) / compress_single.as_secs_f64());
    println!("  C-Speedup:         {:.2}x", speedup_c);
    println!("  ---------------------------------------------");
    println!("  Decompress (auto): {:.2?} ({:.1} MB/s)", decompress_auto, (total_bytes as f64 / 1024.0 / 1024.0) / decompress_auto.as_secs_f64());
    println!("  Decompress(1 thr): {:.2?} ({:.1} MB/s)", decompress_single, (total_bytes as f64 / 1024.0 / 1024.0) / decompress_single.as_secs_f64());
    println!("  D-Speedup:         {:.2}x", speedup_d);
    println!("═══════════════════════════════════════════════");
}
