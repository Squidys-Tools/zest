//! The settings window's presentation layer: paint, with no state and no events.
//!
//! Every function here is a pure `(&Palette, data) -> element` mapping. Nothing
//! in this module knows that `SettingsView` exists, so a control's appearance
//! can be read, changed, and moved without dragging its state along.
//!
//! Controls that need a click return [`Stateful`] rather than `AnyElement`:
//! gpui 0.2 does not implement `InteractiveElement` for `AnyElement`, so the
//! caller has to be able to attach the handler. The wrappers in `widgets.rs`
//! attach the handler and own the state change; this module only draws.
//!
//! The geometry and colour helpers live here too, because they are the same
//! kind of thing: arithmetic that decides what gets painted.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, BorderStyle, Bounds, Div, FontWeight, Pixels, Point, SharedString, Stateful, canvas,
    div, point, px, quad, size, transparent_black,
};
use gpui::prelude::*;
use zest_core::HexColor;

use crate::theme::{Palette, tint};

/// A dropdown is taller than this and scrolls.
pub const MENU_MAX_HEIGHT: f32 = 248.0;
pub const MENU_ROW_HEIGHT: f32 = 28.0;

// ── layout ───────────────────────────────────────────────────────────────

pub fn header(palette: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Zest Settings"),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette.text_muted)
                .child("Radial file converter"),
        )
        .into_any_element()
}

/// How a notice should read, so the caller decides the meaning and this module
/// only picks the colours.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Error,
    Warning,
    Saved,
}

pub fn notice(palette: &Palette, message: &str, tone: Tone) -> AnyElement {
    let (border, background) = match tone {
        Tone::Error => (palette.danger, tint(palette.danger, 0.15)),
        Tone::Warning => (palette.warning, tint(palette.warning, 0.15)),
        Tone::Saved => (palette.border, palette.surface_raised),
    };
    div()
        .px(px(12.0))
        .py(px(10.0))
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(background)
        .text_sm()
        .child(message.to_string())
        .into_any_element()
}

/// A titled group. `children` are stacked inside it.
pub fn section(palette: &Palette, title: &str, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text_muted)
                .child(title.to_uppercase()),
        )
        .child(div().flex().flex_col().gap(px(10.0)).children(children))
        .into_any_element()
}

/// Caption above a control, the way a form reads.
pub fn field(palette: &Palette, label: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .text_sm()
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(control)
        .into_any_element()
}

pub fn hint(palette: &Palette, message: &str, danger: bool) -> AnyElement {
    div()
        .text_sm()
        .text_color(if danger { palette.danger } else { palette.text_muted })
        .child(message.to_string())
        .into_any_element()
}

// ── buttons ──────────────────────────────────────────────────────────────

/// The Save button without its handler. A disabled Save is drawn the same way
/// and simply never gets one.
pub fn save_button(palette: &Palette, disabled: bool) -> Stateful<Div> {
    div()
        .id("save")
        .px(px(24.0))
        .h(px(34.0))
        .flex()
        .items_center()
        .rounded_md()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if disabled {
            palette.text_muted
        } else {
            gpui::black()
        })
        .bg(if disabled {
            palette.field
        } else {
            palette.accent
        })
        .child("Save")
}

/// The hotkey recorder. `recording` only changes the label and colours, so the
/// caller can keep the armed flag wherever it likes.
pub fn record_button(palette: &Palette, recording: bool) -> Stateful<Div> {
    div()
        .id("record-hotkey")
        .px(px(14.0))
        .h(px(32.0))
        .flex()
        .items_center()
        .rounded_md()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if recording { gpui::black() } else { palette.text })
        .bg(if recording { palette.accent } else { palette.field })
        .border_1()
        .border_color(if recording {
            palette.accent
        } else {
            palette.border_strong
        })
        .cursor_pointer()
        .hover(|style| style.bg(palette.surface_raised))
        .child(if recording { "Listening…" } else { "Record" })
}

/// The recorded shortcut, in a field that looks like one.
pub fn hotkey_readout(palette: &Palette, recording: bool, hotkey: &str) -> Div {
    div()
        .px(px(12.0))
        .h(px(32.0))
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(if recording {
            palette.accent
        } else {
            palette.border
        })
        .bg(palette.field)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(if recording {
            "press a shortcut".to_string()
        } else {
            hotkey.to_string()
        })
}

// ── checkbox ─────────────────────────────────────────────────────────────

pub fn checkbox(palette: &Palette, label: &str, value: bool) -> Stateful<Div> {
    let box_style = div()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(if value { palette.accent } else { palette.border_strong })
        .bg(if value { palette.accent } else { palette.field });
    let box_style = if value {
        box_style.child(div().text_xs().text_color(gpui::black()).child("✓"))
    } else {
        box_style
    };
    div()
        .id(SharedString::from(format!("checkbox-{label}")))
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(4.0))
        .cursor_pointer()
        .child(box_style)
        .child(div().text_sm().child(label.to_string()))
}

// ── slider ───────────────────────────────────────────────────────────────

/// A slider: its draggable track and the scale beneath it, as two parts.
///
/// The track records its own painted bounds so a drag can be resolved against
/// the track rather than the window, and it carries no listeners — the caller
/// attaches them to the returned element.
pub fn slider(
    palette: &Palette,
    id: &str,
    fraction: f32,
    readout: String,
    area: Rc<RefCell<Option<Bounds<Pixels>>>>,
) -> (Stateful<Div>, Div) {
    let track = slider_track(palette, id, fraction, area);
    let scale = slider_scale(palette, readout);
    (track, scale)
}

fn slider_track(
    palette: &Palette,
    id: &str,
    fraction: f32,
    area: Rc<RefCell<Option<Bounds<Pixels>>>>,
) -> Stateful<Div> {
    let fraction = fraction.clamp(0.0, 1.0);

    let paint_area = area.clone();
    let palette = palette.clone();
    let canvas_element = canvas(
        move |bounds, _, _| {
            *paint_area.borrow_mut() = Some(bounds);
        },
        move |bounds, _, window, _| {
            let center = bounds.center().y;
            let rail = Bounds {
                origin: point(bounds.origin.x, center - px(3.0)),
                size: size(bounds.size.width, px(6.0)),
            };
            window.paint_quad(quad(
                rail,
                px(3.0),
                palette.track,
                px(0.0),
                transparent_black(),
                BorderStyle::Solid,
            ));
            let filled = Bounds {
                size: size(bounds.size.width * fraction, px(6.0)),
                ..rail
            };
            window.paint_quad(quad(
                filled,
                px(3.0),
                palette.accent,
                px(0.0),
                transparent_black(),
                BorderStyle::Solid,
            ));
            let knob = Bounds {
                origin: point(
                    bounds.origin.x + bounds.size.width * fraction - px(8.0),
                    center - px(8.0),
                ),
                size: size(px(16.0), px(16.0)),
            };
            window.paint_quad(quad(
                knob,
                px(8.0),
                palette.accent,
                px(2.0),
                palette.window,
                BorderStyle::Solid,
            ));
        },
    )
    .size_full();

    div()
        .id(SharedString::from(format!("{id}-track")))
        .h(px(24.0))
        .w_full()
        .cursor_pointer()
        .child(canvas_element)
}

/// The `1 … readout … 100` scale under a slider.
fn slider_scale(palette: &Palette, readout: String) -> Div {
    div()
        .flex()
        .justify_between()
        .text_xs()
        .text_color(palette.text_muted)
        .child("1")
        .child(readout)
        .child("100")
}

// ── stepper ──────────────────────────────────────────────────────────────

/// A `− value +` control, returned as its three parts so each button can carry
/// its own handler. `unit` is appended to the value, e.g. `"kbps"`.
///
/// `id` names the control and seeds all three element ids, the way `index` does
/// for [`option_row`]. Fixed ids would collide the moment a second stepper
/// shared this window, and gpui tracks hover by element id.
pub struct Stepper {
    pub decrease: Stateful<Div>,
    pub value: Stateful<Div>,
    pub increase: Stateful<Div>,
}

pub fn stepper(palette: &Palette, id: &str, value: u32, unit: &str) -> Stepper {
    Stepper {
        decrease: div()
            .id(SharedString::from(format!("{id}-down")))
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(palette.border)
            .bg(palette.field)
            .child("−"),
        value: div()
            .id(SharedString::from(format!("{id}-value")))
            .min_w(px(92.0))
            .h(px(30.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(palette.border_strong)
            .bg(palette.field)
            .text_sm()
            .child(format!("{value} {unit}")),
        increase: div()
            .id(SharedString::from(format!("{id}-up")))
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(palette.border)
            .bg(palette.field)
            .child("+"),
    }
}

// ── menus ────────────────────────────────────────────────────────────────

/// The closed/open trigger every menu shares. `label` is the current value's
/// text, already chosen by the caller.
pub fn menu_trigger(
    palette: &Palette,
    id: &str,
    label: String,
    open: bool,
    missing: bool,
) -> Stateful<Div> {
    let border = if open {
        palette.accent
    } else if missing {
        palette.danger
    } else {
        palette.border_strong
    };
    div()
        .id(SharedString::from(format!("{id}-trigger")))
        .h(px(32.0))
        .w_full()
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(palette.field)
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(palette.surface_raised))
        .child(label)
        .child(
            div()
                .text_color(palette.text_muted)
                .child(if open { "▲" } else { "▼" }),
        )
}

/// The scrollable panel under a trigger.
pub fn menu_list(palette: &Palette, id: &str, rows: Vec<AnyElement>) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("{id}-list")))
        .flex()
        .flex_col()
        .mt(px(4.0))
        .max_h(px(MENU_MAX_HEIGHT))
        .overflow_y_scroll()
        .rounded_md()
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.surface_raised)
        .children(rows)
}

/// One selectable row. `index` is part of the element id, which is what keeps
/// gpui from collapsing the rows into one hover target.
pub fn option_row(
    palette: &Palette,
    id: &str,
    index: usize,
    label: String,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id((SharedString::from(format!("{id}-option")), index))
        .h(px(MENU_ROW_HEIGHT))
        .px(px(12.0))
        .flex()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .bg(if selected {
            tint(palette.accent, 0.3)
        } else {
            palette.surface_raised
        })
        .hover(|style| style.bg(palette.surface))
        .child(label)
}

/// A trigger stacked over a list that carries its own top margin.
pub fn dropdown_body(trigger: AnyElement, list: AnyElement) -> Div {
    div().flex().flex_col().w_full().child(trigger).child(list)
}

/// The font control's variant: its list is a vector with the gap above the
/// whole panel, so a heading row and the scrolling list can differ.
pub fn font_body(trigger: AnyElement, list: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(trigger)
        .child(div().flex().flex_col().children(list).mt(px(4.0)))
}

// ── font ─────────────────────────────────────────────────────────────────

pub fn font_missing_note(palette: &Palette, family: &str) -> AnyElement {
    div()
        .px(px(12.0))
        .py(px(6.0))
        .text_xs()
        .text_color(palette.danger)
        .child(format!(
            "{family} is not installed on this PC. Pick a family below."
        ))
        .into_any_element()
}

/// The `System UI font` entry that heads the family list.
pub fn font_system_row(palette: &Palette) -> Stateful<Div> {
    div()
        .id("font-system")
        .h(px(MENU_ROW_HEIGHT))
        .px(px(12.0))
        .flex()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .bg(tint(palette.accent, 0.3))
        .hover(|style| style.bg(palette.surface))
        .child("System UI font")
}

/// One installed family, shown in its own typeface. `index` is part of the id
/// for the same reason it is on [`option_row`].
pub fn font_row(
    palette: &Palette,
    index: usize,
    name: SharedString,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(("font-row", index))
        .h(px(MENU_ROW_HEIGHT))
        .px(px(12.0))
        .flex()
        .items_center()
        .text_sm()
        .font_family(name.clone())
        .cursor_pointer()
        .bg(if selected {
            tint(palette.accent, 0.3)
        } else {
            palette.surface_raised
        })
        .hover(|style| style.bg(palette.surface))
        .child(name)
}

/// The scrollable family list. A plain column rather than a virtualized one:
/// the panel is only on screen while the user is picking a font, and every row
/// needs its own click listener.
pub fn font_list(palette: &Palette, rows: Vec<AnyElement>) -> Stateful<Div> {
    div()
        .id("font-list")
        .mt(px(4.0))
        .h(px(MENU_MAX_HEIGHT))
        .overflow_y_scroll()
        .rounded_md()
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.surface_raised)
        .children(rows)
}

// ── colour chips ─────────────────────────────────────────────────────────

/// gpui backgrounds hold exactly two stops, so a 3-color ramp is painted as
/// adjacent segments. The overlay draws all stops in one D2D brush; the seam
/// between segments is the only difference.
pub fn gradient_bar(colors: &[HexColor], height: f32) -> AnyElement {
    div()
        .flex()
        .w_full()
        .h(px(height))
        .rounded_sm()
        .overflow_hidden()
        .children(gradient_segments(colors).into_iter().map(|background| {
            div().flex_1().h_full().bg(background)
        }))
        .into_any_element()
}

pub fn swatch(palette: &Palette, color: HexColor, selected: bool) -> AnyElement {
    div()
        .size(px(20.0))
        .rounded_sm()
        .border_1()
        .border_color(if selected { palette.accent } else { palette.window })
        .bg(to_color(color))
        .into_any_element()
}

// ── geometry ─────────────────────────────────────────────────────────────

pub type TrackArea = Rc<RefCell<Option<Bounds<Pixels>>>>;

/// Where the pointer sits along a recorded track, 0..=1.
pub fn fraction_at(area: &TrackArea, at: Point<Pixels>) -> Option<f32> {
    let bounds = (*area.borrow())?;
    fraction_in(bounds, at, Axis::Horizontal)
}

pub fn fraction_in(bounds: Bounds<Pixels>, at: Point<Pixels>, axis: Axis) -> Option<f32> {
    let extent = match axis {
        Axis::Horizontal => bounds.size.width,
        Axis::Vertical => bounds.size.height,
    };
    if extent <= px(0.0) {
        return None;
    }
    let delta = match axis {
        Axis::Horizontal => at.x - bounds.origin.x,
        Axis::Vertical => at.y - bounds.origin.y,
    };
    Some((delta / extent).clamp(0.0, 1.0))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

// ── colour ───────────────────────────────────────────────────────────────

/// Split a 1–3 color ramp into the two-stop gradients gpui can paint.
pub fn gradient_segments(colors: &[HexColor]) -> Vec<gpui::Background> {
    if colors.is_empty() {
        return Vec::new();
    }
    if colors.len() == 1 {
        return vec![gpui::solid_background(to_color(colors[0]))];
    }
    (0..colors.len() - 1)
        .map(|i| {
            gpui::linear_gradient(
                90.0,
                gpui::linear_color_stop(to_color(colors[i]), 0.0),
                gpui::linear_color_stop(to_color(colors[i + 1]), 1.0),
            )
        })
        .collect()
}

pub fn to_color(color: HexColor) -> gpui::Rgba {
    let (r, g, b) = color.to_unit_rgb();
    gpui::Rgba { r, g, b, a: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    #[test]
    fn fraction_measures_from_the_origin_and_clamps() {
        let area = bounds(10.0, 0.0, 100.0, 20.0);
        assert_eq!(
            fraction_in(area, point(px(60.0), px(0.0)), Axis::Horizontal),
            Some(0.5)
        );
        // Past the right edge clamps to the far end rather than overshooting.
        assert_eq!(
            fraction_in(area, point(px(400.0), px(0.0)), Axis::Horizontal),
            Some(1.0)
        );
        // Before the left edge clamps to zero.
        assert_eq!(
            fraction_in(area, point(px(0.0), px(0.0)), Axis::Horizontal),
            Some(0.0)
        );
    }

    #[test]
    fn fraction_uses_the_named_axis() {
        let area = bounds(0.0, 5.0, 100.0, 40.0);
        assert_eq!(
            fraction_in(area, point(px(0.0), px(25.0)), Axis::Vertical),
            Some(0.5)
        );
    }

    #[test]
    fn a_degenerate_track_has_no_fraction() {
        let flat = bounds(0.0, 0.0, 0.0, 10.0);
        assert_eq!(fraction_in(flat, point(px(0.0), px(0.0)), Axis::Horizontal), None);
    }

    #[test]
    fn fraction_at_reads_the_recorded_track() {
        let area: TrackArea = Rc::new(RefCell::new(Some(bounds(0.0, 0.0, 50.0, 10.0))));
        assert_eq!(fraction_at(&area, point(px(25.0), px(0.0))), Some(0.5));
        // Nothing painted yet means nothing to measure against.
        *area.borrow_mut() = None;
        assert_eq!(fraction_at(&area, point(px(25.0), px(0.0))), None);
    }

    #[test]
    fn a_ramp_paints_one_segment_per_gap() {
        let colors = [
            HexColor::new(0, 0, 0),
            HexColor::new(255, 0, 0),
            HexColor::new(255, 255, 255),
        ];
        assert_eq!(gradient_segments(&colors).len(), 2);
        assert_eq!(gradient_segments(&[colors[0]]).len(), 1);
        assert!(gradient_segments(&[]).is_empty());
    }
}
