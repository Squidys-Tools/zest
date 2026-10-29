//! Archive engine: pure-Rust `zip` / `tar` / `flate2` (MVP).
//! Create zip/tar/tar.gz/gzip from files+folders; extract the same set.
//! (PRD names libarchive; pure-Rust covers the MVP format set with no
//! native vcpkg dependency. A libarchive swap remains possible later.)
//!
//! Layout conventions, so a create → extract round-trip is predictable:
//!
//! - A folder `data/` archives as `data.zip` / `data.tar` / `data.tar.gz`
//!   with `data/` as the top-level entry; a file keeps its full name inside.
//! - `gzip` is a stream, not a container: it compresses one file and is
//!   named `<file>.gz`, so the original extension survives a round-trip.
//! - Extraction lands beside the archive: zip/tar/tar.gz into a folder
//!   named after the archive, a bare `.gz` back to the decompressed file.
//! - Entries that would land outside the target (`..`, absolute paths,
//!   drive prefixes) are rejected before they touch the filesystem; tar
//!   links and special entries are skipped rather than followed.

use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use super::{ensure_parent_dir, ConvertError, Job};
use zest_core::{output_path_for, OutputLocation, Settings};

pub const CREATE_TARGETS: &[&str] = &["zip", "tar", "tar.gz", "gzip"];

pub async fn convert_or_extract(
    job: &Job,
    settings: &Settings,
) -> Result<std::path::PathBuf, ConvertError> {
    let ext = job.output_ext.as_str();
    if !CREATE_TARGETS.contains(&ext) {
        return Err(ConvertError::Unsupported(
            "archive".to_string(),
            job.output_ext.clone(),
        ));
    }
    if ext == "gzip" && job.input.is_dir() {
        return Err(ConvertError::Io(format!(
            "gzip compresses a single file; {} is a folder — pick zip, tar, or tar.gz instead",
            job.input.display()
        )));
    }

    let output = create_output_path(&job.input, ext, &settings.output)?;
    ensure_parent_dir(&output)?;

    let input = job.input.clone();
    let target = output.clone();
    let ext = ext.to_string();
    // Packing is IO + CPU bound; keep it off the async worker.
    tokio::task::spawn_blocking(move || create(&input, &target, &ext))
        .await
        .map_err(|e| ConvertError::Io(format!("archive worker failed: {e}")))??;
    Ok(output)
}

/// Unpack an archive into a folder beside it (SQU-49).
pub async fn extract(job: &Job, _settings: &Settings) -> Result<std::path::PathBuf, ConvertError> {
    let input = job.input.clone();
    let target = extract_target(&input)?;
    let output = target.clone();
    tokio::task::spawn_blocking(move || unpack(&input, &target))
        .await
        .map_err(|e| ConvertError::Io(format!("archive worker failed: {e}")))??;
    Ok(output)
}

/// Where a created archive lands, honouring the configured output location.
fn create_output_path(
    input: &Path,
    ext: &str,
    location: &OutputLocation,
) -> Result<PathBuf, ConvertError> {
    if ext == "gzip" {
        let stem = file_name(input)?;
        let parent = match location {
            OutputLocation::Folder(folder) => folder.clone(),
            OutputLocation::BesideOriginal | OutputLocation::AskEachTime => input
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
        };
        return Ok(unique_file(&parent, &format!("{stem}.gz")));
    }
    Ok(output_path_for(input, ext, location))
}

/// Where extracted contents land — always beside the archive: a folder for
/// zip/tar bundles, a stripped-name file for a bare `.gz`.
fn extract_target(input: &Path) -> Result<PathBuf, ConvertError> {
    let name = file_name(input)?;
    let lower = name.to_ascii_lowercase();
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    for suffix in [".tar.gz", ".tgz", ".zip", ".tar"] {
        if lower.ends_with(suffix) {
            return Ok(unique_dir(parent, &name[..name.len() - suffix.len()]));
        }
    }
    if lower.ends_with(".gz") {
        return Ok(unique_file(parent, &name[..name.len() - 3]));
    }
    Err(ConvertError::Unsupported(
        name,
        "extract (expected zip, tar, tar.gz, or gzip)".to_string(),
    ))
}

fn file_name(input: &Path) -> Result<String, ConvertError> {
    input
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| ConvertError::Io(format!("{} has no usable file name", input.display())))
}

fn io(context: &str, path: &Path, error: impl std::fmt::Display) -> ConvertError {
    ConvertError::Io(format!("{context} {}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Create

fn create(input: &Path, output: &Path, ext: &str) -> Result<(), ConvertError> {
    if !input.exists() {
        return Err(ConvertError::Io(format!("cannot read {}", input.display())));
    }
    match ext {
        "zip" => create_zip(input, output),
        "tar" => {
            let file = create_file(output)?;
            let mut writer = create_tar(input, std::io::BufWriter::new(file))?;
            writer.flush().map_err(|e| io("cannot finish", output, e))
        }
        "tar.gz" => {
            let file = create_file(output)?;
            let encoder = flate2::write::GzEncoder::new(
                std::io::BufWriter::new(file),
                flate2::Compression::default(),
            );
            // `into_inner` writes the tar end-of-archive blocks; the gzip
            // trailer needs an explicit finish of its own.
            let encoder = create_tar(input, encoder)?;
            let mut writer = encoder
                .finish()
                .map_err(|e| io("cannot finish", output, e))?;
            writer.flush().map_err(|e| io("cannot finish", output, e))
        }
        "gzip" => create_gzip(input, output),
        other => Err(ConvertError::Unsupported(
            "archive".to_string(),
            other.to_string(),
        )),
    }
}

fn create_file(output: &Path) -> Result<std::fs::File, ConvertError> {
    std::fs::File::create(output).map_err(|e| io("cannot write", output, e))
}

fn create_zip(input: &Path, output: &Path) -> Result<(), ConvertError> {
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(create_file(output)?));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let name = file_name(input)?;
    if input.is_dir() {
        zip_add_dir(&mut zip, input, &name, options)?;
    } else {
        zip_add_file(&mut zip, input, &name, options)?;
    }
    let mut writer = zip.finish().map_err(|e| io("cannot finish", output, e))?;
    writer.flush().map_err(|e| io("cannot finish", output, e))
}

fn zip_add_dir<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    name: &str,
    options: zip::write::SimpleFileOptions,
) -> Result<(), ConvertError> {
    // Trailing-slash entries are what keep empty folders alive in a zip.
    zip.add_directory(format!("{name}/"), options)
        .map_err(|e| io("cannot add", dir, e))?;
    // read_dir order is arbitrary; sorting keeps archives deterministic.
    let mut entries = std::fs::read_dir(dir)
        .map_err(|e| io("cannot read", dir, e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io("cannot read", dir, e))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let child = format!("{name}/{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type().map_err(|e| io("cannot read", &path, e))?;
        if kind.is_dir() {
            zip_add_dir(zip, &path, &child, options)?;
        } else if kind.is_file() {
            zip_add_file(zip, &path, &child, options)?;
        } else {
            // Links are skipped, not followed — a symlink loop would never
            // terminate, and a link entry extracts as a dangling pointer.
            tracing::warn!(path = %path.display(), "skipping non-file entry while archiving");
        }
    }
    Ok(())
}

fn zip_add_file<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &Path,
    name: &str,
    options: zip::write::SimpleFileOptions,
) -> Result<(), ConvertError> {
    zip.start_file(name.to_string(), options)
        .map_err(|e| io("cannot add", path, e))?;
    let mut source = std::fs::File::open(path).map_err(|e| io("cannot read", path, e))?;
    std::io::copy(&mut source, zip).map_err(|e| io("cannot add", path, e))?;
    Ok(())
}

/// Append `input` under its own name and return the finished writer so the
/// caller can flush whatever it wraps (gzip encoder, buffer).
fn create_tar<W: Write>(input: &Path, writer: W) -> Result<W, ConvertError> {
    let mut builder = tar::Builder::new(writer);
    let name = file_name(input)?;
    if input.is_dir() {
        // Recursive: the directory itself plus every entry under it,
        // including empty folders.
        builder
            .append_dir_all(&name, input)
            .map_err(|e| io("cannot add", input, e))?;
    } else {
        builder
            .append_path_with_name(input, &name)
            .map_err(|e| io("cannot add", input, e))?;
    }
    builder
        .into_inner()
        .map_err(|e| io("cannot finish", input, e))
}

fn create_gzip(input: &Path, output: &Path) -> Result<(), ConvertError> {
    let mut source = std::fs::File::open(input).map_err(|e| io("cannot read", input, e))?;
    // The header carries the original name for tools that honour it.
    let name = file_name(input)?;
    let mut encoder = flate2::GzBuilder::new()
        .filename(name)
        .write(create_file(output)?, flate2::Compression::default());
    std::io::copy(&mut source, &mut encoder).map_err(|e| io("cannot write", output, e))?;
    encoder
        .finish()
        .map_err(|e| io("cannot finish", output, e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Extract

fn unpack(input: &Path, target: &Path) -> Result<(), ConvertError> {
    let lower = file_name(input)?.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        return unpack_zip(input, target);
    }
    if lower.ends_with(".gz") && !lower.ends_with(".tar.gz") {
        return unpack_gzip(input, target);
    }
    let file = std::fs::File::open(input).map_err(|e| io("cannot read", input, e))?;
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        unpack_tar(
            tar::Archive::new(flate2::read::GzDecoder::new(file)),
            target,
            input,
        )
    } else {
        // `is_extractable_archive` in dispatch already gates the set; the
        // only container left is a bare .tar.
        unpack_tar(tar::Archive::new(file), target, input)
    }
}

fn unpack_zip(input: &Path, dir: &Path) -> Result<(), ConvertError> {
    let file = std::fs::File::open(input).map_err(|e| io("cannot read", input, e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        ConvertError::Io(format!(
            "cannot read {} as a zip archive: {e}",
            input.display()
        ))
    })?;

    // The central directory is known up front, so every name is checked
    // before a single byte lands on disk (zip-slip guard).
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| io("cannot read", input, e))?;
        if entry.enclosed_name().is_none() {
            return Err(ConvertError::Io(format!(
                "{} contains an unsafe path ({}); refusing to extract",
                input.display(),
                entry.name()
            )));
        }
    }

    std::fs::create_dir_all(dir).map_err(|e| io("cannot create", dir, e))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io("cannot read", input, e))?;
        let target = dir.join(entry.enclosed_name().expect("validated above"));
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| io("cannot create", &target, e))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io("cannot create", parent, e))?;
            }
            let mut out =
                std::fs::File::create(&target).map_err(|e| io("cannot write", &target, e))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| io("cannot write", &target, e))?;
        }
    }
    Ok(())
}

fn unpack_tar<R: Read>(
    mut archive: tar::Archive<R>,
    dir: &Path,
    input: &Path,
) -> Result<(), ConvertError> {
    std::fs::create_dir_all(dir).map_err(|e| io("cannot create", dir, e))?;
    for entry in archive.entries().map_err(|e| io("cannot read", input, e))? {
        let mut entry = entry.map_err(|e| io("cannot read", input, e))?;
        // `path()` borrows the entry; own it so the entry can be unpacked.
        let path = entry
            .path()
            .map_err(|e| io("cannot read", input, e))?
            .into_owned();
        let Some(rel) = safe_relative_path(&path) else {
            return Err(ConvertError::Io(format!(
                "{} contains an unsafe path ({}); refusing to extract",
                input.display(),
                path.display()
            )));
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        match entry.header().entry_type() {
            tar::EntryType::Regular | tar::EntryType::Directory => {
                // `unpack` creates the target's parent directories itself.
                entry
                    .unpack(dir.join(&rel))
                    .map_err(|e| io("cannot write", &dir.join(&rel), e))?;
            }
            // Links, devices, fifos: skipped rather than followed. A link's
            // *target* can point anywhere even when its path is safe.
            other => tracing::warn!(
                path = %path.display(),
                ?other,
                "skipping non-file archive entry"
            ),
        }
    }
    Ok(())
}

fn unpack_gzip(input: &Path, output: &Path) -> Result<(), ConvertError> {
    let file = std::fs::File::open(input).map_err(|e| io("cannot read", input, e))?;
    // Multi-member so concatenated .gz files decompress in full.
    let mut decoder = flate2::read::MultiGzDecoder::new(file);
    let mut out = std::fs::File::create(output).map_err(|e| io("cannot write", output, e))?;
    std::io::copy(&mut decoder, &mut out)
        .map_err(|e| ConvertError::Io(format!("cannot decompress {}: {e}", input.display())))?;
    Ok(())
}

/// Keep the `Normal` components of an archive entry path; reject anything
/// that would escape the target folder (`..`, roots, drive prefixes).
fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(safe)
}

// ---------------------------------------------------------------------------
// Collision-safe naming

/// First free `name` / `name (n)` under `parent` — for directories.
fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if !first.exists() {
        return first;
    }
    for i in 1..10_000 {
        let candidate = parent.join(format!("{name} ({i})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    timestamped(parent, name)
}

/// First free `stem.ext` / `stem (n).ext` under `parent` — for files whose
/// stem `zest_core::naming` cannot derive (gzip keeps the input's full name).
fn unique_file(parent: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = path.extension().and_then(|s| s.to_str());
    let first = parent.join(name);
    if !first.exists() {
        return first;
    }
    for i in 1..10_000 {
        let candidate = match ext {
            Some(ext) => parent.join(format!("{stem} ({i}).{ext}")),
            None => parent.join(format!("{stem} ({i})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    timestamped(parent, name)
}

fn timestamped(parent: &Path, name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    parent.join(format!("{name} ({stamp})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A scratch directory that removes itself, unique per test and per
    /// process so the parallel test runner cannot collide.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("zest-archive-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create scratch dir");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Files, a nested folder, and an empty folder — the cases a round-trip
    /// has to preserve.
    fn fixture_tree(root: &Path) {
        std::fs::create_dir_all(root).expect("create fixture root");
        std::fs::write(root.join("readme.txt"), "hello").expect("write readme");
        std::fs::write(root.join("notes.md"), "# notes").expect("write notes");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).expect("create sub");
        std::fs::write(sub.join("deep.txt"), "deep").expect("write deep");
        std::fs::create_dir_all(root.join("empty")).expect("create empty dir");
    }

    fn assert_tree_matches(extracted_root: &Path, label: &str) {
        let base = extracted_root.join(label);
        assert_eq!(
            std::fs::read_to_string(base.join("readme.txt")).expect("readme restored"),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("notes.md")).expect("notes restored"),
            "# notes"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("sub/deep.txt")).expect("deep restored"),
            "deep"
        );
        assert!(base.join("empty").is_dir(), "empty folder was lost");
    }

    #[tokio::test]
    async fn zip_round_trip_preserves_folder_structure() {
        let scratch = Scratch::new("zip-roundtrip");
        let data = scratch.join("data");
        fixture_tree(&data);

        let mut job = Job::archive(&data, "zip");
        let archive = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("folder packs to zip");
        assert_eq!(archive, scratch.join("data.zip"));

        // `data/` still exists, so the extract folder gets a collision number.
        let mut job = Job::extract(&archive);
        let extracted = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("zip extracts");
        assert_eq!(extracted, scratch.join("data (1)"));
        assert_tree_matches(&extracted, "data");

        // Extracting again lands in the next free slot.
        let mut job = Job::extract(&archive);
        let again = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("second extract");
        assert_eq!(again, scratch.join("data (2)"));
    }

    #[tokio::test]
    async fn tar_and_tar_gz_round_trip() {
        for ext in ["tar", "tar.gz"] {
            let scratch = Scratch::new(&format!("{ext}-roundtrip"));
            let data = scratch.join("data");
            fixture_tree(&data);

            let mut job = Job::archive(&data, ext);
            let archive = crate::dispatch(&mut job, &Settings::default())
                .await
                .unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!(archive, scratch.join(&format!("data.{ext}")));

            let mut job = Job::extract(&archive);
            let extracted = crate::dispatch(&mut job, &Settings::default())
                .await
                .unwrap_or_else(|e| panic!("{ext} extract: {e}"));
            assert_eq!(extracted, scratch.join("data (1)"));
            assert_tree_matches(&extracted, "data");
        }
    }

    #[tokio::test]
    async fn a_single_file_zips_and_extracts_as_itself() {
        let scratch = Scratch::new("single-file-zip");
        let source = scratch.join("photo.png");
        std::fs::write(&source, b"png-bytes").expect("write source");

        let mut job = Job::archive(&source, "zip");
        let archive = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("file packs to zip");
        assert_eq!(archive, scratch.join("photo.zip"));

        let mut job = Job::extract(&archive);
        let extracted = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("zip extracts");
        assert_eq!(
            std::fs::read(extracted.join("photo.png")).expect("file restored"),
            b"png-bytes"
        );
    }

    #[tokio::test]
    async fn gzip_compresses_one_file_and_extracts_back_to_its_name() {
        let scratch = Scratch::new("gzip-roundtrip");
        let source = scratch.join("report.txt");
        std::fs::write(&source, "quarterly").expect("write source");

        let mut job = Job::archive(&source, "gzip");
        let archive = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("file gzips");
        assert_eq!(archive, scratch.join("report.txt.gz"));
        let bytes = std::fs::read(&archive).expect("read .gz");
        assert_eq!(&bytes[..2], &[0x1F, 0x8B], "output is not gzip");

        let mut job = Job::extract(&archive);
        let extracted = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("gz decompresses");
        // The original still sits there, so the restore is numbered.
        assert_eq!(extracted, scratch.join("report (1).txt"));
        assert_eq!(
            std::fs::read_to_string(&extracted).expect("contents restored"),
            "quarterly"
        );
    }

    #[tokio::test]
    async fn gzip_says_no_to_a_folder() {
        let scratch = Scratch::new("gzip-folder");
        let data = scratch.join("data");
        fixture_tree(&data);

        let mut job = Job::archive(&data, "gzip");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("a folder cannot be gzipped");
        assert!(error.to_string().contains("single file"), "{error}");
        assert!(
            !scratch.join("data.gz").exists(),
            "output was still written"
        );
    }

    /// A zip-slip entry (`..`) must fail the whole extract before anything
    /// is written outside the target.
    #[tokio::test]
    async fn a_zip_with_a_traversal_entry_is_rejected() {
        let scratch = Scratch::new("zip-slip");
        let evil_zip = scratch.join("evil.zip");
        {
            let file = std::fs::File::create(&evil_zip).expect("create zip");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("../evil.txt", options)
                .expect("write traversal entry");
            zip.write_all(b"owned").expect("write payload");
            zip.finish().expect("finish zip");
        }

        let mut job = Job::extract(&evil_zip);
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("zip-slip must be rejected");
        assert!(error.to_string().contains("unsafe path"), "{error}");
        assert!(
            !scratch.join("evil.txt").exists(),
            "the traversal payload landed outside the target"
        );
        assert!(
            !scratch.join("evil").exists(),
            "the target folder should not have been created"
        );
    }

    /// A tar entry written byte-for-byte, because `tar::Builder` itself
    /// refuses `..` names — a hostile archive can only come from a writer
    /// with no such scruples.
    fn write_raw_tar(path: &Path, entry_name: &str, payload: &[u8]) {
        let put = |dst: &mut [u8], text: &str| {
            dst[..text.len()].copy_from_slice(text.as_bytes());
        };
        let mut header = [0u8; 512];
        put(&mut header[0..100], entry_name);
        put(&mut header[100..108], "0000644\0");
        put(&mut header[108..116], "0000000\0");
        put(&mut header[116..124], "0000000\0");
        put(&mut header[124..136], &format!("{:011o}\0", payload.len()));
        put(&mut header[136..148], "00000000000\0");
        header[156] = b'0'; // regular file
        put(&mut header[257..263], "ustar\0");
        put(&mut header[263..265], "00");
        // The checksum field reads as spaces while being computed.
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        put(&mut header[148..156], &format!("{sum:06o}\0 "));

        let mut file = std::fs::File::create(path).expect("create tar");
        file.write_all(&header).expect("write header");
        file.write_all(payload).expect("write payload");
        // Data pads to a 512 boundary; two zero blocks end the archive.
        file.write_all(&vec![0u8; 512 - (payload.len() % 512) + 1024])
            .expect("write trailer");
    }

    #[tokio::test]
    async fn a_tar_with_a_traversal_entry_is_rejected() {
        let scratch = Scratch::new("tar-slip");
        let evil_tar = scratch.join("evil.tar");
        write_raw_tar(&evil_tar, "../evil.txt", b"owned");

        let mut job = Job::extract(&evil_tar);
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("tar-slip must be rejected");
        assert!(error.to_string().contains("unsafe path"), "{error}");
        assert!(
            !scratch.join("evil.txt").exists(),
            "the traversal payload landed outside the target"
        );
    }

    #[tokio::test]
    async fn created_archives_get_collision_numbers() {
        let scratch = Scratch::new("collision");
        let source = scratch.join("note.txt");
        std::fs::write(&source, "x").expect("write source");
        let decoy = b"already here";
        std::fs::write(scratch.join("note.zip"), decoy).expect("write decoy");

        let mut job = Job::archive(&source, "zip");
        let output = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("collision-safe archive");
        assert_eq!(output, scratch.join("note (1).zip"));
        assert_eq!(
            std::fs::read(scratch.join("note.zip")).expect("decoy kept"),
            decoy,
            "the existing archive was overwritten"
        );
    }

    #[tokio::test]
    async fn archive_outputs_honour_a_configured_folder() {
        let scratch = Scratch::new("output-folder");
        let source = scratch.join("note.txt");
        std::fs::write(&source, "x").expect("write source");
        let out_dir = scratch.join("out");
        let settings = Settings {
            output: OutputLocation::Folder(out_dir.clone()),
            ..Settings::default()
        };

        let mut job = Job::archive(&source, "tar");
        let output = crate::dispatch(&mut job, &settings)
            .await
            .expect("archive into chosen folder");
        assert_eq!(output, out_dir.join("note.tar"));

        let mut job = Job::archive(&source, "gzip");
        let output = crate::dispatch(&mut job, &settings)
            .await
            .expect("gzip into chosen folder");
        assert_eq!(output, out_dir.join("note.txt.gz"));
    }
}
