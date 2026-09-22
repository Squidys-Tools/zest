fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "gnu" {
        return;
    }

    let lib_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("gnu-libs");
    let import_lib = lib_dir.join("libshlwapi.a");
    if !import_lib.is_file() {
        println!(
            "cargo::warning=missing {} — x86_64-pc-windows-gnu link of -lshlwapi will fail",
            import_lib.display()
        );
        return;
    }

    // webbrowser -> egui-winit links -lshlwapi; Rust's self-contained MinGW
    // sysroot ships no libshlwapi.a, so provide a vendored import lib.
    println!("cargo::rustc-link-search=native={}", lib_dir.display());
}
