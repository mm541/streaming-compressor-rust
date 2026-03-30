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

/// Creates a closure that produces `Write` sinks for output files, creating parent directories as needed.
pub fn file_writer_factory(output_dir: PathBuf) -> impl Fn(&str) -> Result<Box<dyn std::io::Write>> {
    move |identifier: &str| {
        let path = output_dir.join(identifier);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Box::new(File::create(&path)?) as Box<dyn std::io::Write>)
    }
}
