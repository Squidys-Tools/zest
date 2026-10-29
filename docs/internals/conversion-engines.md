# Conversion engines

One `dispatch(job, settings)` in `zest-convert` routes by `FileKind` to four
engine modules. Each validates its target set, then does the work as a tokio
task. Engines land one at a time: images → media → archives → text.

## Images (`convert::image`)

`image` handles raster formats in both directions; `resvg` rasterizes SVG input
at its intrinsic size before the usual output encoder runs. SVG stays
input-only. The engine covers every advertised output except HEIC and PDF, and
it is where `jpeg_quality` is applied.

WIC was the planned primary path and this section used to say so. It is not
usable through `windows` 0.61, verified rather than assumed:

- `IWICBitmapFrameEncode::EndWrite` is unbound, so every
  `IWICBitmapEncoder::Commit` fails with `WINCODEC_ERR_WRONGSTATE` — no
  container encodes.
- Encoder property bags from `IWICImagingFactory::CreateEncoderPropertyBag`
  are rejected by `IWICBitmapFrameEncode::Initialize` with `E_INVALIDARG`, so
  `jpeg_quality` was unreachable through WIC.
- `IWICImagingFactory::CreateDecoderFromFilename` is generated at the wrong
  vtable slot and fails (`0x8007007F` / `0x80070006`). The stream path works.

Two consequences worth remembering:

- **HEIC is HEVC in a container, and nothing decodes HEVC yet.** The engine says
  so (`ConvertError::HevcUnsupported`) rather than blaming a missing codec on the
  machine. The likely fix is the FFmpeg subprocess in Phase 4, which decodes
  HEVC and is already a committed dependency — that would make HEIC free.
- **JPEG has no alpha.** Transparency is composited onto white, not dropped,
  because dropping it leaves transparent pixels black.

PDF output belongs to `convert::text` (SQU-51), and the image engine rejects it
rather than half-doing it.

## Media (`convert::media`)

Bundled FFmpeg as a tokio subprocess. Install path:
`%LOCALAPPDATA%\Zest\ffmpeg\ffmpeg.exe` (MSI lays it down); dev machines fall
back to `PATH`. Neither present → `FfmpegMissing` with an actionable message,
not a panic.

Argument construction is a pure function (`ffmpeg_args`), so presets, the GIF
caps, and every audio target are unit-tested without ffmpeg installed. The one
non-extension target is `mp4-h265`: a menu token `Job::new` splits into
`mp4` + `VideoCodec::Hevc` before a path is derived. Codec picks: MP4/MKV/MOV/AVI
→ libx264 (H.265 MP4 → libx265, `hvc1` tag), WebM → libvpx-vp9; audio in the
container is AAC except WebM (libopus) and AVI (libmp3lame, for old-player
compat). Presets map to per-codec CRF + speed; Lossless is real lossless —
x264 `-crf 0`, x265 `-x265-params lossless=1`, VP9 `-lossless 1`.

Video-to-GIF is special-cased with resolution and framerate caps
(`GIF_MAX_WIDTH = 480`, `GIF_MAX_FPS = 15`) via the
palettegen/paletteuse filter graph — not the naive encoder — and the width is
a ceiling (`min(480,iw)`), not an upscale. Without caps a GIF easily reaches
hundreds of megabytes.

Quality wiring: video preset (low/medium/high/lossless) maps to FFmpeg args;
audio bitrate maps to `-b:a` on lossy codecs only (PCM/FLAC take none).
JPEG-style sliders do not apply here. The lossy→lossy predicate warns on every
video re-encode — even same-container — and dispatch silences it when the
Lossless preset applies (never for GIF).

## Text (`convert::text`)

Serde ecosystem: `serde_json`, `serde_yaml`, `toml`, `quick-xml`, `csv`.
Structured formats convert through a `serde_json::Value` interchange:
`json`/`yaml`/`toml`/`xml`/`csv` in any direction, with format limits surfaced
as errors rather than silent loss — CSV needs a top-level array of flat
records, TOML a top-level object (and has no `null`), XML input unwraps the
document root (`@attr` attributes, text-only elements collapsed from `$text`)
and XML output re-wraps under `<root>`. `txt`/`md` are passthrough targets
for any text input (md→txt renders the Markdown to plain text), and txt/md
cannot become structured. `pdf` is output-only.

Markdown/text-to-PDF is deliberately simple for MVP and hand-rolled rather
than a dependency: Letter pages, 10pt Courier (a base-14 font, so nothing to
embed), word-wrap at 84 columns, 57 rows per page, uncompressed streams.

## Archives (`convert::archive`)

MVP uses pure-Rust `zip` / `tar` / `flate2` for zip, tar, tar.gz, and gzip
create + extract — no native vcpkg dependency. A libarchive swap remains
possible later if format coverage demands it.

## Shared rules

- `dispatch` computes the collision-safe sibling path before delegating.
- The lossy → lossy predicate (`should_warn_lossy`) fires before work starts;
  Settings can silence it.
- Errors surface as toasts with the action needed, never silent drops.
