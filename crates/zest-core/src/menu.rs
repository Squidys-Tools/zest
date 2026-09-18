//! Two-ring radial menu model (PRD §How It Works).
//!
//! Ring 1: categories filtered by selection.
//! Ring 2: leaf targets after picking a category (or via second hotkey for Convert).

use crate::file_kind::{classify_path, FileKind};
use crate::Selection;

/// Ring-1 categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Convert,
    Archive,
    Extract,
}

/// Ring-1 filtering rules:
/// - single archive (.zip/.tar/.tgz/.gz) → Extract only
/// - uniform non-archive kind → Convert + Archive
/// - folders or mixed kinds → Archive only (conversion needs uniform type)
/// - empty → none
pub fn categories_for_selection(sel: &Selection) -> Vec<ActionCategory> {
    if sel.is_empty() {
        return vec![];
    }
    let kinds: Vec<FileKind> = sel.files.iter().map(|p| classify_path(p)).collect();

    // Single extractable archive → Extract only.
    if kinds.len() == 1 {
        if let Some(p) = sel.files.first() {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if kinds[0] == FileKind::Archive && FileKind::is_extractable_archive(&ext) {
                return vec![ActionCategory::Extract];
            }
        }
    }

    let first = kinds[0];
    let uniform = kinds.iter().all(|k| *k == first);
    match (uniform, first) {
        // Archives selected directly are archivable, not convertible.
        (true, FileKind::Archive) => vec![ActionCategory::Extract, ActionCategory::Archive],
        (true, FileKind::Other) => vec![ActionCategory::Archive],
        (true, _) => vec![ActionCategory::Convert, ActionCategory::Archive],
        (false, _) => vec![ActionCategory::Archive],
    }
}

/// Ring-2 Convert leaves per kind (output extensions, no dot).
pub fn convert_targets(kind: FileKind) -> &'static [&'static str] {
    match kind {
        FileKind::Image => &[
            "png", "jpg", "webp", "bmp", "gif", "tiff", "ico", "heic", "pdf",
        ],
        FileKind::Video => &["mp4", "webm", "avi", "mkv", "mov", "gif"],
        FileKind::Audio => &["mp3", "wav", "flac", "aac", "ogg", "m4a", "opus"],
        FileKind::TextData => &["txt", "csv", "json", "xml", "yaml", "toml", "md", "pdf"],
        _ => &[],
    }
}

/// Ring-2 Archive leaves (PRD: zip, tar, tar.gz, gzip).
pub fn archive_targets() -> &'static [&'static str] {
    &["zip", "tar", "tar.gz", "gzip"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sel(names: &[&str]) -> Selection {
        Selection::new(names.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn png_gets_convert_and_archive() {
        assert_eq!(
            categories_for_selection(&sel(&["photo.png"])),
            vec![ActionCategory::Convert, ActionCategory::Archive]
        );
    }

    #[test]
    fn zip_gets_extract_only() {
        assert_eq!(
            categories_for_selection(&sel(&["a.zip"])),
            vec![ActionCategory::Extract]
        );
    }

    #[test]
    fn mixed_gets_archive_only() {
        assert_eq!(
            categories_for_selection(&sel(&["a.png", "b.mp3"])),
            vec![ActionCategory::Archive]
        );
    }
}
