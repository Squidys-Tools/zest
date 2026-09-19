# Converting files

Pick **Convert** in ring 1 and the ring fans out with target formats for the
selected kind. Output appears next to the original unless Settings says
otherwise.

## Images

In: PNG, JPG, BMP, GIF, TIFF, WebP, HEIC, ICO, SVG.
Out: any of those plus PDF.

Most conversion goes through Windows Imaging Component (WIC):
hardware-accelerated and already aware of the common formats. SVG and edge
cases fall back to Rust-native libraries.

HEIC depends on HEVC support on the machine. If it is missing, Zest tells you
what to install instead of failing silently.

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
