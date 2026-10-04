//! DirectWrite text for the ring.
//!
//! Labels and the centre badge go through `ID2D1RenderTarget::DrawText` with an
//! `IDWriteTextFormat`. Text is unaffected by the gradient-brush binding defect
//! documented in `docs/internals/overlay.md` — that is a brush, not text — so
//! this is the one piece of the ring's chrome that does not work around
//! anything.
//!
//! A format's size is fixed at creation and the ring's size depends only on how
//! many sectors it has, so formats are cached and quantised: a run of
//! submenus with differing sector counts builds a handful of formats, not one
//! per ring.

use anyhow::{Context, Result};
use windows::{
    core::{BOOL, PCWSTR},
    Win32::{
        Graphics::{
            Direct2D::{
                Common::D2D_RECT_F, ID2D1Brush, ID2D1RenderTarget, D2D1_DRAW_TEXT_OPTIONS_CLIP,
            },
            DirectWrite::{
                DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING,
                DWRITE_TRIMMING_GRANULARITY_WORD, DWRITE_WORD_WRAPPING_NO_WRAP,
            },
        },
        UI::WindowsAndMessaging::{
            SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
        },
    },
};

/// The locale asked of DirectWrite. It only picks locale-specific glyph
/// variants; the ring's labels are short and in whatever language the menu is.
const LOCALE: &str = "en-us";

/// What `SYSTEM_UI_FONT` literally is. Windows reports it from
/// `SPI_GETNONCLIENTMETRICS` as a marker meaning "whatever the shell is using",
/// and DirectWrite has no such family.
const SYSTEM_UI_FONT_SENTINEL: &str = ".SystemUIFont";

/// The first family tried when the OS does not name one. The sentinel exists
/// precisely because naming a family outright was not portable, so this is a
/// last resort rather than the plan.
const FALLBACK_FAMILY: &str = "Segoe UI";

/// Sizes are rounded to this step before a format is built, so a size that
/// differs by a fraction reuses the format instead of adding another.
const SIZE_STEP: f32 = 0.5;

fn quantise(size: f32) -> f32 {
    (size / SIZE_STEP).round() * SIZE_STEP
}

/// A NUL-terminated UTF-16 buffer, which is what the DirectWrite factory wants.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The family name to ask DirectWrite for, as UTF-16.
///
/// `SYSTEM_UI_FONT` is the sentinel `.SystemUIFont`, which is not a registered
/// font family. DirectWrite does not reject it — it silently substitutes its own
/// default — so the overlay would come up in a font nobody chose, and nothing
/// would say so. The sentinel is therefore resolved to a real family first: the
/// shell's message font when the OS names one, otherwise Segoe UI. Each
/// candidate is checked against the system font collection before use, so a
/// stripped Windows build falls through instead of failing to draw.
fn resolve_family(factory: &IDWriteFactory) -> Result<Vec<u16>> {
    let mut candidates = Vec::new();
    if let Some(named) = shell_ui_family() {
        if named != SYSTEM_UI_FONT_SENTINEL {
            candidates.push(named);
        }
    }
    candidates.push(FALLBACK_FAMILY.to_string());

    let mut collection = None;
    unsafe { factory.GetSystemFontCollection(&mut collection, false) }
        .context("read the DirectWrite system font collection")?;
    let collection = collection.context("DirectWrite returned no font collection")?;

    for candidate in candidates {
        let name = wide(&candidate);
        let mut index = 0u32;
        let mut exists = BOOL(0);
        let known =
            unsafe { collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists) }
                .is_ok()
                && exists.as_bool();
        if known {
            return Ok(name);
        }
        tracing::debug!(family = %candidate, "font family is not installed; trying the next");
    }
    // Nothing matched. Segoe UI rather than an error: a substituted font still
    // tells the user what every action is, and a missing overlay does not.
    Ok(wide(FALLBACK_FAMILY))
}

/// The face name the shell uses for message text, if the OS names one.
///
/// `SPI_GETNONCLIENTMETRICS` reports the sentinel itself on most builds, which is
/// not an answer, so the caller still has to check for it.
fn shell_ui_family() -> Option<String> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            0,
            Some((&mut metrics) as *mut NONCLIENTMETRICSW as *mut std::ffi::c_void),
            Default::default(),
        )
    }
    .ok()?;
    let face: Vec<u16> = metrics.lfMessageFont.lfFaceName.iter().copied().collect();
    if face.is_empty() {
        return None;
    }
    String::from_utf16(&face)
        .ok()
        .filter(|name| !name.is_empty())
}

/// Owns the DirectWrite factory, the resolved UI family, and one text format
/// per size in use.
pub struct TextStack {
    factory: IDWriteFactory,
    /// NUL-terminated UTF-16, because that is what `CreateTextFormat` takes.
    family: Vec<u16>,
    formats: Vec<(f32, IDWriteTextFormat)>,
}

impl TextStack {
    pub fn new() -> Result<Self> {
        let factory = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) }
            .context("create DirectWrite factory")?;
        let family = resolve_family(&factory)?;
        Ok(Self {
            factory,
            family,
            formats: Vec::new(),
        })
    }

    fn create_format(&self, size: f32) -> Result<IDWriteTextFormat> {
        let locale = wide(LOCALE);
        let format = unsafe {
            self.factory
                .CreateTextFormat(
                    PCWSTR(self.family.as_ptr()),
                    None::<&IDWriteFontCollection>,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(locale.as_ptr()),
                )
                .context("create overlay text format")?
        };
        unsafe {
            format
                .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)
                .context("center overlay label text")?;
            format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
                .context("center overlay label lines")?;
            // One line, always. A label that wrapped would spill onto a second
            // line and straight out of its own sector.
            format
                .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                .context("keep overlay labels on one line")?;
            // The last line of defence for a label wider than its sector: trim
            // with an ellipsis rather than let it bleed into the neighbour.
            let ellipsis = self
                .factory
                .CreateEllipsisTrimmingSign(&format)
                .context("create overlay trimming sign")?;
            format
                .SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                        // The binding types the platform's `WCHAR delimiter[2]` as
                        // a single u32, which is the same four bytes; zero leaves
                        // the platform default of no extra delimiter, with the
                        // ellipsis supplied by the trimming sign.
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    &ellipsis,
                )
                .context("trim long overlay labels")?;
        }
        Ok(format)
    }

    /// A single-line, centred format at `size`, built on first use.
    fn format_for(&mut self, size: f32) -> Result<&IDWriteTextFormat> {
        let size = quantise(size.max(1.0));
        if let Some(index) = self.formats.iter().position(|(cached, _)| *cached == size) {
            return Ok(&self.formats[index].1);
        }
        let format = self.create_format(size)?;
        self.formats.push((size, format));
        Ok(&self.formats.last().expect("just pushed a format").1)
    }

    /// Draw `text` centred inside `rect` at `size`, in `brush`.
    ///
    /// `rect` is the sector's own slice of the ring, so trimming and centring
    /// are already bounded by where the label is allowed to reach.
    pub fn draw_centered(
        &mut self,
        render_target: &ID2D1RenderTarget,
        text: &str,
        size: f32,
        rect: D2D_RECT_F,
        brush: &ID2D1Brush,
    ) -> Result<()> {
        let format = self.format_for(size)?;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        // `DrawText` reports nothing: a failed glyph run is a dropped frame, not
        // an error worth unwinding a repaint over.
        unsafe {
            render_target.DrawText(
                &utf16,
                format,
                &rect,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
        Ok(())
    }

    /// Drop every cached format, so a run of submenus with different sector
    /// counts cannot leave one format per past ring alive.
    pub fn clear(&mut self) {
        self.formats.clear();
    }

    /// The family the ring's labels are drawn in.
    #[cfg(test)]
    pub fn family(&self) -> String {
        let units: Vec<u16> = self
            .family
            .iter()
            .copied()
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zest_core::SYSTEM_UI_FONT;

    fn known_family(factory: &IDWriteFactory, name: &str) -> bool {
        let name = wide(name);
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false) }.expect("collection");
        let collection = collection.expect("a font collection");
        let mut index = 0u32;
        let mut exists = BOOL(0);
        unsafe { collection.FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists) }.is_ok()
            && exists.as_bool()
    }

    /// `SYSTEM_UI_FONT` is the sentinel `.SystemUIFont`, and DirectWrite has no
    /// such family. This is the whole reason `resolve_family` exists: pass the
    /// sentinel to `CreateTextFormat` and DirectWrite does not fail, it quietly
    /// substitutes its own default, so the ring would come up in a font nobody
    /// chose with nothing to say so.
    #[test]
    fn the_settings_font_sentinel_is_not_a_directwrite_family() {
        assert_eq!(SYSTEM_UI_FONT, SYSTEM_UI_FONT_SENTINEL);
        let factory = TextStack::new().expect("DirectWrite");
        assert!(
            !known_family(&factory.factory, SYSTEM_UI_FONT),
            "the sentinel resolved to a real family, so resolve_family can stop"
        );
    }

    #[test]
    fn the_resolved_family_is_one_directwrite_actually_knows() {
        let stack = TextStack::new().expect("DirectWrite");
        let family = stack.family();
        assert!(
            !family.is_empty(),
            "an empty family name would fall back silently"
        );
        assert_ne!(
            family, SYSTEM_UI_FONT,
            "the resolved family must not be the sentinel"
        );
        assert!(
            known_family(&stack.factory, &family),
            "resolved to {family:?}, which is not installed"
        );
    }

    /// A format must come out of every size the ring asks for, and a bad family
    /// must not be what makes that work.
    #[test]
    fn a_format_is_built_for_every_size_the_ring_asks_for() {
        let mut stack = TextStack::new().expect("DirectWrite");
        for size in [8.0_f32, 9.5, 10.5, 11.5, 13.0, 34.0] {
            let format = stack.format_for(size).expect("a text format");
            let _ = format;
        }
        // Quantisation means a near-identical size reuses the format rather than
        // building a second one.
        let before = stack.formats.len();
        stack.format_for(13.1).expect("a text format");
        assert_eq!(
            stack.formats.len(),
            before,
            "13.1 should quantise onto 13.0"
        );
    }
}
