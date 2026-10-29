//! Media engine: bundled FFmpeg subprocess via tokio.
//! Video→GIF is special-cased (cap resolution + framerate to avoid 500MB GIFs).
//!
//! Argument construction is a pure function (`ffmpeg_args`) so every preset,
//! container, and codec choice is unit-tested without an ffmpeg on disk.

use super::{ConvertError, Job, VideoCodec};
use std::path::{Path, PathBuf};
use zest_core::file_kind::classify_path;
use zest_core::{FileKind, Settings, VideoPreset};

/// Outputs the engine accepts from a video input.
pub const VIDEO_OUTPUTS: &[&str] = &["mp4", "webm", "avi", "mkv", "mov", "gif"];
/// Outputs the engine accepts from an audio input.
pub const AUDIO_OUTPUTS: &[&str] =
    &["mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus"];

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

/// What one job encodes, resolved from the normalized output extension and the
/// codec token a menu leaf can carry (`mp4-h265` → `mp4` + `Hevc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Video { container: Container, codec: VideoCodec },
    Gif,
    Audio { codec: AudioCodec },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Mp4,
    Webm,
    Avi,
    Mkv,
    Mov,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioCodec {
    Mp3,
    Pcm,
    Flac,
    Aac,
    Vorbis,
    Wma,
    Opus,
}

impl AudioCodec {
    fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Mp3 => "libmp3lame",
            Self::Pcm => "pcm_s16le",
            Self::Flac => "flac",
            Self::Aac => "aac",
            Self::Vorbis => "libvorbis",
            Self::Wma => "wmav2",
            Self::Opus => "libopus",
        }
    }

    /// Lossy codecs honour `-b:a`; PCM and FLAC have no bitrate knob.
    fn is_lossy(self) -> bool {
        !matches!(self, Self::Pcm | Self::Flac)
    }
}

/// The bundled binary first, `ffmpeg` on PATH second (dev machines). A missing
/// binary is an actionable error, never a panic.
async fn resolve_ffmpeg() -> Result<PathBuf, ConvertError> {
    let bundled = ffmpeg_path();
    if bundled.is_file() {
        return Ok(bundled);
    }
    if tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
    {
        return Ok(PathBuf::from("ffmpeg"));
    }
    Err(ConvertError::FfmpegMissing(missing_ffmpeg_message(
        &bundled,
    )))
}

fn missing_ffmpeg_message(bundled: &Path) -> String {
    format!(
        "FFmpeg is required for media conversion but was not found at {} and \
         `ffmpeg` is not on PATH. Reinstall Zest to restore the bundled copy, \
         or install FFmpeg yourself (e.g. `winget install Gyan.FFmpeg`) and \
         try again.",
        bundled.display()
    )
}

fn plan_for(ext: &str, codec: Option<VideoCodec>) -> Result<Plan, ConvertError> {
    // Only MP4 takes the codec token; on every other container it is ignored
    // because the menu cannot produce it there.
    let video = |container| {
        Plan::Video {
            container,
            codec: codec.unwrap_or(VideoCodec::H264),
        }
    };
    match ext {
        "mp4" => Ok(video(Container::Mp4)),
        "webm" => Ok(video(Container::Webm)),
        "avi" => Ok(video(Container::Avi)),
        "mkv" => Ok(video(Container::Mkv)),
        "mov" => Ok(video(Container::Mov)),
        "gif" => Ok(Plan::Gif),
        "mp3" => Ok(Plan::Audio {
            codec: AudioCodec::Mp3,
        }),
        "wav" => Ok(Plan::Audio {
            codec: AudioCodec::Pcm,
        }),
        "flac" => Ok(Plan::Audio {
            codec: AudioCodec::Flac,
        }),
        "aac" | "m4a" => Ok(Plan::Audio {
            codec: AudioCodec::Aac,
        }),
        "ogg" => Ok(Plan::Audio {
            codec: AudioCodec::Vorbis,
        }),
        "wma" => Ok(Plan::Audio {
            codec: AudioCodec::Wma,
        }),
        "opus" => Ok(Plan::Audio {
            codec: AudioCodec::Opus,
        }),
        other => Err(ConvertError::Unsupported(
            "media".to_string(),
            other.to_string(),
        )),
    }
}

/// x264 quality per preset: the familiar 23 default, tighter and looser either
/// side, and mathematically lossless at 0.
fn x264_crf(preset: VideoPreset) -> u8 {
    match preset {
        VideoPreset::Low => 30,
        VideoPreset::Medium => 23,
        VideoPreset::High => 18,
        VideoPreset::Lossless => 0,
    }
}

/// x265's CRF scale sits a few points higher than x264's for the same look.
fn x265_crf(preset: VideoPreset) -> u8 {
    match preset {
        VideoPreset::Low => 32,
        VideoPreset::Medium => 28,
        VideoPreset::High => 22,
        VideoPreset::Lossless => 0,
    }
}

fn vp9_crf(preset: VideoPreset) -> u8 {
    match preset {
        VideoPreset::Low => 36,
        VideoPreset::Medium => 32,
        VideoPreset::High => 26,
        VideoPreset::Lossless => 0,
    }
}

fn speed(preset: VideoPreset) -> &'static str {
    match preset {
        VideoPreset::Low => "fast",
        VideoPreset::Medium => "medium",
        VideoPreset::High => "slow",
        VideoPreset::Lossless => "veryslow",
    }
}

/// palettegen/paletteuse, not the naive `gif` encoder: the two-step palette
/// keeps banding down at 15fps and 480px-wide.
fn gif_filter() -> String {
    format!(
        "fps={GIF_MAX_FPS},scale='min({GIF_MAX_WIDTH},iw)':-2:flags=lanczos,\
         split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"
    )
}

fn video_args(
    container: Container,
    codec: VideoCodec,
    preset: VideoPreset,
    audio_bitrate_kbps: u32,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match (container, codec) {
        (Container::Mp4, VideoCodec::Hevc) => {
            args.extend(
                ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1"]
                    .map(String::from),
            );
            if preset == VideoPreset::Lossless {
                args.extend(
                    ["-x265-params", "lossless=1", "-preset", speed(preset)].map(String::from),
                );
            } else {
                args.extend(
                    [
                        "-crf".to_string(),
                        x265_crf(preset).to_string(),
                        "-preset".to_string(),
                        speed(preset).to_string(),
                    ],
                );
            }
        }
        (Container::Webm, _) => {
            args.extend(["-c:v", "libvpx-vp9", "-b:v", "0"].map(String::from));
            if preset == VideoPreset::Lossless {
                args.push("-lossless".to_string());
                args.push("1".to_string());
            } else {
                args.extend(["-crf".to_string(), vp9_crf(preset).to_string()]);
            }
        }
        _ => {
            // MP4/H.264 plus MKV, MOV, AVI: one codec keeps the behaviour
            // predictable; containers differ, the stream does not.
            args.extend(
                [
                    "-c:v".to_string(),
                    "libx264".to_string(),
                    "-crf".to_string(),
                    x264_crf(preset).to_string(),
                    "-preset".to_string(),
                    speed(preset).to_string(),
                    "-pix_fmt".to_string(),
                    "yuv420p".to_string(),
                ],
            );
        }
    }
    let audio = match container {
        Container::Webm => "libopus",
        // MP3-in-AVI plays on anything; AAC-in-AVI is spotty in old players.
        Container::Avi => "libmp3lame",
        _ => "aac",
    };
    args.extend(
        [
            "-c:a".to_string(),
            audio.to_string(),
            "-b:a".to_string(),
            format!("{audio_bitrate_kbps}k"),
        ],
    );
    args
}

fn audio_args(codec: AudioCodec, audio_bitrate_kbps: u32) -> Vec<String> {
    // `-vn` drops embedded cover art rather than dragging it into the output.
    let mut args = vec![
        "-vn".to_string(),
        "-c:a".to_string(),
        codec.ffmpeg_name().to_string(),
    ];
    if codec.is_lossy() {
        args.extend(["-b:a".to_string(), format!("{audio_bitrate_kbps}k")]);
    }
    args
}

/// The full argument vector for one conversion. Pure: same inputs, same args.
fn ffmpeg_args(
    input: &Path,
    output: &Path,
    plan: Plan,
    preset: VideoPreset,
    audio_bitrate_kbps: u32,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-i",
    ]
    .map(String::from)
    .into();
    args.push(input.to_string_lossy().into_owned());
    match plan {
        Plan::Gif => {
            args.extend(
                ["-filter_complex".to_string(), gif_filter()],
            );
        }
        Plan::Video { container, codec } => {
            args.extend(video_args(container, codec, preset, audio_bitrate_kbps));
        }
        Plan::Audio { codec } => {
            args.extend(audio_args(codec, audio_bitrate_kbps));
        }
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Last few hundred characters of ffmpeg stderr, for the error that lands in
/// a toast. ffmpeg narrates on stderr, so the tail carries the actual reason.
fn stderr_tail(stderr: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(stderr);
    let trimmed = text.trim();
    let mut start = trimmed.len().saturating_sub(max);
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    trimmed[start..].to_string()
}

pub async fn convert(job: &Job, settings: &Settings) -> Result<PathBuf, ConvertError> {
    let kind = classify_path(&job.input);
    let ext = job.output_ext.as_str();
    let advertised = match kind {
        FileKind::Video => VIDEO_OUTPUTS.contains(&ext),
        FileKind::Audio => AUDIO_OUTPUTS.contains(&ext),
        _ => false,
    };
    if !advertised {
        return Err(ConvertError::Unsupported(
            format!("{kind:?}"),
            ext.to_string(),
        ));
    }

    // `dispatch` resolves the collision-safe sibling before delegating.
    let output = job.output.clone().ok_or_else(|| {
        ConvertError::Io(format!(
            "no output path resolved for {}",
            job.input.display()
        ))
    })?;

    let plan = plan_for(ext, job.codec)?;
    let ffmpeg = resolve_ffmpeg().await?;
    let args = ffmpeg_args(
        &job.input,
        &output,
        plan,
        settings.video_preset,
        settings.audio_bitrate_kbps,
    );

    let result = tokio::process::Command::new(&ffmpeg)
        .args(&args)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| {
            ConvertError::Io(format!(
                "could not start ffmpeg at {}: {e}",
                ffmpeg.display()
            ))
        })?;
    if !result.status.success() {
        // ffmpeg leaves a partial file on failure; do not leave it lying next
        // to the original where it looks like a finished conversion.
        let _ = std::fs::remove_file(&output);
        let detail = stderr_tail(&result.stderr, 600);
        return Err(ConvertError::FfmpegFailed(format!(
            "{} -> {}: {}",
            job.input.display(),
            ext,
            if detail.is_empty() {
                format!("ffmpeg exited with {}", result.status)
            } else {
                detail
            }
        )));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Job;

    fn args_for(
        ext: &str,
        codec: Option<VideoCodec>,
        preset: VideoPreset,
        audio_bitrate_kbps: u32,
    ) -> Vec<String> {
        let plan = plan_for(ext, codec).expect("valid target");
        ffmpeg_args(
            Path::new("in.mp4"),
            Path::new("out.bin"),
            plan,
            preset,
            audio_bitrate_kbps,
        )
    }

    fn value_after(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|arg| arg == flag)
            .map(|i| args[i + 1].clone())
    }

    #[test]
    fn every_video_preset_maps_to_distinct_x264_quality() {
        let expected = [
            (VideoPreset::Low, "30", "fast"),
            (VideoPreset::Medium, "23", "medium"),
            (VideoPreset::High, "18", "slow"),
            (VideoPreset::Lossless, "0", "veryslow"),
        ];
        for (preset, crf, speed) in expected {
            let args = args_for("mp4", None, preset, 192);
            assert_eq!(value_after(&args, "-c:v").as_deref(), Some("libx264"));
            assert_eq!(
                value_after(&args, "-crf").as_deref(),
                Some(crf),
                "{preset:?}"
            );
            assert_eq!(
                value_after(&args, "-preset").as_deref(),
                Some(speed),
                "{preset:?}"
            );
        }
    }

    #[test]
    fn h265_token_encodes_hevc_with_a_real_lossless_mode() {
        let args = args_for("mp4", Some(VideoCodec::Hevc), VideoPreset::High, 192);
        assert_eq!(value_after(&args, "-c:v").as_deref(), Some("libx265"));
        assert_eq!(value_after(&args, "-tag:v").as_deref(), Some("hvc1"));
        assert_eq!(value_after(&args, "-crf").as_deref(), Some("22"));

        // x265's CRF 0 is not bit-exact; the codec's own lossless flag is.
        let args = args_for("mp4", Some(VideoCodec::Hevc), VideoPreset::Lossless, 192);
        assert!(value_after(&args, "-crf").is_none());
        assert_eq!(
            value_after(&args, "-x265-params").as_deref(),
            Some("lossless=1")
        );
    }

    #[test]
    fn webm_and_avi_pick_container_appropriate_codecs() {
        let args = args_for("webm", None, VideoPreset::Medium, 128);
        assert_eq!(value_after(&args, "-c:v").as_deref(), Some("libvpx-vp9"));
        assert_eq!(value_after(&args, "-crf").as_deref(), Some("32"));
        assert_eq!(value_after(&args, "-c:a").as_deref(), Some("libopus"));
        assert_eq!(value_after(&args, "-b:a").as_deref(), Some("128k"));

        let args = args_for("avi", None, VideoPreset::Medium, 192);
        assert_eq!(value_after(&args, "-c:v").as_deref(), Some("libx264"));
        assert_eq!(
            value_after(&args, "-c:a").as_deref(),
            Some("libmp3lame")
        );
    }

    #[test]
    fn gif_args_cap_size_and_use_the_palette_path() {
        let args = args_for("gif", None, VideoPreset::Medium, 192);
        let filter = value_after(&args, "-filter_complex").expect("a filter graph");
        assert!(filter.contains("fps=15"), "{filter}");
        assert!(filter.contains("min(480,iw)"), "{filter}");
        assert!(filter.contains("palettegen"), "{filter}");
        assert!(filter.contains("paletteuse"), "{filter}");
    }

    #[test]
    fn gif_args_do_not_depend_on_the_video_preset() {
        let reference = args_for("gif", None, VideoPreset::Medium, 192);
        for preset in [
            VideoPreset::Low,
            VideoPreset::High,
            VideoPreset::Lossless,
        ] {
            assert_eq!(args_for("gif", None, preset, 192), reference, "{preset:?}");
        }
    }

    #[test]
    fn every_audio_target_gets_a_codec_and_the_right_bitrate_handling() {
        // ext -> (ffmpeg codec, is the -b:a flag expected)
        let table = [
            ("mp3", "libmp3lame", true),
            ("wav", "pcm_s16le", false),
            ("flac", "flac", false),
            ("aac", "aac", true),
            ("m4a", "aac", true),
            ("ogg", "libvorbis", true),
            ("wma", "wmav2", true),
            ("opus", "libopus", true),
        ];
        assert_eq!(table.len(), AUDIO_OUTPUTS.len());
        for (ext, codec, lossy) in table {
            assert!(AUDIO_OUTPUTS.contains(&ext), "{ext} not advertised");
            let args = args_for(ext, None, VideoPreset::Medium, 96);
            assert!(args.iter().any(|arg| arg == "-vn"), "{ext} keeps cover art");
            assert_eq!(value_after(&args, "-c:a").as_deref(), Some(codec), "{ext}");
            let bitrate = value_after(&args, "-b:a");
            if lossy {
                assert_eq!(bitrate.as_deref(), Some("96k"), "{ext}");
            } else {
                assert!(bitrate.is_none(), "{ext} must not take a bitrate");
            }
        }
    }

    #[test]
    fn args_end_with_input_then_output_and_overwrite_is_on() {
        let args = args_for("mkv", None, VideoPreset::Medium, 192);
        assert_eq!(value_after(&args, "-i").as_deref(), Some("in.mp4"));
        assert_eq!(args.last().map(String::as_str), Some("out.bin"));
        assert!(args.iter().any(|arg| arg == "-y"));
        assert!(args.iter().any(|arg| arg == "-nostdin"));
    }

    #[test]
    fn unknown_targets_are_rejected_before_any_work() {
        for ext in ["xyz", "png", "txt", "mp4-h265"] {
            // `mp4-h265` never reaches here: Job::new splits it into
            // `mp4` + `Hevc` before plan lookup.
            assert!(plan_for(ext, None).is_err(), "{ext}");
        }
    }

    #[test]
    fn a_missing_ffmpeg_error_says_what_to_do() {
        let bundled = Path::new(r"C:\Users\test\AppData\Local\Zest\ffmpeg\ffmpeg.exe");
        let message = missing_ffmpeg_message(bundled);
        assert!(message.contains("ffmpeg.exe"), "{message}");
        assert!(message.contains("not on PATH"), "{message}");
        // The next action has to be named, not just the absence.
        assert!(message.contains("winget install Gyan.FFmpeg"), "{message}");
    }

    #[test]
    fn engine_targets_match_the_menu_ring() {
        // Every leaf `convert_targets` advertises must land on a real plan,
        // after `Job` normalizes codec tokens like `mp4-h265`.
        for kind in [FileKind::Video, FileKind::Audio] {
            for target in zest_core::convert_targets(kind) {
                let job = Job::new(Path::new("clip.mp4"), target);
                let allowed = match kind {
                    FileKind::Video => VIDEO_OUTPUTS.contains(&job.output_ext.as_str()),
                    FileKind::Audio => AUDIO_OUTPUTS.contains(&job.output_ext.as_str()),
                    _ => false,
                };
                assert!(allowed, "menu offers {target} but media rejects it");
                plan_for(&job.output_ext, job.codec)
                    .unwrap_or_else(|e| panic!("menu offers {target} but media rejects it: {e}"));
            }
        }
    }

    #[tokio::test]
    async fn video_input_rejects_audio_targets_and_vice_versa() {
        let mut job = Job::new(Path::new("clip.mp4"), "mp3");
        job.output = Some(PathBuf::from("clip.mp3"));
        let error = convert(&job, &Settings::default())
            .await
            .expect_err("video input must not take an audio target");
        assert!(matches!(error, ConvertError::Unsupported(_, _)), "{error}");

        let mut job = Job::new(Path::new("song.mp3"), "mp4");
        job.output = Some(PathBuf::from("song.mp4"));
        let error = convert(&job, &Settings::default())
            .await
            .expect_err("audio input must not take a video target");
        assert!(matches!(error, ConvertError::Unsupported(_, _)), "{error}");
    }

    /// Real ffmpeg, real files — the proof the arg builder is wired to a
    /// working subprocess. Skips quietly when there is no ffmpeg, the same way
    /// the overlay tests skip with no interactive desktop.
    #[tokio::test]
    async fn end_to_end_when_ffmpeg_is_present() {
        let Ok(ffmpeg) = resolve_ffmpeg().await else {
            eprintln!("skipping: no bundled ffmpeg and none on PATH");
            return;
        };

        let scratch =
            std::env::temp_dir().join(format!("zest-media-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create scratch dir");
        let clip = scratch.join("clip.mp4");
        let tone = scratch.join("tone.mp3");

        // A 1s test clip with a tone, generated entirely by lavfi.
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
                "testsrc=duration=1:size=128x96:rate=10", "-f", "lavfi", "-i",
                "sine=frequency=440:duration=1", "-c:v", "libx264", "-pix_fmt",
                "yuv420p", "-c:a", "aac", "-shortest",
            ])
            .arg(&clip)
            .status()
            .expect("spawn ffmpeg");
        assert!(status.success(), "clip generation failed");
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
                "sine=frequency=440:duration=1", "-c:a", "libmp3lame",
            ])
            .arg(&tone)
            .status()
            .expect("spawn ffmpeg");
        assert!(status.success(), "tone generation failed");

        let settings = Settings::default();
        let check = |out: &Path, magic: &[u8]| {
            let bytes = std::fs::read(out).expect("read output");
            assert!(
                bytes.starts_with(magic),
                "{} does not start with {magic:02X?}",
                out.display()
            );
            eprintln!("{}: {} bytes", out.display(), bytes.len());
        };

        let mut job = Job::new(&clip, "gif");
        let out = crate::dispatch(&mut job, &settings).await.expect("mp4 -> gif");
        check(&out, b"GIF8");

        let mut job = Job::new(&clip, "webm");
        let out = crate::dispatch(&mut job, &settings)
            .await
            .expect("mp4 -> webm");
        check(&out, &[0x1A, 0x45, 0xDF, 0xA3]);

        let mut job = Job::new(&tone, "ogg");
        let out = crate::dispatch(&mut job, &settings).await.expect("mp3 -> ogg");
        check(&out, b"OggS");

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
