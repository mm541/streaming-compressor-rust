//! Directory walker — scans a root path and collects file entries.

use std::path::PathBuf;

use anyhow::Result;
use jwalk::WalkDir;

use super::{Entry, entry_from_metadata};

/// Walk a directory tree and collect metadata for all regular files.
///
/// Uses `jwalk` for parallel directory traversal. Entries are sorted
/// by relative path for deterministic byte-stream ordering.
///
/// - Symlinks are **not** followed to avoid infinite cycles.
///   Symlinks are recorded as entries with `symlink_target` populated and `original_size = 0`.
/// - Unreadable files (permission denied, etc.) are **skipped** with a warning, not fatal.
/// - Special files (sockets, FIFOs, device files) are **skipped**.
///
/// `byte_offset` is set to 0 — call `compute_byte_offsets` afterwards.
/// `checksum` is set to None — populated later when file data is read.
pub fn walk_directory(root: &PathBuf) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut skipped = 0usize;

    // follow_links(false) prevents infinite symlink cycles
    for dir_entry in WalkDir::new(&root).follow_links(false).skip_hidden(false) {
        let dir_entry = match dir_entry {
            Ok(e) => e,
            Err(err) => {
                eprintln!("  [WARN] skipping unreadable entry: {}", err);
                skipped += 1;
                continue;
            }
        };

        let path = dir_entry.path();

        // Compute relative path early - needed for both symlinks and regular files
        let relative_path = match path.strip_prefix(&root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => {
                eprintln!("  [WARN] skipping (cannot strip prefix): {}", path.display());
                skipped += 1;
                continue;
            }
        };

        // Handle symlinks: record them but don't follow
        let file_type = match path.symlink_metadata() {
            Ok(meta) => meta.file_type(),
            Err(err) => {
                eprintln!("  [WARN] skipping (metadata error): {} — {}", path.display(), err);
                skipped += 1;
                continue;
            }
        };

        if file_type.is_symlink() {
            let target = std::fs::read_link(&path).ok();
            entries.push(Entry {
                relative_path,
                original_size: 0,
                permissions: 0o777,
                modified_at: 0,
                byte_offset: 0,
                checksum: None,
                symlink_target: target,
            });
            continue;
        }

        // Skip directories — structure is implicit from file paths
        if file_type.is_dir() {
            continue;
        }

        // Skip special files (sockets, FIFOs, device files, etc.)
        if !file_type.is_file() {
            eprintln!("  [WARN] skipping special file: {}", path.display());
            skipped += 1;
            continue;
        }

        // Regular file — read metadata
        let metadata = match path.metadata() {
            Ok(m) => m,
            Err(err) => {
                eprintln!("  [WARN] skipping (permission denied or removed): {} — {}", path.display(), err);
                skipped += 1;
                continue;
            }
        };

        entries.push(entry_from_metadata(relative_path, &metadata));
    }

    if skipped > 0 {
        eprintln!("  [INFO] skipped {} problematic entries", skipped);
    }

    // Sort by relative path for deterministic ordering
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;

    use super::walk_directory;
    use crate::manifest::{build_manifest, compute_byte_offsets};

    /// Create a temp directory with this structure:
    /// tmp/
    /// ├── hello.txt       ("hello world")
    /// ├── subdir/
    /// │   └── nested.txt  ("nested file content")
    /// └── empty.txt       ("")
    fn create_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("hello.txt"), "hello world").unwrap();
        fs::create_dir_all(root.join("subdir")).unwrap();
        fs::write(root.join("subdir/nested.txt"), "nested file content").unwrap();
        fs::write(root.join("empty.txt"), "").unwrap();

        dir
    }

    #[test]
    fn test_walk_directory_collects_files_only() {
        let dir = create_test_dir();
        let entries = walk_directory(&dir.path().to_path_buf()).unwrap();

        // Should have 3 files, no directory entries
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_walk_directory_sorted_paths() {
        let dir = create_test_dir();
        let entries = walk_directory(&dir.path().to_path_buf()).unwrap();

        let paths: Vec<_> = entries
            .iter()
            .map(|e| e.relative_path.to_string_lossy().to_string())
            .collect();

        // Should be sorted alphabetically
        assert_eq!(paths, vec!["empty.txt", "hello.txt", "subdir/nested.txt"]);
    }

    #[test]
    fn test_walk_directory_file_sizes() {
        let dir = create_test_dir();
        let entries = walk_directory(&dir.path().to_path_buf()).unwrap();

        assert_eq!(entries[0].original_size, 0);  // empty.txt
        assert_eq!(entries[1].original_size, 11); // "hello world"
        assert_eq!(entries[2].original_size, 19); // "nested file content"
    }

    #[test]
    fn test_compute_byte_offsets() {
        let dir = create_test_dir();
        let mut entries = walk_directory(&dir.path().to_path_buf()).unwrap();

        compute_byte_offsets(&mut entries);

        assert_eq!(entries[0].byte_offset, 0);   // empty.txt (size 0)
        assert_eq!(entries[1].byte_offset, 0);   // hello.txt starts at 0 (empty.txt is 0 bytes)
        assert_eq!(entries[2].byte_offset, 11);  // nested.txt starts at 11
    }

    #[test]
    fn test_build_manifest() {
        let dir = create_test_dir();
        let manifest = build_manifest(dir.path(), 1024 * 1024).unwrap();

        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.fragment_size, 1024 * 1024);
        assert_eq!(manifest.total_original_size, 30); // 0 + 11 + 19
        assert_eq!(manifest.entries.len(), 3);

        // Should serialize to valid JSON
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        assert!(json.contains("hello.txt"));
        assert!(json.contains("subdir/nested.txt"));
    }

    #[test]
    fn test_walk_directory_handles_symlinks() {
        let dir = create_test_dir();
        let root = dir.path();

        // Create a symlink
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("hello.txt"), root.join("link.txt")).unwrap();

        let entries = walk_directory(&root.to_path_buf()).unwrap();

        #[cfg(unix)]
        {
            // Find the symlink entry
            let link = entries.iter().find(|e| e.relative_path.to_str() == Some("link.txt"));
            assert!(link.is_some(), "symlink should be recorded");
            let link = link.unwrap();
            assert_eq!(link.original_size, 0, "symlink should have 0 size");
            assert!(link.symlink_target.is_some(), "symlink should have target");
        }
    }
}