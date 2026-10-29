# Converting files

Pick **Convert** in ring 1 and the ring fans out with target formats for the
selected kind. Output appears next to the original unless Settings says
otherwise.

## Images

In: PNG, JPG, BMP, GIF, TIFF, WebP, ICO, SVG.
Out: PNG, JPG, BMP, GIF, TIFF, WebP, ICO.

Raster conversion runs through the `image` crate, and SVG input is rasterized
with `resvg`. Two formats the menu used to advertise are not offered yet, and
Zest would rather omit a target than offer one that always fails:

- **HEIC** is HEVC in a container and nothing decodes HEVC yet, so it is not
  offered as an input you can convert, nor as an output.
- **PDF** is a text-engine format, not an image one — select a text or
  Markdown file to produce one.

SVG is rasterized at its intrinsic dimensions and can be converted to any
supported raster output. It is input-only.

JPEG has no alpha channel, so converting a transparent PNG to JPG composites
onto white rather than leaving transparent pixels black.

## Video

In: MP4, AVI, MKV, MOV, WMV, FLV, WebM.
Out: MP4 (H.264, or H.265 via the `mp4 (h.265)` leaf), WebM, AVI, MKV, MOV,
animated GIF.

Handled by an FFmpeg subprocess. Zest looks for `ffmpeg.exe` at
`%LOCALAPPDATA%\Zest\ffmpeg\` first, then on `PATH`; if neither exists it says
so and names the fix rather than failing silently. Video-to-GIF is
special-cased: Zest caps resolution (480px wide) and framerate (15fps)
automatically so a GIF does not balloon to hundreds of megabytes.

## Audio

In and out: MP3, WAV, FLAC, AAC, OGG, WMA, M4A, OPUS.
Handled by the same FFmpeg subprocess; the Settings bitrate applies to lossy
targets (WAV and FLAC are lossless and have no bitrate knob).

## Text and data

In: TXT, CSV, JSON, XML, YAML, TOML, Markdown.
Out: TXT, CSV, JSON, XML, YAML, TOML, Markdown, PDF.

The structured formats — CSV, JSON, XML, YAML, TOML — convert between each
other through a common data model, so JSON ↔ YAML ↔ TOML round-trips
losslessly. A few pairs still fail with a clear reason rather than silently
losing data:

- **CSV** only carries a flat table: it reads as an array of records (every
  field stays a string) and writing it needs that same shape back.
- **TOML** needs a top-level object and has no `null`.
- **XML** output is wrapped in a `<root>` element; on input, attributes show
  up as `@name` fields and a text-only element collapses to its text.
- Plain **TXT** and **Markdown** cannot be turned into data — there is no
  structure to parse.

Anything → TXT or MD is a passthrough (Markdown → TXT strips the markup down
to plain text), and anything → PDF renders the file as clean fixed-width text
with basic pagination — deliberately simple for MVP. PDF itself is
output-only; Zest cannot read PDFs back.

## Lossy-to-lossy warning

Re-encoding a lossy file as lossy (MP3 → OGG, JPEG → WebP) degrades quality.
Zest shows a gentle warning before doing it. Turn it off in
**Settings → Warn before lossy-to-lossy conversion**.

## Next steps

- [Archives](./archives.md): the other half of the menu.
- [Settings](./settings.md): JPEG quality, video preset, audio bitrate.
