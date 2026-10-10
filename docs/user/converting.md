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
- **PDF** is a text-engine format, not an image one, and the text engine is
  still to come.

SVG is rasterized at its intrinsic dimensions and can be converted to any
supported raster output. It is input-only.

JPEG has no alpha channel, so converting a transparent PNG to JPG composites
onto white rather than leaving transparent pixels black.

## Video

In: MP4, AVI, MKV, MOV, WMV, FLV, WebM.
Out: MP4 (H.264 or H.265), WebM, AVI, MKV, MOV, animated GIF.

Handled by the bundled FFmpeg subprocess. Video-to-GIF is special-cased: Zest
caps resolution and framerate automatically so a GIF does not balloon to
hundreds of megabytes.

## Audio

In and out where applicable: MP3, WAV, FLAC, AAC, OGG, WMA, M4A, OPUS.
Handled by the bundled FFmpeg subprocess.

## Text and data

TXT, CSV, JSON, XML, YAML, TOML, and Markdown convert between each other where
the conversion is meaningful (structured formats stay structured).

Markdown-to-PDF is intentionally simple for MVP: clean fixed-width text with
basic pagination, nothing fancy.

## Lossy-to-lossy warning

Re-encoding a lossy file as lossy (MP3 → OGG, JPEG → WebP) degrades quality.
Zest shows a gentle warning before doing it. Turn it off in
**Settings → Warn before lossy-to-lossy conversion**.

## Next steps

- [Archives](./archives.md): the other half of the menu.
- [Settings](./settings.md): JPEG quality, video preset, audio bitrate.
