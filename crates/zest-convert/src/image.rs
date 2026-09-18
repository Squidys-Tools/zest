//! Image engine: WIC primary (hardware-accelerated), `image`/`resvg` fallback.
//! HEIC: needs OS HEVC extension or a capable crate — detect + guide, never fail silently.

use super::{ConvertError, Job};
use zest_core::Settings;

/// Outputs the engine accepts (SVG is input-only).
pub const OUTPUTS: &[&str] = &[
    "png", "jpg", "bmp", "gif", "tiff", "webp", "heic", "ico", "pdf",
];

pub async fn convert(job: &Job, settings: &Settings) -> Result<std::path::PathBuf, ConvertError> {
    if !OUTPUTS.contains(&job.output_ext.as_str()) {
        return Err(ConvertError::Unsupported(
            "image".to_string(),
            job.output_ext.clone(),
        ));
    }
    if job.output_ext == "heic" {
        // TODO(MVP-images): probe HEVC extension / heic crate; guide user if missing.
        tracing::warn!("HEIC output needs HEVC support — detection lands with the engine");
    }
    let _ = settings.jpeg_quality; // wired when WIC encode lands
    Err(ConvertError::NotImplemented(
        "image engine (WIC → image/resvg fallback)".to_string(),
    ))
}
