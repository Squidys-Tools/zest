# Roadmap

Everything that needs to get done, in dependency order, from the PRD. MVP is
phases 0–7. Phase 8+ is explicitly later.

## Phase 0 — Prove the selection resolver (do first)

The single riskiest piece: the COM chain that reads the Explorer selection.
Semi-documented, and Microsoft can shift Explorer internals across major
Windows updates. Nothing else matters until this works.

- [x] Enumerate `ShellWindows` → match focused Explorer HWND → read
      `SelectedItems()` → filesystem paths (`zest-selection::resolve`).
- [x] Win11 tabbed Explorer: resolve the active tab, not just the window.
- [x] Desktop as a special shell view (`Progman` / `WorkerW`).
- [x] Gracefully decline virtual folders (Recycle Bin, This PC, `::` paths).
- [x] Verify on Win10 and Win11.

Exit: hotkey in a real Explorer window reliably yields the selected paths;
`cargo run -p zest-app -- --check-selection` shows the right ring 1.

## Phase 1 — Skeleton: tray, hotkey, static overlay

- [x] Tray icon with Show / Settings / Quit (`zest-shell::tray`).
- [ ] Global hotkey listener, default `Shift+F`, plus a second hotkey that
      jumps straight to the Convert ring (`zest-shell::hotkey`).
- [x] Transparent layered window that draws a static circle
      (`zest-overlay::Overlay::precreate` + Direct2D).
- [x] Single-instance guard (named mutex) in `zest-app`.

Exit: press hotkey → circle at cursor; press Esc → gone.

## Phase 2 — Interactive radial menu

- [ ] Sectors, cursor-angle hit-testing, ring expansion/replacement.
- [ ] Ring 1 filtering (PNG → Convert+Archive; zip → Extract; mixed → Archive).
- [ ] Ring 2 fan-out per category; Convert ring via either hotkey path.
- [ ] ~200ms ease-out sector transitions; gradient active sector (1–3 colors).
- [ ] Center thumbnail / multi-file count badge; Segoe UI Variable; Lucide icons.
- [ ] Acrylic/mica blur when available; dark-mode default.
- [ ] Pre-created hidden window reveal (no build-on-open).

Exit: full menu navigable by flick; same action always at the same angle.

## Phase 3 — Image engine (quickest feedback loop)

- [ ] WIC primary encode/decode via `windows-rs` (`convert::image`).
- [ ] `image` + `resvg` fallback; SVG input.
- [ ] Outputs: PNG, JPG, BMP, GIF, TIFF, WebP, HEIC, ICO + PDF.
- [ ] JPEG quality slider (1–100) wired through.
- [ ] HEIC/HEVC detection with install guidance (never silent failure).

Exit: PNG → JPG/WebP/PDF beside the original with collision numbering.

## Phase 4 — Video / audio engine

- [ ] Bundle FFmpeg as a subprocess under `%LOCALAPPDATA%\Zest\ffmpeg\`.
- [ ] Video in (MP4, AVI, MKV, MOV, WMV, FLV, WebM) → out (MP4 H.264/H.265,
      WebM, AVI, MKV, MOV, GIF).
- [ ] Audio both directions (MP3, WAV, FLAC, AAC, OGG, WMA, M4A, OPUS).
- [ ] Video→GIF auto-caps (480px, 15fps).
- [ ] Video preset (low/medium/high/lossless) + audio bitrate dropdown wired.
- [ ] Lossy→lossy warning + Settings toggle.

Exit: MP4 → GIF/WebM and MP3 → OGG work from the menu with sane sizes.

## Phase 5 — Archives

- [ ] Create zip / tar / tar.gz / gzip from files + folders.
- [ ] Extract the same set next to the archive.
- [ ] (MVP uses pure-Rust `zip`/`tar`/`flate2`; libarchive swap only if
      coverage demands it.)

Exit: mixed selection → zip; zip → Extract restores contents.

## Phase 6 — Text engine + settings window

- [ ] Serde conversions between TXT, CSV, JSON, XML, YAML, TOML, Markdown
      where meaningful (structured stays structured).
- [ ] Markdown→PDF simple: fixed-width text, basic pagination.
- [ ] egui settings window in parallel (independent): hotkey recorder,
      quality presets, output location, theme, font dropdown, 1–3 gradient
      pickers, startup checkbox, update cadence, lossy-warning toggle.

Exit: JSON ↔ YAML ↔ TOML round-trips; settings persist and take effect.

## Phase 7 — Polish, then packaging

- [ ] Toast notifications on done/error for every job.
- [ ] Error handling that always says what to do next (HEVC, FFmpeg, bad input).
- [ ] DPI awareness; cursor-near-edge placement; batch edge cases.
- [ ] MSI via WiX (per-user, no elevation) + FFmpeg bundled.
- [ ] Auto-update: GitHub Releases check on startup + daily/weekly/never,
      prompt on next restart.
- [ ] EV code-signing + AV vendor whitelisting submissions.

Exit: signed MSI installs per-user, converts, notifies, updates.

## Later (post-MVP, in rough order)

- Batch progress UI.
- 7z support.
- Image ops: resize, crop, rotate, strip metadata.
- In-process FFmpeg (progress + cancellation).
- Video trimming.
- PDF merge / split.
- Plugin system.
- Settings migration to Windows Reactor once it matures.

## Cross-cutting watch-outs

- COM resolver fragility across Windows updates (Phase 0, re-verify per release).
- AV false positives from the hotkey hook (signing + whitelisting, Phase 7).
- HEIC/HEVC availability (detect + guide, Phases 3–4).
- Lossy→lossy degradation (warn by default, toggle in Settings).
