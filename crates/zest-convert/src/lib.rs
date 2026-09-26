//! Conversion engines behind one `dispatch()` (PRD §What It Can Convert).
//!
//! - image: `image` crate; `resvg` for SVG input (SQU-39).
//! - media: bundled FFmpeg subprocess; video→GIF caps resolution/framerate.
//! - text: serde parsers; md→PDF intentionally simple (fixed-width, paginated).
//! - archive: pure-Rust `zip`/`tar`/`flate2` for MVP (zip/tar/tar.gz/gzip
//!   create+extract); libarchive swap is a later option if needed.
//!
//! MVP order: images → media → archives → text. Media, archive, and text
//! still validate and stub; the image engine is live (SQU-40).

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
    /// HEIC is HEVC in a container, and nothing decodes HEVC yet. Never a bare
    /// failure: the message names the gap rather than blaming the machine.
    #[error("HEIC needs HEVC decoding, which Zest does not do yet (SQU-38)")]
    HevcUnsupported,
}

/// What the user picked. The operation picks the engine; the input kind only
/// picks the conversion engine within `Convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Re-encode the input into `output_ext`.
    Convert,
    /// Pack the input into an archive of format `output_ext`.
    Archive,
    /// Unpack the input archive next to itself.
    Extract,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub input: PathBuf,
    pub operation: Operation,
    /// Target extension for `Convert`/`Archive` (no dot); unused for `Extract`.
    pub output_ext: String,
    /// Resolved output path (collision-safe). Filled by `dispatch()`.
    pub output: Option<PathBuf>,
}

impl Job {
    pub fn new(input: &Path, output_ext: &str) -> Self {
        Self {
            input: input.to_path_buf(),
            operation: Operation::Convert,
            output_ext: output_ext.trim_start_matches('.').to_ascii_lowercase(),
            output: None,
        }
    }

    pub fn archive(input: &Path, output_ext: &str) -> Self {
        Self {
            operation: Operation::Archive,
            ..Self::new(input, output_ext)
        }
    }

    pub fn extract(input: &Path) -> Self {
        Self {
            input: input.to_path_buf(),
            operation: Operation::Extract,
            output_ext: String::new(),
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

    match job.operation {
        Operation::Extract => {
            if !FileKind::is_extractable_archive(&input_ext) {
                return Err(ConvertError::Unsupported(input_ext, "extract".to_string()));
            }
            return archive::extract(job, settings).await;
        }
        // Archives and folders are inputs here, whatever they contain.
        Operation::Archive => {
            return archive::convert_or_extract(job, settings).await;
        }
        Operation::Convert => {}
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The operation picks the engine; the input kind must not override it.
    #[tokio::test]
    async fn archive_picks_the_archive_engine_for_any_input() {
        let mut job = Job::archive(Path::new("photo.png"), "zip");
        let error = dispatch(&mut job, &Settings::default())
            .await
            .expect_err("archive engine is still a stub");
        assert!(
            error.to_string().contains("archive engine"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn convert_picks_the_engine_from_the_input_kind() {
        // A real file, so the image engine runs instead of failing on a missing
        // path. The routing assertion is about which engine answers.
        // `::image` because this module has a child named `image`.
        let source = std::env::temp_dir().join(format!("zest-routing-{}.png", std::process::id()));
        ::image::RgbaImage::new(4, 4)
            .save(&source)
            .expect("write png");
        let expected_bmp = source.with_extension("bmp");
        let _ = std::fs::remove_file(&expected_bmp);

        let mut job = Job::new(&source, "bmp");
        crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("png converts to bmp");
        assert_eq!(job.output.as_deref(), Some(expected_bmp.as_path()));

        // "zip" is not an image output, so the image engine rejects it rather
        // than quietly packing the file.
        let mut job = Job::new(&source, "zip");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("images do not convert to zip");
        assert!(matches!(error, ConvertError::Unsupported(_, _)), "{error}");

        let _ = std::fs::remove_file(&expected_bmp);
    }

    #[tokio::test]
    async fn extract_rejects_anything_but_an_archive() {
        let mut job = Job::extract(Path::new("photo.png"));
        let error = dispatch(&mut job, &Settings::default())
            .await
            .expect_err("photo.png is not an archive");
        assert!(matches!(error, ConvertError::Unsupported(_, _)), "{error}");

        let mut job = Job::extract(Path::new("bundle.zip"));
        let error = dispatch(&mut job, &Settings::default())
            .await
            .expect_err("extraction engine is still a stub");
        assert!(
            error.to_string().contains("archive extraction"),
            "unexpected error: {error}"
        );
    }
}
