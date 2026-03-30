use std::path::PathBuf;
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use core::compressor::ZstdEngine;
use core::manifest::{build_manifest, save_manifest, load_manifest};
use core::progress::ProgressEvent;
use cli::fs_provider::{FileSystemProvider, fragment_writer_factory, fragment_reader_factory, file_writer_factory};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Compress a directory or file into a streaming archive
    Compress {
        /// The file or directory to compress
        input: PathBuf,

        /// The directory where the compressed archive files will be saved
        output_dir: PathBuf,

        /// Optional: Fragment size in bytes. If omitted, computes an adaptive optimal size based on CPU cores.
        #[arg(short, long)]
        fragment_size: Option<u64>,

        /// Compression level (1=fastest, 22=best ratio, default: 3)
        #[arg(short, long, default_value_t = 3, value_parser = clap::value_parser!(i32).range(1..=22))]
        level: i32,

        /// Optional: Number of worker threads (default: auto-detect from CPU cores)
        #[arg(short = 'j', long)]
        threads: Option<usize>,
    },
    /// Decompress a streaming archive back to its original state
    Decompress {
        /// The compressed archive directory containing manifest.json and .zst files
        archive_dir: PathBuf,

        /// The output directory to extract the files into
        output_dir: PathBuf,

        /// Optional: Number of worker threads (default: auto-detect from CPU cores)
        #[arg(short = 'j', long)]
        threads: Option<usize>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress { input, output_dir, fragment_size, level, threads } => {
            println!("Building manifest for {}...", input.display());
            let manifest = build_manifest(&input, fragment_size)?;

            std::fs::create_dir_all(&output_dir)?;

            let num_fragments = if manifest.total_original_size == 0 {
                0
            } else {
                manifest.total_original_size.div_ceil(manifest.fragment_size) as usize
            };

            println!("Compressing and streaming archive to {}...", output_dir.display());

            let pb = ProgressBar::new(num_fragments as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
                .progress_chars("#>-"));

            if let Some(n) = threads {
                rayon::ThreadPoolBuilder::new().num_threads(n).build_global().unwrap_or_default();
            }

            let provider = FileSystemProvider::new(&input);
            let wf = fragment_writer_factory(output_dir.clone());
            let engine = ZstdEngine::new(level);

            let mut skip_map = std::collections::HashMap::new();
            if output_dir.exists()
                && let Ok(entries) = std::fs::read_dir(&output_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file()
                            && let Some(name) = path.file_name().and_then(|n| n.to_str())
                                && name.starts_with("fragment_") && name.ends_with(".zst") {
                                    let idx_str = name.trim_start_matches("fragment_").trim_end_matches(".zst");
                                    if let Ok(idx) = idx_str.parse::<usize>()
                                        && let Ok(meta) = entry.metadata() {
                                            skip_map.insert(idx, meta.len());
                                        }
                                }
                    }
                }
            let skip_map = if skip_map.is_empty() { None } else { Some(skip_map) };

            let final_manifest = std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::channel();
                
                let handle = s.spawn(|| {
                    core::publisher::compress_archive(
                        provider,
                        manifest,
                        wf,
                        Some(tx),
                        &engine,
                        skip_map,
                    )
                });

                for event in rx {
                    if let ProgressEvent::FragmentCompleted { .. } = event { pb.inc(1) }
                }

                handle.join().unwrap()
            })?;

            pb.finish_with_message("Compression complete");

            save_manifest(&final_manifest, &output_dir.join("manifest.json"))?;
            println!("Manifest written. Compression pipeline complete.");
        }
        Commands::Decompress { archive_dir, output_dir, threads } => {
            println!("Reading manifest from {}...", archive_dir.display());
            let manifest = load_manifest(&archive_dir.join("manifest.json"))?;

            println!("Extracting archive to {}...", output_dir.display());
            
            let pb = ProgressBar::new(manifest.fragments.len() as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.magenta/blue}] {pos}/{len} ({eta})")?
                .progress_chars("#>-"));

            std::fs::create_dir_all(&output_dir)?;

            if let Some(n) = threads {
                rayon::ThreadPoolBuilder::new().num_threads(n).build_global().unwrap_or_default();
            }

            let rf = fragment_reader_factory(archive_dir.clone());
            let ff = file_writer_factory(output_dir.clone());
            let engine = ZstdEngine::new(3);

            std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::channel();

                let handle = s.spawn(|| {
                    if threads == Some(1) {
                        core::reassembler::extract_archive(
                            &manifest,
                            rf,
                            ff,
                            Some(tx),
                            &engine,
                        )
                    } else {
                        core::reassembler::parallel_extract_archive(
                            &manifest,
                            rf,
                            ff,
                            Some(tx),
                            &engine,
                        )
                    }
                });

                for event in rx {
                    if let ProgressEvent::FragmentCompleted { .. } = event { pb.inc(1) }
                }

                handle.join().unwrap()
            })?;
            
            pb.finish_with_message("Decompression complete");
            println!("Decompression complete!");
        }
    }

    Ok(())
}
