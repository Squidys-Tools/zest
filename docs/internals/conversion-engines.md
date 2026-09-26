# Conversion engines

One `dispatch(job, settings)` in `zest-convert` routes by `FileKind` to four
engine modules. Each validates its target set, then does the work as a tokio
task. Engines land one at a time: images → media → archives → text.

## Images (`convert::image`)

Pure `image`, both directions. It covers every advertised output except HEIC and
PDF, and it is where `jpeg_quality` is applied.

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

SVG input needs `resvg` (SQU-39). PDF output belongs to `convert::text`
(SQU-51), and the image engine rejects it rather than half-doing it.

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
