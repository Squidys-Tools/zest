//! File-kind detection from extensions (PRD §What It Can Convert).

use std::path::Path;

/// Broad bucket used for menu filtering. Conversion requires a uniform kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Image,
    Video,
    Audio,
    TextData,
    Archive,
    Folder,
    Other,
}

impl FileKind {
    /// Classify by lowercase extension (no dot). `None`/unknown → Other.
    /// Folders are detected by the caller (see `classify_path`).
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            // Images (WIC primary, `image`/`resvg` fallback; SVG input only)
            "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tiff" | "tif" | "webp" | "heic" | "heif"
            | "ico" | "svg" => Self::Image,
            // Video (FFmpeg)
            "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => Self::Video,
            // Audio (FFmpeg)
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" | "opus" => Self::Audio,
            // Text / data (serde)
            "txt" | "csv" | "json" | "xml" | "yaml" | "yml" | "toml" | "md" | "pdf" => {
                Self::TextData
            }
            // Archives (zip/tar/flate2)
            "zip" | "tar" | "tgz" | "gz" => Self::Archive,
            _ => Self::Other,
        }
    }

    /// True for extensions we know how to *extract* (MVP set).
    pub fn is_extractable_archive(ext: &str) -> bool {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "zip" | "tar" | "tgz" | "gz"
        )
    }
}

/// Classify a path, treating existing directories as Folder.
pub fn classify_path(path: &Path) -> FileKind {
    if path.is_dir() {
        return FileKind::Folder;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(FileKind::from_extension)
        .unwrap_or(FileKind::Other)
}

/// Lossy-to-lossy warning predicate (PRD §What to Watch Out For).
/// Returns true when re-encoding would degrade quality.
pub fn should_warn_lossy(input_ext: &str, output_ext: &str, kind: FileKind) -> bool {
    fn lossy_image(e: &str) -> bool {
        matches!(e, "jpg" | "jpeg" | "webp" | "heic" | "heif")
    }
    fn lossy_audio(e: &str) -> bool {
        matches!(e, "mp3" | "aac" | "ogg" | "opus" | "wma" | "m4a")
    }
    let (i, o) = (
        input_ext.to_ascii_lowercase(),
        output_ext.to_ascii_lowercase(),
    );
    match kind {
        FileKind::Image => lossy_image(&i) && lossy_image(&o),
        FileKind::Audio => lossy_audio(&i) && lossy_audio(&o),
        FileKind::Video => {
            // MVP: any video→video/gif re-encode is lossy except explicit lossless.
            i != o || o == "gif"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_kinds() {
        assert_eq!(FileKind::from_extension("png"), FileKind::Image);
        assert_eq!(FileKind::from_extension("SVG"), FileKind::Image);
        assert_eq!(FileKind::from_extension("HEIC"), FileKind::Image);
    }

    #[test]
    fn lossy_warning() {
        assert!(should_warn_lossy("jpg", "webp", FileKind::Image));
        assert!(!should_warn_lossy("png", "jpg", FileKind::Image));
        assert!(should_warn_lossy("mp3", "ogg", FileKind::Audio));
        assert!(!should_warn_lossy("wav", "mp3", FileKind::Audio));
    }
}
