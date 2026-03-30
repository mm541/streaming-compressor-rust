use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::Result;
use core::stream::StreamProvider;

/// A local disk provider for FragmentReader when compiling for native environments.
#[derive(Clone)]
pub struct FileSystemProvider {
    root: PathBuf,
}

impl FileSystemProvider {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }
}

impl StreamProvider<File> for FileSystemProvider {
    fn provide_stream(&self, identifier: &str) -> Result<File> {
        let path = self.root.join(identifier);
        let file = File::open(&path)?;
        Ok(file)
    }
}

/// Creates a closure that produces `File` writers for compressed fragments.
pub fn fragment_writer_factory(output_dir: PathBuf) -> impl Fn(usize) -> Result<File> + Send + Sync {
    move |idx: usize| {
        let path = output_dir.join(format!("fragment_{:06}.zst", idx));
        Ok(File::create(&path)?)
    }
}

/// Creates a closure that produces `Read` streams for compressed fragments.
pub fn fragment_reader_factory(archive_dir: PathBuf) -> impl Fn(usize) -> Result<Box<dyn std::io::Read>> + Send + Sync {
    move |idx: usize| {
        let path = archive_dir.join(format!("fragment_{:06}.zst", idx));
        Ok(Box::new(File::open(&path)?) as Box<dyn std::io::Read>)
    }
}

/// Pre-allocates and truncates a target output file natively building directories if required.
pub fn file_initializer(output_dir: PathBuf) -> impl Fn(&str, u64) -> Result<()> + Send + Sync + Clone {
    move |identifier: &str, size: u64| {
        let path = output_dir.join(identifier);
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let f = File::create(&path)?;
        f.set_len(size)?;
        Ok(())
    }
}

/// Creates a closure that produces `Write` sinks pointing mathematically at extreme offset values over random-access descriptors.
pub fn file_writer_factory_at(output_dir: PathBuf) -> impl Fn(&str, u64) -> Result<Box<dyn std::io::Write>> + Send + Sync + Clone {
    move |identifier: &str, offset: u64| {
        let path = output_dir.join(identifier);
        let mut file = std::fs::OpenOptions::new().write(true).open(&path)?;
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(offset))?;
        Ok(Box::new(file) as Box<dyn std::io::Write>)
    }
}
