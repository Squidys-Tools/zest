# Development and local builds

## Requirements

- Rust 1.92 (`rust-toolchain.toml`; `rustup` picks it up automatically).
- Windows 10 1809+ / Windows 11. `windows-rs` needs no separate SDK install.
- FFmpeg is bundled at packaging time; dev machines may use a `PATH` copy.
- WiX Toolset v4+ only for MSI packaging (`wix/`).

### Target triples

| Target | Status | Notes |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | Default / supported | `cargo build -p zest-app` just works. |
| `x86_64-pc-windows-gnu` | Supported (see below) | Rust's self-contained MinGW sysroot lacks `libshlwapi.a`; `crates/zest-app/build.rs` adds a vendored import lib (`crates/zest-app/gnu-libs/`) to the link path. You still need `x86_64-w64-mingw32-gcc` on `PATH` (or set `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`). |

## Commands

| Task | Command |
| --- | --- |
| Check everything | `cargo check --workspace` |
| Unit tests (pure logic) | `cargo test -p zest-core -p zest-overlay -p zest-selection` |
| Run the scaffold | `cargo run -p zest-app -- --check-selection` |
| Open settings window | `cargo run -p zest-app -- --settings` |
| Lint | `cargo clippy --workspace` |
| Format | `cargo fmt --all` |

`--check-selection` prints the resolved selection, both rings with the action
behind every leaf, and the `Shift+C` ring, falling back to a mock PNG when COM
has nothing focused. Settings persist to
`%LOCALAPPDATA%\Zest\settings.json`.

## Contribution policy

Internal notes under `docs/internals/` record decisions and traps the source
does not explain. Most code changes do not need a docs update. Update internals
when you change a boundary, a default, or a platform workaround — not for
routine engine work.
