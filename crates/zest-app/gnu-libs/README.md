# MinGW shlwapi import library (windows-gnu only)

Rust's self-contained `x86_64-pc-windows-gnu` MinGW sysroot does **not**
ship `libshlwapi.a`. The `webbrowser` crate (via `egui-winit`'s `links`
feature, pulled in by `eframe`) emits `-lshlwapi`, so
`cargo build --target x86_64-pc-windows-gnu` fails with:

```
ld: cannot find -lshlwapi: No such file or directory
```

`zest-app`'s `build.rs` adds this directory to the linker search path
when `CARGO_CFG_TARGET_OS=windows` and `CARGO_CFG_TARGET_ENV=gnu`.
MSVC builds are unaffected (Windows SDK already has `ShLwApi.Lib`).

Contents:

- `libshlwapi.a` — minimal import library exporting `AssocQueryStringW`
  (the only symbol `webbrowser` needs).
- `shlwapi.def` — the module definition used to generate the `.a`.

Regenerate with MinGW `dlltool`:

```
dlltool -d shlwapi.def -l libshlwapi.a -m i386:x86-64
```
