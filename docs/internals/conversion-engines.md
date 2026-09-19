# Conversion engines

One `dispatch(job, settings)` in `zest-convert` routes by `FileKind` to four
engine modules. Each validates its target set, then does the work as a tokio
task. Engines land one at a time: images → media → archives → text.

## Images (`convert::image`)

Primary path is WIC through `windows-rs`: hardware-accelerated, already knows
the common raster formats. `image` and `resvg` are fallbacks; SVG is input-only.

HEIC is the sharp edge: output needs either a capable crate or the OS HEVC
extension, which may not be installed. Detect it and guide the user to install
it. Never fail silently.

## Media (`convert::media`)

Bundled FFmpeg as a tokio subprocess. Install path:
`%LOCALAPPDATA%\Zest\ffmpeg\ffmpeg.exe` (MSI lays it down); dev machines fall
back to `PATH`.

Video-to-GIF is special-cased with resolution and framerate caps
(`GIF_MAX_WIDTH = 480`, `GIF_MAX_FPS = 15`). Without caps a GIF easily reaches
hundreds of megabytes.

Quality wiring: video preset (low/medium/high/lossless) maps to FFmpeg args;
audio bitrate maps to `-b:a`. JPEG-style sliders do not apply here.

## Text (`convert::text`)

Serde ecosystem: `serde_json`, `serde_yaml`, `toml`, `quick-xml`, `csv`.
Conversions stay structured → structured where meaningful.

Markdown-to-PDF is deliberately simple for MVP: clean fixed-width text with
basic pagination. No fancy layout.

## Archives (`convert::archive`)

MVP uses pure-Rust `zip` / `tar` / `flate2` for zip, tar, tar.gz, and gzip
create + extract — no native vcpkg dependency. A libarchive swap remains
possible later if format coverage demands it.

## Shared rules

- `dispatch` computes the collision-safe sibling path before delegating.
- The lossy → lossy predicate (`should_warn_lossy`) fires before work starts;
  Settings can silence it.
- Errors surface as toasts with the action needed, never silent drops.
