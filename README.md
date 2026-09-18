# Zest

Lightweight Windows tray utility: select files in Explorer, press a hotkey, get a
radial menu with only the conversions / archive actions that make sense.

> Scaffold status: app architecture + crate skeletons are in place. Conversion
> engines, COM selection resolver, and Direct2D overlay are stubbed with explicit
> `TODO(MVP)` markers in dependency order. See `docs/ARCHITECTURE.md`.

## Layout

```text
crates/
  zest-core/       shared types: FileKind, menu model, naming, Settings
  zest-selection/  Explorer selection via COM (riskiest — prototype first)
  zest-convert/    image / media(ffmpeg) / text(serde) / archive(zip+tar+flate2)
  zest-shell/      tray, hotkey, toast, startup, auto-update
  zest-overlay/    radial menu model + Direct2D layered-window renderer
  zest-settings/   egui settings window
  zest-app/        tokio binary wiring everything together
docs/ARCHITECTURE.md
wix/               MSI packaging placeholder (WiX v4/v5)
```

## Prerequisites

- Rust 1.92 (see `rust-toolchain.toml`; `rustup` picks it up automatically)
- Windows 10 1809+ / Windows 11 SDK (via `windows-rs`, no separate SDK install)
- FFmpeg bundled later under `assets/ffmpeg/` (stubbed for now)
- WiX Toolset v4+ only needed for `wix/` MSI packaging

## Run

```powershell
cargo check --workspace
cargo run -p zest-app -- --check-selection
cargo run -p zest-app -- --settings
```

Settings persist to `%LOCALAPPDATA%\Zest\settings.json`.

## Milestones (from PRD)

1. Prove `zest-selection` COM resolver on Win10 + Win11 (tabs, Desktop, virtual folders).
2. Tray + hotkey + transparent overlay drawing a static circle.
3. Interactive radial menu (sectors, hit-test, ring expansion, ~200ms ease-out).
4. Engines one at a time: images → media → archives → text. Settings in parallel.
5. Polish: toasts, errors, DPI, edge cases. Then MSI + auto-update.
