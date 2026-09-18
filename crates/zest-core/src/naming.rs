//! Collision-safe sibling output naming: `photo.jpg`, `photo (1).jpg`, …

use std::path::{Path, PathBuf};

/// Given the original file and desired extension, return a non-existing
/// sibling path. Never touches the original.
pub fn unique_sibling_path(original: &Path, new_ext: &str) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let parent = original.parent().unwrap_or_else(|| Path::new("."));
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
}
