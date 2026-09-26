//! Collision-safe output naming: `photo.jpg`, `photo (1).jpg`, …
//!
//! The destination comes from `Settings::output`, so a user who picked a folder
//! gets files there rather than silently beside the original.

use crate::settings::OutputLocation;
use std::path::{Path, PathBuf};

/// Given the original file and desired extension, return a non-existing sibling
/// path. Never touches the original.
pub fn unique_sibling_path(original: &Path, new_ext: &str) -> PathBuf {
    let parent = original.parent().unwrap_or_else(|| Path::new("."));
    unique_path_in(parent, file_stem(original), new_ext)
}

/// Resolve where a conversion should write, honouring the configured
/// destination. The original is never modified or moved.
///
/// `AskEachTime` has no UI yet (SQU-47), so it falls back to beside-the-original
/// rather than failing the conversion.
pub fn output_path_for(original: &Path, new_ext: &str, location: &OutputLocation) -> PathBuf {
    match location {
        OutputLocation::BesideOriginal | OutputLocation::AskEachTime => {
            unique_sibling_path(original, new_ext)
        }
        OutputLocation::Folder(folder) => unique_path_in(folder, file_stem(original), new_ext),
    }
}

fn file_stem(original: &Path) -> &str {
    original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
}

/// First free `stem.ext` in `parent`, then `stem (1).ext`, and so on.
fn unique_path_in(parent: &Path, stem: &str, new_ext: &str) -> PathBuf {
    let ext = new_ext.trim_start_matches('.');

    let first = parent.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    for i in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({i}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Practically unreachable; fall back to timestamp.
    parent.join(format!(
        "{stem} ({}).{ext}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_stem_and_ext() {
        let p = unique_sibling_path(Path::new("C:/pics/photo.png"), "jpg");
        assert_eq!(p.extension().unwrap(), "jpg");
        assert!(p.to_string_lossy().contains("photo"));
    }

    #[test]
    fn beside_the_original_is_the_default() {
        let original = Path::new("C:/pics/photo.png");
        let beside = output_path_for(original, "jpg", &OutputLocation::BesideOriginal);
        assert_eq!(beside.parent(), Some(Path::new("C:/pics")));
    }

    #[test]
    fn a_chosen_folder_keeps_the_stem() {
        let original = Path::new("C:/pics/photo.png");
        let target = output_path_for(
            original,
            "jpg",
            &OutputLocation::Folder(PathBuf::from("D:/out")),
        );
        assert_eq!(target.parent(), Some(Path::new("D:/out")));
        assert!(
            target.to_string_lossy().contains("photo"),
            "stem was lost: {target:?}"
        );
        assert_eq!(target.extension().unwrap(), "jpg");
    }

    #[test]
    fn ask_each_time_falls_back_to_beside_the_original() {
        // There is no prompt yet (SQU-47), so this must not fail the job or
        // silently send output somewhere the user did not choose.
        let original = Path::new("C:/pics/photo.png");
        let asked = output_path_for(original, "jpg", &OutputLocation::AskEachTime);
        assert_eq!(asked, unique_sibling_path(original, "jpg"));
    }

    #[test]
    fn a_dot_prefixed_extension_is_accepted() {
        let original = Path::new("C:/pics/photo.png");
        let p = unique_sibling_path(original, ".jpg");
        assert_eq!(p.extension().unwrap(), "jpg");
    }
}
