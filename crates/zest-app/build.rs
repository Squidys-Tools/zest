fn main() {
    embed_manifest();
}

/// Embed the application manifest. Common Controls v6 is what makes gpui's
/// `TaskDialogIndirect` import resolvable, so zest.exe will not even start
/// without it. See `zest-app.manifest.xml` for what is deliberately left out.
#[cfg(windows)]
fn embed_manifest() {
    let manifest = std::path::Path::new("zest-app.manifest.xml");
    let rc_file = std::path::Path::new("zest-app.rc");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", rc_file.display());
    embed_resource::compile(rc_file, embed_resource::NONE)
        .manifest_required()
        .expect("embed the Windows application manifest");
}

#[cfg(not(windows))]
fn embed_manifest() {}
