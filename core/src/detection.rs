//! Content-aware detection to identify already-compressed files.
//!
//! This module detects known compressed formats via magic bytes and
//! file extensions, allowing the compression engine to skip re-compression
//! of data that would not benefit from it.

/// Well-known magic byte signatures for compressed/binary formats.
const MAGIC_BYTES: &[(&[u8], &str)] = &[
    // Image formats (already compressed)
    (&[0xFF, 0xD8, 0xFF],                "JPEG"),
    (&[0x89, 0x50, 0x4E, 0x47],          "PNG"),
    (&[0x47, 0x49, 0x46, 0x38],          "GIF"),
    (&[0x52, 0x49, 0x46, 0x46],          "WEBP/AVI/WAV"),  // RIFF container

    // Archive/compression formats
    (&[0x50, 0x4B, 0x03, 0x04],          "ZIP"),
    (&[0x1F, 0x8B],                       "GZIP"),
    (&[0x28, 0xB5, 0x2F, 0xFD],          "ZSTD"),
    (&[0x04, 0x22, 0x4D, 0x18],          "LZ4"),
    (&[0xFD, 0x37, 0x7A, 0x58, 0x5A],   "XZ"),
    (&[0x42, 0x5A, 0x68],                "BZIP2"),
    (&[0x37, 0x7A, 0xBC, 0xAF],          "7Z"),

    // Video formats
    // MP4/MOV: byte 4-7 = "ftyp"
    // Handled separately in check_mp4_magic()

    // Audio formats  
    (&[0x49, 0x44, 0x33],                "MP3 (ID3)"),
    (&[0xFF, 0xFB],                       "MP3 (sync)"),
    (&[0xFF, 0xF3],                       "MP3 (sync)"),
    (&[0x66, 0x4C, 0x61, 0x43],          "FLAC"),
    (&[0x4F, 0x67, 0x67, 0x53],          "OGG"),

    // Other binary formats
    (&[0x25, 0x50, 0x44, 0x46],          "PDF"),
];

/// File extensions known to be pre-compressed or incompressible.
const COMPRESSED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "jxl",
    // Video
    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v",
    // Audio
    "mp3", "aac", "ogg", "opus", "flac", "m4a", "wma",
    // Archives
    "zip", "gz", "bz2", "xz", "zst", "lz4", "br", "7z", "rar", "tar.gz", "tgz",
    // Other
    "pdf", "woff2", "woff",
];

/// Check if content appears to be already compressed based on magic bytes.
fn has_compressed_magic(header: &[u8]) -> bool {
    if header.len() < 2 {
        return false;
    }

    // Check standard magic bytes
    for (magic, _name) in MAGIC_BYTES {
        if header.len() >= magic.len() && header.starts_with(magic) {
            return true;
        }
    }

    // Special check for MP4/MOV: bytes 4-7 should be "ftyp"
    if header.len() >= 8 && &header[4..8] == b"ftyp" {
        return true;
    }

    false
}

/// Check if a file extension indicates pre-compressed content.
fn has_compressed_extension(identifier: &str) -> bool {
    let lower = identifier.to_ascii_lowercase();
    COMPRESSED_EXTENSIONS.iter().any(|ext| lower.ends_with(&format!(".{}", ext)))
}

/// Determine whether a file is likely already compressed.
///
/// Uses both magic byte detection (more reliable) and extension fallback.
/// If `header` is empty, falls back to extension-only check.
pub fn is_compressible(identifier: &str, header: &[u8]) -> bool {
    // If we can detect magic bytes, use that (most reliable)
    if !header.is_empty() && has_compressed_magic(header) {
        return false;
    }
    
    // Fallback: check file extension
    if has_compressed_extension(identifier) {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jpeg_detection() {
        assert!(!is_compressible("photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!is_compressible("photo.jpeg", &[]));
    }

    #[test]
    fn test_png_detection() {
        assert!(!is_compressible("image.png", &[0x89, 0x50, 0x4E, 0x47]));
    }

    #[test]
    fn test_mp4_detection() {
        let header = [0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p'];
        assert!(!is_compressible("video.mp4", &header));
    }

    #[test]
    fn test_zstd_detection() {
        assert!(!is_compressible("data.zst", &[0x28, 0xB5, 0x2F, 0xFD]));
    }

    #[test]
    fn test_text_is_compressible() {
        assert!(is_compressible("readme.md", b"# Hello World"));
        assert!(is_compressible("data.json", b"{\"key\": \"value\"}"));
        assert!(is_compressible("code.rs", b"fn main() {}"));
    }

    #[test]
    fn test_extension_fallback() {
        // No header, but extension indicates compressed
        assert!(!is_compressible("archive.7z", &[]));
        assert!(!is_compressible("font.woff2", &[]));
        assert!(!is_compressible("audio.opus", &[]));
    }

    #[test]
    fn test_unknown_is_compressible() {
        assert!(is_compressible("unknown.dat", &[0x00, 0x01, 0x02, 0x03]));
    }
}
