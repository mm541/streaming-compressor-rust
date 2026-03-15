use std::time::Instant;

use compressor::manifest::build_manifest;
use compressor::publisher::{start_pipeline, CompressEvent};
use compressor::reassembler::extract_archive;

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
    let manifest = build_manifest(input_dir, 1024 * 1024).unwrap(); // 1 MB fragments
    let total_bytes = manifest.total_original_size as usize;

    std::fs::create_dir_all(archive_dir).unwrap();

    let start = Instant::now();

    let pipeline = start_pipeline(
        input_dir.to_path_buf(),
        manifest,
        concurrency,
        128,
    );

    let n_workers = pipeline.receivers.len();

    // Drain each receiver, writing to disk
    let mut consumer_handles = Vec::new();
    for rx in pipeline.receivers {
        let out = archive_dir.to_path_buf();
        consumer_handles.push(std::thread::spawn(move || {
            let mut current_file: Option<std::fs::File> = None;
            for event in rx {
                match event.unwrap() {
                    CompressEvent::Start { fragment_idx } => {
                        let path = out.join(format!("fragment_{:06}.zst", fragment_idx));
                        current_file = Some(std::fs::File::create(&path).unwrap());
                    }
                    CompressEvent::Chunk { data } => {
                        std::io::Write::write_all(current_file.as_mut().unwrap(), &data).unwrap();
                    }
                    CompressEvent::Complete { .. } => {
                        current_file = None;
                    }
                }
            }
        }));
    }

    for h in consumer_handles {
        h.join().unwrap();
    }

    let manifest = pipeline.handle.join().unwrap().unwrap();
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
    std::fs::write(archive_dir.join("manifest.json"), manifest_json).unwrap();

    let elapsed = start.elapsed();

    let compressed_size: u64 = manifest.fragments.iter().map(|f| f.compressed_size).sum();
    let ratio = (compressed_size as f64 / total_bytes as f64) * 100.0;

    println!("  Compression: {:.2?} | {} workers | {:.1} MB/s | ratio: {:.1}%",
        elapsed,
        n_workers,
        (total_bytes as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64(),
        ratio,
    );

    (elapsed, total_bytes)
}

fn bench_decompress(archive_dir: &std::path::Path, output_dir: &std::path::Path) -> std::time::Duration {
    let manifest_content = std::fs::read_to_string(archive_dir.join("manifest.json")).unwrap();
    let manifest: compressor::manifest::Manifest = serde_json::from_str(&manifest_content).unwrap();
    let total_bytes = manifest.total_original_size;

    let start = Instant::now();
    extract_archive(archive_dir, output_dir, &manifest, |_| {}).unwrap();
    let elapsed = start.elapsed();

    println!("  Decompression: {:.2?} | {:.1} MB/s",
        elapsed,
        (total_bytes as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64(),
    );

    elapsed
}

fn main() {
    let test_size_mb: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    println!("═══════════════════════════════════════════════");
    println!(" Compressor Benchmark — {} MB test data", test_size_mb);
    println!("═══════════════════════════════════════════════");

    let base = tempfile::TempDir::new().unwrap();
    let input_dir = base.path().join("input");
    std::fs::create_dir_all(&input_dir).unwrap();

    println!("\n[1/4] Generating test data...");
    generate_test_data(&input_dir, test_size_mb);

    // --- Benchmark: Auto concurrency ---
    println!("\n[2/4] Compress (auto concurrency)...");
    let archive_auto = base.path().join("archive_auto");
    let (compress_auto, total_bytes) = bench_compress(&input_dir, &archive_auto, None);

    println!("\n[3/4] Decompress (streaming)...");
    let output_auto = base.path().join("output_auto");
    let decompress_auto = bench_decompress(&archive_auto, &output_auto);

    // --- Benchmark: Single thread ---
    println!("\n[4/4] Compress (1 thread, baseline)...");
    let archive_single = base.path().join("archive_single");
    let (compress_single, _) = bench_compress(&input_dir, &archive_single, Some(1));

    // --- Summary ---
    let speedup = compress_single.as_secs_f64() / compress_auto.as_secs_f64();

    println!("\n═══════════════════════════════════════════════");
    println!(" RESULTS");
    println!("═══════════════════════════════════════════════");
    println!("  Data size:         {} MB", test_size_mb);
    println!("  Compress (auto):   {:.2?} ({:.1} MB/s)", compress_auto, (total_bytes as f64 / 1024.0 / 1024.0) / compress_auto.as_secs_f64());
    println!("  Compress (1 thr):  {:.2?} ({:.1} MB/s)", compress_single, (total_bytes as f64 / 1024.0 / 1024.0) / compress_single.as_secs_f64());
    println!("  Speedup:           {:.2}x", speedup);
    println!("  Decompress:        {:.2?} ({:.1} MB/s)", decompress_auto, (total_bytes as f64 / 1024.0 / 1024.0) / decompress_auto.as_secs_f64());
    println!("═══════════════════════════════════════════════");
}
