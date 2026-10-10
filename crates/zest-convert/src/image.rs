//! Image engine. Pure `image`: decode, convert, encode.
//!
//! WIC was the intended primary path, and the roadmap still says so. It is not
//! usable through `windows` 0.61, verified rather than assumed:
//!
//! - `IWICBitmapFrameEncode::EndWrite` is unbound, so every `Commit` fails with
//!   `WINCODEC_ERR_WRONGSTATE` — no container encodes, at all.
//! - Encoder property bags from `CreateEncoderPropertyBag` are rejected by
//!   `IWICBitmapFrameEncode::Initialize` with `E_INVALIDARG`, so `jpeg_quality`
//!   was unreachable through WIC.
//! - `CreateDecoderFromFilename` is generated at the wrong vtable slot and
//!   fails outright.
//!
//! `image` covers every advertised target except HEIC and PDF, and honours
//! `jpeg_quality`. SVG input is rasterized with `resvg`. HEIC needs HEVC
//! decoding, which nothing here does yet; the error says so instead of
//! pretending a codec is missing.

use super::{ConvertError, Job};
use std::path::PathBuf;
use zest_core::Settings;

/// Outputs the engine accepts (SVG is input-only).
///
/// `heic` and `pdf` are accepted-and-rejected on purpose rather than omitted:
/// the menu is built from `zest_core::convert_targets`, which does not offer
/// them, so a job for one can only come from a direct API caller. Rejecting
/// with a specific reason beats a generic "unsupported".
pub const OUTPUTS: &[&str] = &[
    "png", "jpg", "bmp", "gif", "tiff", "webp", "heic", "ico", "pdf",
];

const MAX_SVG_DIMENSION: f32 = 16_384.0;
const MAX_SVG_PIXELS: u64 = 16 * 1024 * 1024;

pub async fn convert(job: &Job, settings: &Settings) -> Result<PathBuf, ConvertError> {
    let ext = job.output_ext.as_str();
    if !OUTPUTS.contains(&ext) {
        return Err(ConvertError::Unsupported(
            "image".to_string(),
            job.output_ext.clone(),
        ));
    }

    // `dispatch` resolves the collision-safe sibling before delegating; without
    // it there is nowhere to write.
    let output = job.output.clone().ok_or_else(|| {
        ConvertError::Io(format!(
            "no output path resolved for {}",
            job.input.display()
        ))
    })?;

    if ext == "heic" {
        return Err(ConvertError::HevcUnsupported);
    }
    if ext == "pdf" {
        return Err(ConvertError::Unsupported(
            ext.to_string(),
            "pdf belongs to the text engine (SQU-51)".to_string(),
        ));
    }

    let input = job.input.clone();
    let target = output.clone();
    let ext = job.output_ext.clone();
    let quality = settings.jpeg_quality.clamp(1, 100);

    // Decoding and encoding are both CPU-bound; keep them off the async worker
    // so a large image cannot stall the event loop.
    tokio::task::spawn_blocking(move || transcode(&input, &target, &ext, quality))
        .await
        .map_err(|e| ConvertError::Io(format!("image worker failed: {e}")))??;

    Ok(output)
}

fn transcode(
    input: &std::path::Path,
    output: &std::path::Path,
    ext: &str,
    jpeg_quality: u8,
) -> Result<(), ConvertError> {
    let rgba = if input
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
    {
        rasterize_svg(input)?
    } else {
        image::open(input)
            .map_err(|e| {
                ConvertError::Io(format!(
                    "{} cannot be read as an image: {e}",
                    input.display()
                ))
            })?
            .to_rgba8()
    };
    let (width, height) = (rgba.width(), rgba.height());
    write(output, ext, rgba.as_raw(), width, height, jpeg_quality)
}

fn rasterize_svg(input: &std::path::Path) -> Result<image::RgbaImage, ConvertError> {
    let data = std::fs::read(input)
        .map_err(|e| ConvertError::Io(format!("cannot read SVG {}: {e}", input.display())))?;
    let mut options = resvg::usvg::Options {
        resources_dir: input.parent().and_then(|parent| {
            if parent.as_os_str().is_empty() {
                None
            } else {
                Some(std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()))
            }
        }),
        ..Default::default()
    };
    options.fontdb_mut().load_system_fonts();

    let tree = resvg::usvg::Tree::from_data(&data, &options)
        .map_err(|e| ConvertError::Io(format!("{} is not a valid SVG: {e}", input.display())))?;
    let size = tree.size();
    let (width, height) = (
        size.width().round().max(1.0),
        size.height().round().max(1.0),
    );
    if width > MAX_SVG_DIMENSION
        || height > MAX_SVG_DIMENSION
        || u64::from(width as u32) * u64::from(height as u32) > MAX_SVG_PIXELS
    {
        return Err(ConvertError::Io(format!(
            "{} is too large to rasterize safely ({width}×{height} pixels); reduce its dimensions and try again",
            input.display()
        )));
    }

    let (width, height) = (width as u32, height as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        ConvertError::Io(format!(
            "cannot allocate a {width}×{height} pixel image for {}",
            input.display()
        ))
    })?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );

    let mut pixels = pixmap.take();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    image::RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
        ConvertError::Io(format!(
            "could not read rendered SVG pixels from {}",
            input.display()
        ))
    })
}

fn write(
    output: &std::path::Path,
    ext: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
    jpeg_quality: u8,
) -> Result<(), ConvertError> {
    use image::codecs::{
        bmp::BmpEncoder, gif::GifEncoder, ico::IcoEncoder, jpeg::JpegEncoder, png::PngEncoder,
        tiff::TiffEncoder, webp::WebPEncoder,
    };
    use image::{ExtendedColorType, ImageEncoder};

    // Guarantee the parent here too, so this function is correct on its own
    // rather than depending on dispatch having done it. `dispatch` still does it,
    // which is what covers the other engines and fails before any work starts.
    super::ensure_parent_dir(output)?;

    let file = std::fs::File::create(output)
        .map_err(|e| ConvertError::Io(format!("cannot write {}: {e}", output.display())))?;
    let mut writer = std::io::BufWriter::new(file);

    fn encode<E: ImageEncoder>(
        encoder: E,
        output: &std::path::Path,
        buffer: &[u8],
        width: u32,
        height: u32,
        color: ExtendedColorType,
    ) -> Result<(), ConvertError> {
        encoder
            .write_image(buffer, width, height, color)
            .map_err(|e| {
                ConvertError::Io(format!("{} could not be encoded: {e}", output.display()))
            })
    }

    // JPEG has no alpha channel, so transparency is composited onto white
    // rather than dropped, which would leave transparent pixels black.
    if ext == "jpg" || ext == "jpeg" {
        return encode(
            JpegEncoder::new_with_quality(writer, jpeg_quality),
            output,
            &flatten_onto_white(rgba, width, height),
            width,
            height,
            ExtendedColorType::Rgb8,
        );
    }

    // BMP is the one encoder that borrows its writer, which is why `writer` is
    // a named local for the whole match.
    match ext {
        "png" => encode(
            PngEncoder::new(writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        "bmp" => encode(
            BmpEncoder::new(&mut writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        "gif" => encode(
            GifEncoder::new(writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        "tiff" => encode(
            TiffEncoder::new(writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        "webp" => encode(
            WebPEncoder::new_lossless(writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        "ico" => encode(
            IcoEncoder::new(writer),
            output,
            rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        other => Err(ConvertError::Unsupported(
            "image".to_string(),
            other.to_string(),
        )),
    }
}

/// Composite RGBA8 onto an opaque white background, returning RGB8.
fn flatten_onto_white(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in rgba.chunks_exact(4) {
        let alpha = pixel[3] as u32;
        for channel in pixel.iter().take(3) {
            // value * alpha + 255 * (255 - alpha), rounded.
            let blended = (*channel as u32 * alpha + 255 * (255 - alpha) + 127) / 255;
            rgb.push(blended as u8);
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Job;
    use std::path::{Path, PathBuf};
    use zest_core::OutputLocation;

    #[test]
    fn rejects_targets_outside_the_engine() {
        for ext in ["zip", "mp4", "json", "exe"] {
            assert!(!OUTPUTS.contains(&ext), "{ext} must not be an image output");
        }
    }

    #[test]
    fn svg_is_input_only() {
        assert!(!OUTPUTS.contains(&"svg"));
    }

    #[tokio::test]
    async fn svg_input_rasterizes_to_png_at_its_intrinsic_size() {
        let scratch = Scratch::new("svg-to-png");
        let source = scratch.join("art.svg");
        std::fs::write(
            &source,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="8"><rect width="12" height="8" fill="#ff0000"/></svg>"##,
        )
        .expect("write SVG source");

        let mut job = Job::new(&source, "png");
        let output = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("SVG converts to PNG");
        let decoded = decode_with_image(&output);

        assert_eq!((decoded.width(), decoded.height()), (12, 8));
        assert_eq!(decoded.get_pixel(6, 4).0, [255, 0, 0, 255]);
    }

    #[tokio::test]
    async fn svg_transparency_is_preserved_for_raster_outputs() {
        let scratch = Scratch::new("svg-transparency");
        let source = scratch.join("transparent.svg");
        std::fs::write(
            &source,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect x="0" width="1" height="1" fill="#ff0000" opacity="0.5"/></svg>"##,
        )
        .expect("write SVG source");

        let mut job = Job::new(&source, "png");
        let output = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("transparent SVG converts to PNG");
        let decoded = decode_with_image(&output);

        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 128]);
        assert_eq!(decoded.get_pixel(1, 0).0, [0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn invalid_svg_reports_the_source_path() {
        let scratch = Scratch::new("invalid-svg");
        let source = scratch.join("broken.svg");
        std::fs::write(&source, "<svg>").expect("write invalid SVG");

        let mut job = Job::new(&source, "png");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("malformed SVG must fail");

        assert!(error.to_string().contains("broken.svg"), "{error}");
        assert!(!scratch.join("broken.png").exists());
    }

    #[tokio::test]
    async fn oversized_svg_is_rejected_before_output_creation() {
        let scratch = Scratch::new("oversized-svg");
        let source = scratch.join("large.svg");
        std::fs::write(
            &source,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="5000" height="5000"/>"#,
        )
        .expect("write SVG source");

        let mut job = Job::new(&source, "png");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("oversized SVG must be rejected");

        assert!(
            error.to_string().contains("reduce its dimensions"),
            "{error}"
        );
        assert!(!scratch.join("large.png").exists());
    }

    #[test]
    fn transparent_pixels_flatten_to_white_not_black() {
        // A fully transparent black pixel must come out white behind the alpha.
        let rgba = [0, 0, 0, 0];
        assert_eq!(flatten_onto_white(&rgba, 1, 1), [255, 255, 255]);

        // Opaque black stays black.
        let rgba = [0, 0, 0, 255];
        assert_eq!(flatten_onto_white(&rgba, 1, 1), [0, 0, 0]);

        // Half-transparent mid grey lands halfway to white.
        let rgba = [128, 128, 128, 128];
        let [r, _, _] = flatten_onto_white(&rgba, 1, 1)
            .try_into()
            .expect("3 channels");
        assert!((190..=193).contains(&r), "unexpected blend: {r}");
    }

    /// A scratch directory that removes itself, unique per test and per process
    /// so the parallel test runner cannot collide.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("zest-image-{tag}-{}", std::process::id()));
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

    /// Deterministic RGBA noise. Flat or smooth images compress to nearly the
    /// same size at any quality, which would hide whether the quality knob is
    /// live.
    fn write_noise_png(path: &Path, width: u32, height: u32) {
        use image::{Rgba, RgbaImage};
        let mut img = RgbaImage::new(width, height);
        let mut state: u32 = 0x1234_5678;
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *pixel = Rgba([
                (state >> 16) as u8,
                ((state >> 8) ^ x.wrapping_mul(31)) as u8,
                (state as u8) ^ y as u8,
                0xFF,
            ]);
        }
        img.save(path).expect("write source png");
    }

    fn decode_with_image(path: &Path) -> image::RgbaImage {
        image::open(path).expect("output is readable").to_rgba8()
    }

    #[tokio::test]
    async fn png_to_jpg_keeps_dimensions_and_is_a_real_jpeg() {
        let scratch = Scratch::new("png-to-jpg");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 64, 48);

        let mut job = Job::new(&source, "jpg");
        let output = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("png converts to jpg");

        assert_eq!(output, scratch.join("noise.jpg"));
        assert!(output.exists(), "{} was not written", output.display());

        // JPEG start-of-image marker.
        let bytes = std::fs::read(&output).expect("read output");
        assert_eq!(
            &bytes[..2],
            &[0xFF, 0xD8],
            "output is not a JPEG (first bytes {:02X?})",
            &bytes[..2]
        );

        let decoded = decode_with_image(&output);
        assert_eq!((decoded.width(), decoded.height()), (64, 48));
    }

    #[tokio::test]
    async fn every_advertised_raster_output_produces_a_readable_file() {
        let scratch = Scratch::new("raster-outputs");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 40, 24);

        for ext in ["png", "jpg", "bmp", "gif", "tiff", "webp", "ico"] {
            let mut job = Job::new(&source, ext);
            let output = crate::dispatch(&mut job, &Settings::default())
                .await
                .unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(output.exists(), "{ext} produced no file");

            // ICO is a container of fixed-size images and GIF is indexed, so
            // this only asserts the file is a real image, not its dimensions.
            let decoded = decode_with_image(&output);
            assert!(
                decoded.width() > 0 && decoded.height() > 0,
                "{ext} decoded to an empty image"
            );
        }
    }

    #[tokio::test]
    async fn jpeg_quality_setting_changes_the_encoded_size() {
        let scratch = Scratch::new("jpeg-quality");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 96, 96);

        let low = Settings {
            jpeg_quality: 5,
            ..Settings::default()
        };
        let high = Settings {
            jpeg_quality: 98,
            ..Settings::default()
        };

        let mut job = Job::new(&source, "jpg");
        let low_out = crate::dispatch(&mut job, &low)
            .await
            .expect("low quality encode");
        let mut job = Job::new(&source, "jpg");
        let high_out = crate::dispatch(&mut job, &high)
            .await
            .expect("high quality encode");

        let low_len = std::fs::metadata(&low_out).expect("low out").len();
        let high_len = std::fs::metadata(&high_out).expect("high out").len();
        assert!(
            high_len > low_len,
            "quality had no effect: q5={low_len}B q98={high_len}B"
        );
    }

    #[tokio::test]
    async fn collision_numbering_keeps_the_first_output_intact() {
        let scratch = Scratch::new("collision");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 32, 32);
        // An existing target must push the new one aside, not overwrite it.
        let decoy = b"not really a jpeg";
        std::fs::write(scratch.join("noise.jpg"), decoy).expect("write decoy");

        let mut job = Job::new(&source, "jpg");
        let output = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect("collision-safe convert");

        assert_eq!(output, scratch.join("noise (1).jpg"));
        assert_eq!(
            std::fs::read(scratch.join("noise.jpg")).expect("decoy kept"),
            decoy,
            "the existing file was overwritten"
        );
    }

    #[tokio::test]
    async fn heic_says_why_instead_of_failing_silently() {
        let scratch = Scratch::new("heic");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 16, 16);

        let mut job = Job::new(&source, "heic");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("heic needs HEVC decoding");

        let message = error.to_string();
        assert!(
            message.contains("HEVC"),
            "error does not name the missing codec: {message}"
        );
        assert!(!written(&scratch.join("noise.heic")));
    }

    #[tokio::test]
    async fn pdf_is_reported_as_the_text_engine_s_job() {
        let scratch = Scratch::new("pdf");
        let source = scratch.join("noise.png");
        write_noise_png(&source, 16, 16);

        let mut job = Job::new(&source, "pdf");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("pdf is not an image output");

        assert!(matches!(error, ConvertError::Unsupported(_, _)), "{error}");
        assert!(!written(&scratch.join("noise.pdf")));
    }

    #[tokio::test]
    async fn unreadable_input_reports_the_path() {
        let scratch = Scratch::new("unreadable");
        let source = scratch.join("not-an-image.png");
        std::fs::write(&source, b"this is not a png").expect("write junk");

        let mut job = Job::new(&source, "jpg");
        let error = crate::dispatch(&mut job, &Settings::default())
            .await
            .expect_err("junk is not an image");

        assert!(
            error.to_string().contains("not-an-image.png"),
            "error does not name the input: {error}"
        );
    }

    #[tokio::test]
    async fn a_configured_output_folder_is_honoured() {
        let scratch = Scratch::new("output-folder");
        let source_dir = scratch.join("in");
        let target_dir = scratch.join("out");
        std::fs::create_dir_all(&source_dir).expect("source dir");
        std::fs::create_dir_all(&target_dir).expect("target dir");

        let source = source_dir.join("noise.png");
        write_noise_png(&source, 16, 16);

        let settings = Settings {
            output: OutputLocation::Folder(target_dir.clone()),
            ..Settings::default()
        };
        let mut job = Job::new(&source, "jpg");
        let output = crate::dispatch(&mut job, &settings)
            .await
            .expect("convert into the chosen folder");

        assert_eq!(
            output.parent(),
            Some(target_dir.as_path()),
            "output went somewhere the user did not choose"
        );
        assert!(output.exists(), "{} was not written", output.display());
        assert!(
            !source_dir.join("noise.jpg").exists(),
            "output also landed beside the original"
        );
    }

    #[tokio::test]
    async fn a_missing_output_folder_is_created_rather_than_failing() {
        // A chosen folder can be deleted or never exist. Converting into it
        // should create it, not fail with a raw filesystem error (finding
        // 01M3F3SJ7VA7 / 01M3F41WGD3F on the first attempt at this).
        let scratch = Scratch::new("missing-output-folder");
        let source_dir = scratch.join("in");
        std::fs::create_dir_all(&source_dir).expect("source dir");
        let source = source_dir.join("noise.png");
        write_noise_png(&source, 16, 16);

        // Two levels deep, so `create_dir_all` is genuinely exercised.
        let target_dir = scratch.join("does").join("not").join("exist");
        assert!(!target_dir.exists(), "precondition: folder must be missing");

        let settings = Settings {
            output: OutputLocation::Folder(target_dir.clone()),
            ..Settings::default()
        };
        let mut job = Job::new(&source, "jpg");
        let output = crate::dispatch(&mut job, &settings)
            .await
            .expect("a missing output folder must be created");

        assert!(
            target_dir.is_dir(),
            "{} was not created",
            target_dir.display()
        );
        assert!(output.exists(), "{} was not written", output.display());
        assert_eq!(output.parent(), Some(target_dir.as_path()));
    }

    #[test]
    fn write_creates_its_own_parent_rather_than_trusting_the_caller() {
        // Finding 01M3F41WGD3F: `write` must not depend on `dispatch` having
        // created the directory. Exercised directly, bypassing dispatch.
        let scratch = Scratch::new("write-creates-parent");
        let output = scratch.join("deep").join("nested").join("out.jpg");
        let rgba = [255u8, 128, 64, 255];

        write(&output, "jpg", &rgba, 1, 1, 80).expect("write into a missing folder");
        assert!(output.exists(), "{} was not written", output.display());

        let bytes = std::fs::read(&output).expect("read output");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "not a JPEG");
    }

    fn written(path: &Path) -> bool {
        path.exists()
            && std::fs::metadata(path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
    }
}
