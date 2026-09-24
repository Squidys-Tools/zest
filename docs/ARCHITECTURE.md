# Zest architecture

Source of truth for crate boundaries. PRD: tray radial menu for file conversion.

## Principles

- `zest-core` has zero Windows dependencies. Pure logic + unit tests:
  file-kind detection, two-ring menu filtering, collision-safe naming,
  settings schema, lossy-to-lossy warning predicate.
- Windows-only code lives in `selection` / `shell` / `overlay` behind small
  traits so logic stays testable on any host (`cargo test -p zest-core -p zest-overlay`).
- Originals are never modified or deleted. Output goes beside the original
  (default) with `name (1).ext` collision suffix, a chosen folder, or ask-each-time.
- Async everywhere in `app`/`convert` via tokio; FFmpeg runs as a subprocess
  (MVP), archives via pure-Rust `zip`/`tar`/`flate2` (libarchive remains a
  later option), images via WIC first with
  `image`/`resvg` fallback, text via serde parsers.

## Data flow (hotkey path)

```text
hotkey (shell) → selection.resolve() (COM) → core::menu_model::rings()
  → overlay.show(ring) (Direct2D, pre-created hidden window)
  → user picks leaf → convert::dispatch(job) (tokio task)
  → shell::toast done/error
```

## Crates

| crate | responsibility | key deps |
|---|---|---|
| zest-core | FileKind, Selection, rings, naming, Settings | serde, anyhow/thiserror |
| zest-selection | `resolve()` via ShellWindows COM; rejects Recycle Bin/This PC; Win11 tabs + Desktop special-cased | windows (Com, Shell), directories |
| zest-convert | `dispatch()` → image/media/text/archive modules; ffmpeg presence check; GIF caps; md→pdf simple | image, resvg, tokio(process), compress-tools, serde_*, csv, quick-xml, toml |
| zest-shell | tray icon, global hotkeys (default Shift+F; Shift+C opens Convert), toast, HKCU Run startup, GitHub Releases updater | tray-icon, global-hotkey, windows (Notifications, Registry), reqwest, semver |
| zest-overlay | sector geometry + angle hit-test, 200ms ease-out, acrylic/mica + gradient theme, center thumbnail/count | windows (Direct2D), core theme |
| zest-settings | eframe/egui form: hotkey recorder, quality, output, theme, font, 1–3 gradient colors, startup, update cadence, lossy-warning toggle | eframe, core Settings |
| zest-app | clap CLI, single-instance note, tokio main wiring | tokio, clap, tracing |

## Menu model (PRD §How It Works)

Ring 1 (categories, filtered):
- single PNG → Convert + Archive
- single .zip → Extract only
- mixed kinds → Archive only (conversion requires uniform kind)

Ring 2: Convert → formats for that kind; Archive → zip/tar/tar.gz/gzip;
Extract → destination/confirm. `Shift+C` jumps straight to the Convert ring.

## Riskiest first

`zest-selection::resolve()` is semi-documented COM (ShellWindows →
focused Explorer → selected items). Prototype before anything else; handle
tabbed Explorer, Desktop shell view, virtual-folder rejection.

## Later (not MVP)

Batch progress UI, 7z, image ops (resize/crop/rotate/strip-metadata),
in-process FFmpeg, video trim, PDF merge/split, plugins, Reactor settings UI.
