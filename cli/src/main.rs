use std::path::PathBuf;
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use core::manifest::{build_manifest, save_manifest, load_manifest};

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

        /// Optional: Fragment size in bytes (default: 1,048,576 bytes / 1MB)
        #[arg(short, long, default_value_t = 1048576)]
        fragment_size: u64,

        /// Optional: Number of worker threads (default: auto-detect from CPU cores)
        #[arg(short, long)]
        threads: Option<usize>,
    },
    /// Decompress a streaming archive back to its original state
    Decompress {
        /// The compressed archive directory containing manifest.json and .zst files
        archive_dir: PathBuf,

        /// The output directory to extract the files into
        output_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress { input, output_dir, fragment_size, threads } => {
            println!("Building manifest for {}...", input.display());
            let manifest = build_manifest(&input, fragment_size)?;

            std::fs::create_dir_all(&output_dir)?;

            let num_fragments = if manifest.total_original_size == 0 {
                0
            } else {
                ((manifest.total_original_size + manifest.fragment_size - 1) / manifest.fragment_size) as usize
            };

            println!("Compressing and streaming archive to {}...", output_dir.display());

            let pb = ProgressBar::new(num_fragments as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
                .progress_chars("#>-"));

            if let Some(n) = threads {
                rayon::ThreadPoolBuilder::new().num_threads(n).build_global().unwrap_or_default();
            }

            // Rayon processes the fragments, blocking until fully complete
            let final_manifest = cli::publisher::compress_archive(
                input.clone(),
                output_dir.clone(),
                manifest
            )?;

            pb.finish_with_message("Compression complete");

            save_manifest(&final_manifest, &output_dir.join("manifest.json"))?;
            println!("Manifest written. Compression pipeline complete.");
        }
        Commands::Decompress { archive_dir, output_dir } => {
            println!("Reading manifest from {}...", archive_dir.display());
            let manifest = load_manifest(&archive_dir.join("manifest.json"))?;

            println!("Extracting archive to {}...", output_dir.display());
            
            let pb = ProgressBar::new(manifest.fragments.len() as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.magenta/blue}] {pos}/{len} ({eta})")?
                .progress_chars("#>-"));

            cli::reassembler::extract_archive(&archive_dir, &output_dir, &manifest, |_| {
                pb.inc(1);
            })?;
            
            pb.finish_with_message("Decompression complete");
            println!("Decompression complete!");
        }
    }

    Ok(())
}
