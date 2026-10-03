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
    core::PCWSTR,
    Win32::Graphics::{
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
};

use zest_core::SYSTEM_UI_FONT;

/// The locale asked of DirectWrite. It only picks locale-specific glyph
/// variants; the ring's labels are short and in whatever language the menu is.
const LOCALE: &str = "en-us";

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

/// Owns the DirectWrite factory and one text format per size in use.
pub struct TextStack {
    factory: IDWriteFactory,
    formats: Vec<(f32, IDWriteTextFormat)>,
}

impl TextStack {
    pub fn new() -> Result<Self> {
        let factory = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) }
            .context("create DirectWrite factory")?;
        Ok(Self {
            factory,
            formats: Vec::new(),
        })
    }

    fn create_format(&self, size: f32) -> Result<IDWriteTextFormat> {
        let family = wide(SYSTEM_UI_FONT);
        let locale = wide(LOCALE);
        let format = unsafe {
            self.factory
                .CreateTextFormat(
                    PCWSTR(family.as_ptr()),
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
}
