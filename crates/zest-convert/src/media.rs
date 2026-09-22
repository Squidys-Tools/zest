//! Media engine: bundled FFmpeg subprocess via tokio.
//! Video→GIF is special-cased (cap resolution + framerate to avoid 500MB GIFs).

use super::{ConvertError, Job};
use std::path::PathBuf;
use zest_core::Settings;

/// Expected bundled location: `%LOCALAPPDATA%\Zest\ffmpeg\ffmpeg.exe` (MSI installs it).
pub fn ffmpeg_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| {
            b.data_local_dir()
                .join("Zest")
                .join("ffmpeg")
                .join("ffmpeg.exe")
        })
        .unwrap_or_else(|| PathBuf::from("ffmpeg.exe"))
}

/// Caps applied to video→GIF so output stays sane.
pub const GIF_MAX_WIDTH: u32 = 480;
pub const GIF_MAX_FPS: u32 = 15;

pub async fn convert(job: &Job, settings: &Settings) -> Result<std::path::PathBuf, ConvertError> {
    let ffmpeg = ffmpeg_path();
    if !ffmpeg.exists() {
        // Fall back to PATH for dev machines; installer guarantees the bundle.
        if tokio::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .await
            .is_err()
        {
            return Err(ConvertError::FfmpegMissing(ffmpeg.display().to_string()));
        }
    }
    let _ = (
        &job.output_ext,
        &settings.video_preset,
        settings.audio_bitrate_kbps,
    );
    // TODO(MVP-media): build args per preset; GIF path adds
    // `-vf "fps=15,scale=480:-1:flags=lanczos"`.
    Err(ConvertError::NotImplemented(
        "media engine (ffmpeg subprocess)".to_string(),
    ))
}
