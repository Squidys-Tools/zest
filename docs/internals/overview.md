# Architecture

Zest keeps everything local on one Windows machine. A tokio runtime in a single
`tray → hotkey → selection → overlay → convert → toast` pipeline owns the whole
interaction. There is no server, no cloud, no account.

## Ownership boundaries

File selection belongs to `zest-selection` (Explorer COM). Menu filtering,
naming, and settings schema belong to `zest-core`, which has no Windows
dependencies by design. Rendering belongs to `zest-overlay` (Direct2D layered
window). Background work belongs to `zest-convert` (tokio tasks + FFmpeg
subprocess). Tray, hotkey, toast, startup, and update checks belong to
`zest-shell`. The settings form belongs to `zest-settings` (egui). `zest-app`
only wires them together. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the
crate map.

The [menu model](../../crates/zest-core/src/menu.rs) is the boundary between
selection and rendering: given a `Selection`, it returns ring 1 categories and
ring 2 targets. The overlay never classifies files; the converter never draws.

Settings live in one place: `%LOCALAPPDATA%\Zest\settings.json`, owned by
`zest-core`, edited by `zest-settings`, read by `zest-app`. The overlay reads
only theme-relevant fields (font, gradient, dark/light).

## The hotkey pipeline

```text
hotkey (shell) → selection.resolve() → menu model (core)
  → overlay.show(ring) → user picks leaf → convert::dispatch(job)
  → shell::toast done/error
```

The window is pre-created hidden; activation reveals it. Conversion runs as a
tokio task so the menu is already gone by the time work starts. The toast is
the only completion signal.

Originals are never modified or deleted. Every engine writes to a
collision-safe sibling path (`photo.jpg`, `photo (1).jpg`, …).

## What must stay true

- `zest-core` stays platform-independent and unit-tested.
- Windows-only code stays behind small traits in `selection` / `shell` /
  `overlay` so logic stays testable anywhere.
- External I/O (FFmpeg, filesystem, registry, network) stays out of the menu
  model and naming logic.
- A command acknowledgement (menu closed) means the job was accepted, not that
  it finished. Completion is the toast.

See the [glossary](./glossary.md), the
[selection resolver](./selection-resolver.md), [overlay](./overlay.md), and
[conversion engines](./conversion-engines.md) notes, and the
[development runbook](../operations/development.md).
