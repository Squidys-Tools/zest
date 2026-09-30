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
| `x86_64-pc-windows-gnu` | Supported, needs a wrapper | Use `scripts\build-gnu.ps1`. Plain `cargo build` fails on the final link. |

#### Why the GNU target needs a wrapper

Plain `cargo build` for `x86_64-pc-windows-gnu` fails while linking `zest.exe`:

```text
ld: cannot find -lktmw32: No such file or directory
```

This is not a broken gcc and installing a more complete MinGW does not fix it.
rustup's `rust-mingw` component links against its own self-contained MinGW at
`<sysroot>\lib\rustlib\x86_64-pc-windows-gnu\lib\self-contained`, which carries
forty-odd import libraries plus `crt2.o` and `libgcc_eh.a` but not
`libktmw32.a`. The bundled `x86_64-w64-mingw32-gcc` searches only that
directory, so a MinGW installed anywhere else is never consulted. The
`windows_x86_64_gnu` crates on the search path carry one combined
`libwindows.0.5x.0.a` rather than per-DLL libraries, so they do not help
either.

Note that the GNU build also needs `--target x86_64-pc-windows-gnu` passed
explicitly. Nothing in the repo sets a default target, so a bare `cargo build`
happily builds the MSVC one and reports success, which makes it look as though
the wrapper worked when it did nothing at all.

`-lktmw32` reaches the link line because `gpui` enables the `windows` feature
`Win32_Storage_FileSystem`, and those bindings carry `#[link("ktmw32.dll")]` on
the kernel transaction manager calls. Zest never calls into KTM, so the import
library only has to exist at link time — which is why an empty stub is safe.

`scripts\build-gnu.ps1` finds a MinGW import directory holding `libktmw32.a` and
adds it with `-L native=`, falling back to minting a two-line stub under
`target/gnu-stubs`. Both paths are verified end to end. Its help covers the
switches; `-MingwLibDir` overrides the search and `-ForceStub` skips it.

Setting `RUSTFLAGS` at all makes cargo ignore `[build] rustflags` in
`.cargo/config.toml`, so the linker path cannot live in a config file for this
reason as well as the machine-specific path.

#### The toolchain has to be picked, not inherited

Two more traps sit between you and a link, and the script handles both.

`rust-toolchain.toml` pins an MSVC toolchain, and an MSVC-host toolchain cannot
build a GNU target at all — no GNU std, no bundled gcc. Left alone you get
`E0463: can't find crate for core`, once per dependency. Install the GNU one:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
```

cargo also resolves `rustc` from `PATH`, so a second, non-rustup Rust install
sitting ahead of the rustup shims wins that lookup even though
`rust-toolchain.toml` says otherwise. Mixing toolchains produces a wall of
`found crate X compiled by an incompatible version of rustc` (E0514); if you hit
that by hand, `cargo clean` clears the mixed artifacts.

Finally, the gcc has to be the toolchain's own bundled driver, at
`<sysroot>\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\x86_64-w64-mingw32-gcc.exe`.
It was built against that toolchain's `libgcc`, so pointing cargo at a
separately installed MinGW instead fails the link with
`cannot find -lgcc_eh`. That is also why installing a fuller MinGW does not fix
the original `libktmw32` error: the bundled driver only ever searches its own
`self-contained` directory.

## Commands

| Task | Command |
| --- | --- |
| Check everything | `cargo check --workspace` |
| Unit tests (pure logic) | `cargo test -p zest-core -p zest-selection` |
| Overlay Windows tests | `cargo test -p zest-overlay` |
| Build for the GNU target | `powershell -ExecutionPolicy Bypass -File scripts\build-gnu.ps1` |
| Check the selection resolver | `cargo run -p zest-app -- --check-selection` |
| Open settings window | `cargo run -p zest-app -- --settings` |
| Lint | `cargo clippy --workspace` |
| Format | `cargo fmt --all` |

The `zest-overlay` tests create real top-level windows and drive them with the
real cursor, so they need an interactive desktop. Where there is none they print
`skipping: …` and pass without asserting; the pixel checks light a sector
through a test-only window message and do not need a mouse.

`--check-selection` prints the resolved selection, both rings with the action
behind every leaf, and the `Shift+C` ring, falling back to a mock PNG when COM
has nothing focused. Settings persist to
`%LOCALAPPDATA%\Zest\settings.json`.

## Contribution policy

Internal notes under `docs/internals/` record decisions and traps the source
does not explain. Most code changes do not need a docs update. Update internals
when you change a boundary, a default, or a platform workaround — not for
routine engine work.
