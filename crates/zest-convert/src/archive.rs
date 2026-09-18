//! Archive engine: pure-Rust `zip` / `tar` / `flate2` (MVP).
//! Create zip/tar/tar.gz/gzip from files+folders; extract the same set.
//! (PRD names libarchive; pure-Rust covers the MVP format set with no
//! native vcpkg dependency. A libarchive swap remains possible later.)

use super::{ConvertError, Job};
use zest_core::Settings;

pub const CREATE_TARGETS: &[&str] = &["zip", "tar", "tar.gz", "gzip"];

pub async fn convert_or_extract(
    job: &Job,
    _settings: &Settings,
) -> Result<std::path::PathBuf, ConvertError> {
    if !CREATE_TARGETS.contains(&job.output_ext.as_str()) {
        return Err(ConvertError::Unsupported(
            "archive".to_string(),
            job.output_ext.clone(),
        ));
    }
    // TODO(MVP-archives): zip/tar/flate2 create+extract per target.
    Err(ConvertError::NotImplemented(
        "archive engine (zip/tar/flate2)".to_string(),
    ))
}
