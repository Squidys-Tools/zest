//! Conversion engines behind one `dispatch()` (PRD §What It Can Convert).
//!
//! - image: WIC primary (via `windows`), `image` + `resvg` fallback; SVG input.
//! - media: bundled FFmpeg subprocess; video→GIF caps resolution/framerate.
//! - text: serde parsers; md→PDF intentionally simple (fixed-width, paginated).
//! - archive: pure-Rust `zip`/`tar`/`flate2` for MVP (zip/tar/tar.gz/gzip
//!   create+extract); libarchive swap is a later option if needed.
//!
//! MVP order: images → media → archives → text. Each `convert()` currently
//! validates + stubs so the workspace builds; engines land per milestone.

pub mod archive;
pub mod image;
pub mod media;
pub mod text;

use std::path::{Path, PathBuf};
use thiserror::Error;
use zest_core::{file_kind::classify_path, FileKind, Settings};

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("unsupported conversion: {0} -> {1}")]
    Unsupported(String, String),
    #[error("ffmpeg not found at {0}")]
    FfmpegMissing(String),
    #[error("io: {0}")]
    Io(String),
    #[error("engine not yet implemented (MVP): {0}")]
    NotImplemented(String),
}

#[derive(Debug, Clone)]
pub struct Job {
    pub input: PathBuf,
    pub output_ext: String,
    /// Resolved output path (collision-safe). Filled by `dispatch()`.
    pub output: Option<PathBuf>,
}

impl Job {
    pub fn new(input: &Path, output_ext: &str) -> Self {
        Self {
            input: input.to_path_buf(),
            output_ext: output_ext.trim_start_matches('.').to_ascii_lowercase(),
            output: None,
        }
    }
}

/// Route one file to the right engine. Computes the collision-safe sibling
/// path, checks the lossy→lossy warning predicate, then delegates.
pub async fn dispatch(job: &mut Job, settings: &Settings) -> Result<PathBuf, ConvertError> {
    let kind = classify_path(&job.input);
    let input_ext = job
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if settings.warn_lossy_to_lossy
        && zest_core::file_kind::should_warn_lossy(&input_ext, &job.output_ext, kind)
    {
        tracing::warn!(
            input = %job.input.display(),
            target = %job.output_ext,
            "lossy-to-lossy re-encode will degrade quality"
        );
    }

    let out = zest_core::unique_sibling_path(&job.input, &job.output_ext);
    job.output = Some(out.clone());

    match kind {
        FileKind::Image => image::convert(job, settings).await,
        FileKind::Video | FileKind::Audio => media::convert(job, settings).await,
        FileKind::TextData => text::convert(job, settings).await,
        FileKind::Archive | FileKind::Folder => archive::convert_or_extract(job, settings).await,
        FileKind::Other => Err(ConvertError::Unsupported(input_ext, job.output_ext.clone())),
    }
}
