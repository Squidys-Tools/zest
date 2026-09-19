# Development and local builds

## Requirements

- Rust 1.92 (`rust-toolchain.toml`; `rustup` picks it up automatically).
- Windows 10 1809+ / Windows 11. `windows-rs` needs no separate SDK install.
- FFmpeg is bundled at packaging time; dev machines may use a `PATH` copy.
- WiX Toolset v4+ only for MSI packaging (`wix/`).

## Commands

| Task | Command |
| --- | --- |
| Check everything | `cargo check --workspace` |
| Unit tests (pure logic) | `cargo test -p zest-core -p zest-overlay -p zest-selection` |
| Run the scaffold | `cargo run -p zest-app -- --check-selection` |
| Open settings window | `cargo run -p zest-app -- --settings` |
| Lint | `cargo clippy --workspace` |
| Format | `cargo fmt --all` |

`--check-selection` prints the resolved selection and the menu it would show,
falling back to a mock PNG when COM has nothing focused. Settings persist to
`%LOCALAPPDATA%\Zest\settings.json`.

## Contribution policy

Internal notes under `docs/internals/` record decisions and traps the source
does not explain. Most code changes do not need a docs update. Update internals
when you change a boundary, a default, or a platform workaround — not for
routine engine work.
