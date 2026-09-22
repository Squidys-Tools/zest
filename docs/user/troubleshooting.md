# Troubleshooting

## The menu does not open

- Make sure files (not a virtual folder) are selected. Recycle Bin, This PC,
  and similar shell locations have no filesystem paths; Zest declines them.
- Make sure an Explorer window or the Desktop has focus. The selection reader
  follows the focused window, including tabbed Explorer on Windows 11.
- Try the hotkey again after clicking a file. An empty selection shows nothing.

## The output never appears

- Check the toast. Errors (missing HEVC support, missing FFmpeg, unreadable
  input) report there, not in a dialog.
- For HEIC work, follow the prompt to install the HEVC extension, then retry.

## Quality got worse

- You converted lossy → lossy (JPEG → WebP, MP3 → OGG). That always discards
  detail. Convert from the lossless original when you have it, or turn the
  warning back on in Settings.

## The hotkey does nothing

- Another app may own the combination. Re-record it in
  **Settings → Hotkey**.

## Build fails with `cannot find -lshlwapi`

You are building for `x86_64-pc-windows-gnu` and Rust's bundled MinGW
sysroot is missing `libshlwapi.a`. Pull the latest changes — `zest-app`
now vendors a minimal import library under `crates/zest-app/gnu-libs/`
and wires it in via `build.rs`. MSVC builds were never affected. See
`docs/operations/development.md`.
