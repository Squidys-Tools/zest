//! The sector gradient editor.
//!
//! The preview and the picker both read `Settings::gradient`, which is the same
//! value `zest-overlay` paints with, so the preview cannot drift from the ring.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, BorderStyle, Bounds, ClickEvent, Context, Half, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, canvas, div, linear_color_stop,
    linear_gradient, point, prelude::*, px, quad, size, transparent_black,
};

use zest_core::{HexColor, MAX_GRADIENT_COLORS};

use crate::view::{Drag, SettingsView};
use crate::ui::{Axis, fraction_in, gradient_segments, to_color};

/// Size of the two picker surfaces, and the geometry the marker maths uses.
const PICKER_WIDTH: f32 = 240.0;
const SV_HEIGHT: f32 = 140.0;
const HUE_HEIGHT: f32 = 16.0;
const PREVIEW_HEIGHT: f32 = 14.0;

pub struct GradientEditor {
    /// Which color's picker is open, if any.
    pub editing: Option<usize>,
    /// Bounds recorded during prepaint; mouse positions are window-relative.
    pub sv_area: Rc<RefCell<Option<Bounds<Pixels>>>>,
    pub hue_area: Rc<RefCell<Option<Bounds<Pixels>>>>,
}

impl GradientEditor {
    pub fn new() -> Self {
        Self {
            editing: None,
            sv_area: Rc::new(RefCell::new(None)),
            hue_area: Rc::new(RefCell::new(None)),
        }
    }
}

impl Default for GradientEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsView {
    pub fn gradient_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let colors: Vec<HexColor> = self.settings.gradient.colors().to_vec();
        let can_add = colors.len() < MAX_GRADIENT_COLORS;

        // The real thing: one sector of the ring, filled with the same ramp.
        let preview = self.sector_preview(colors.clone());
        let bar = self.gradient_bar(&colors, PREVIEW_HEIGHT);

        let mut rows = div().flex().flex_col().gap(px(6.0));
        for (index, color) in colors.iter().enumerate() {
            rows = rows.child(self.color_row(index, *color, cx));
        }

        let add = if can_add {
            let palette = palette.clone();
            Some(
                div()
                    .flex()
                    .justify_start()
                    .child(
                        div()
                            .id("gradient-add")
                            .px(px(10.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .border_1()
                            .border_color(palette.border_strong)
                            .bg(palette.field)
                            .text_sm()
                            .text_color(palette.text_muted)
                            .cursor_pointer()
                            .hover(|style| style.bg(palette.surface_raised))
                            .child("+ Add colour")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                if this
                                    .settings
                                    .gradient
                                    .push(next_color(this.settings.gradient.colors()))
                                {
                                    this.editor.editing =
                                        Some(this.settings.gradient.colors().len() - 1);
                                    cx.notify();
                                }
                            })),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };

        let picker = match self.editor.editing {
            Some(index) if index < colors.len() => self.color_picker(index, cx),
            _ => None,
        };

        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(preview)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(bar)
                            .child(self.gradient_step_labels(&colors, &palette)),
                    ),
            )
            .child(rows)
            .children([add, picker].into_iter().flatten())
            .into_any_element()
    }

    /// One label per stop, spread across the width of the bar above them.
    fn gradient_step_labels(&self, colors: &[HexColor], palette: &crate::theme::Palette) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .justify_between()
            .text_xs()
            .text_color(palette.text_muted)
            .children(colors.iter().enumerate().map(|(index, color)| {
                div()
                    .flex_1()
                    .child(color.to_hex())
                    .child(format!("stop {index}"))
            }))
            .into_any_element()
    }

    /// One sector of the radial overlay, filled with the configured ramp. The
    /// arcs are subdivided rather than chorded, so the wedge reads as a ring
    /// segment. With 3 colors it is drawn as adjacent sub-wedges, because a
    /// gpui background holds at most 2 stops.
    fn sector_preview(&self, colors: Vec<HexColor>) -> AnyElement {
        let segments = gradient_segments(&colors);
        div()
            .size(px(112.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        // `point` takes plain f32, so work in f32 from here.
                        let half: f32 = f32::from(bounds.size.width.half());
                        let radius = half;
                        let center_x = f32::from(bounds.origin.x) + half;
                        let center_y = f32::from(bounds.origin.y) + half;
                        // One wedge, 100 degrees wide, opening upwards.
                        let start = std::f64::consts::FRAC_PI_2 + 0.35;
                        let sweep = 100.0_f64.to_radians();
                        let inner = radius * 0.45;
                        const STEPS: usize = 16;
                        let count = segments.len();
                        let at = |r: f32, angle: f64| {
                            (
                                px(center_x + r * (angle.cos() as f32)),
                                px(center_y - r * (angle.sin() as f32)),
                            )
                        };

                        for (slice, background) in segments.iter().enumerate() {
                            let from = start + sweep * slice as f64 / count as f64;
                            let to = start + sweep * (slice + 1) as f64 / count as f64;
                            let mut builder = PathBuilder::fill();
                            // Outer edge, out along the sweep.
                            for step in 0..=STEPS {
                                let angle = from + (to - from) * step as f64 / STEPS as f64;
                                let (x, y) = at(radius, angle);
                                let vertex = point(x, y);
                                if step == 0 {
                                    builder.move_to(vertex);
                                } else {
                                    builder.line_to(vertex);
                                }
                            }
                            // Inner edge, back the other way.
                            for step in (0..=STEPS).rev() {
                                let angle = from + (to - from) * step as f64 / STEPS as f64;
                                let (x, y) = at(inner, angle);
                                builder.line_to(point(x, y));
                            }
                            let path = builder.build().expect("closed ring segment");
                            window.paint_path(path, *background);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn color_row(&mut self, index: usize, color: HexColor, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let open = self.editor.editing == Some(index);
        let removable = self.settings.gradient.colors().len() > 1;

        let swatch_button = div()
            .id(("gradient-swatch", index))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .child(self.swatch(color, open))
            .child(
                div()
                    .text_sm()
                    .font(gpui::font("Consolas"))
                    .text_color(palette.text_muted)
                    .child(color.to_hex()),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.editor.editing = if open { None } else { Some(index) };
                cx.notify();
            }));

        let remove = if removable {
            div()
                .id(("gradient-remove", index))
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .text_sm()
                .text_color(palette.text_muted)
                .cursor_pointer()
                .hover(|style| style.bg(palette.surface_raised))
                .child("−")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let removed = this.settings.gradient.remove(index);
                    match (removed, this.editor.editing) {
                        (Some(_), Some(editing)) if editing >= index => {
                            let len = this.settings.gradient.colors().len();
                            this.editor.editing =
                                (editing > 0).then(|| (editing - 1).min(len.saturating_sub(1)));
                        }
                        (Some(_), None) => {}
                        (None, Some(editing)) if editing == index => this.editor.editing = None,
                        _ => {}
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            div().size(px(26.0)).into_any_element()
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .child(swatch_button)
            .child(remove)
            .into_any_element()
    }

    /// Saturation/value square plus a hue strip, for the color being edited.
    fn color_picker(&mut self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = self.settings.gradient.colors();
        let color = *colors.get(index)?;
        let (hue, saturation, value) = color.to_hsv();
        let palette = self.palette.clone();

        // The square: the pure hue, darkened left-to-right and top-to-bottom.
        let sv_area = self.editor.sv_area.clone();
        let hue_at_paint = hue;
        let record = sv_area.clone();
        let paint_area = sv_area.clone();
        let square = div()
            .id(("gradient-sv", index))
            .relative()
            .w(px(PICKER_WIDTH))
            .h(px(SV_HEIGHT))
            .rounded_sm()
            .overflow_hidden()
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| {
                        *record.borrow_mut() = Some(bounds);
                    },
                    move |bounds, _, window, _| {
                        let base = HexColor::from_hsv(hue_at_paint, 1.0, 1.0);
                        window.paint_quad(quad(
                            bounds,
                            px(4.0),
                            to_color(base),
                            px(0.0),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        // Saturation falls left to right, value top to bottom.
                        window.paint_quad(quad(
                            bounds,
                            px(4.0),
                            linear_gradient(
                                0.0,
                                linear_color_stop(gpui::transparent_black(), 0.0),
                                linear_color_stop(gpui::black(), 1.0),
                            ),
                            px(0.0),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        window.paint_quad(quad(
                            bounds,
                            px(4.0),
                            linear_gradient(
                                90.0,
                                linear_color_stop(gpui::transparent_black(), 0.0),
                                linear_color_stop(gpui::black(), 1.0),
                            ),
                            px(0.0),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                    },
                )
                .size_full(),
            )
            .child(marker(
                saturation,
                1.0 - value,
                PICKER_WIDTH,
                SV_HEIGHT,
                palette.window,
                to_color(color),
            ))
            .on_mouse_down(MouseButton::Left, {
                let sv_area = paint_area.clone();
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag::SaturationValue(index));
                    apply_sv(this, &sv_area, event.position, index);
                    cx.notify();
                })
            })
            .on_mouse_move({
                let sv_area = paint_area.clone();
                cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.drag != Some(Drag::SaturationValue(index)) {
                        return;
                    }
                    apply_sv(this, &sv_area, event.position, index);
                    cx.notify();
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag = None;
                }),
            );

        // The hue strip: gpui gradients are 2 stops, so it is 24 slices.
        let hue_area = self.editor.hue_area.clone();
        let strip = div()
            .id(("gradient-hue", index))
            .relative()
            .w(px(PICKER_WIDTH))
            .h(px(HUE_HEIGHT))
            .rounded_sm()
            .overflow_hidden()
            .cursor_pointer()
            .child(
                canvas(
                    {
                        let hue_area = hue_area.clone();
                        move |bounds, _, _| {
                            *hue_area.borrow_mut() = Some(bounds);
                        }
                    },
                    move |bounds, _, window, _| {
                        const SLICES: f32 = 24.0;
                        let slice_width = bounds.size.width / SLICES;
                        for slice in 0..SLICES as u16 {
                            let from = HexColor::from_hsv(slice as f32 * (360.0 / SLICES), 1.0, 1.0);
                            let to = HexColor::from_hsv(
                                (slice + 1) as f32 * (360.0 / SLICES),
                                1.0,
                                1.0,
                            );
                            let slice_bounds = Bounds {
                                origin: point(
                                    bounds.origin.x + slice_width * slice as f32,
                                    bounds.origin.y,
                                ),
                                size: size(slice_width + px(1.0), bounds.size.height),
                            };
                            window.paint_quad(quad(
                                slice_bounds,
                                px(0.0),
                                linear_gradient(
                                    0.0,
                                    linear_color_stop(to_color(from), 0.0),
                                    linear_color_stop(to_color(to), 1.0),
                                ),
                                px(0.0),
                                transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .child(marker(
                hue / 360.0,
                0.5,
                PICKER_WIDTH,
                HUE_HEIGHT,
                palette.window,
                to_color(HexColor::from_hsv(hue, 1.0, 1.0)),
            ))
            .on_mouse_down(MouseButton::Left, {
                let hue_area = hue_area.clone();
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag::Hue(index));
                    apply_hue(this, &hue_area, event.position, index);
                    cx.notify();
                })
            })
            .on_mouse_move({
                let hue_area = hue_area.clone();
                cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.drag != Some(Drag::Hue(index)) {
                        return;
                    }
                    apply_hue(this, &hue_area, event.position, index);
                    cx.notify();
                })
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag = None;
                }),
            );

        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .mt(px(4.0))
                .w(px(PICKER_WIDTH + 20.0))
                .p(px(10.0))
                .rounded_md()
                .border_1()
                .border_color(palette.border)
                .bg(palette.surface)
                .child(square)
                .child(strip)
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child("Drag to set the colour. The ring uses it the next time it opens."),
                )
                .into_any_element(),
        )
    }
}

/// The crosshair on the saturation/value square, or the handle on the hue strip.
/// `x` and `y` are 0..=1 across the surface.
fn marker(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    ring: gpui::Hsla,
    fill: gpui::Rgba,
) -> AnyElement {
    div()
        .absolute()
        .left(px(x * width - 7.0))
        .top(px(y * height - 7.0))
        .size(px(14.0))
        .rounded_full()
        .border_2()
        .border_color(ring)
        .bg(fill)
        .into_any_element()
}

fn apply_sv(
    view: &mut SettingsView,
    area: &Rc<RefCell<Option<Bounds<Pixels>>>>,
    at: Point<Pixels>,
    index: usize,
) {
    let Some(bounds) = *area.borrow() else {
        return;
    };
    let saturation = fraction_in(bounds, at, Axis::Horizontal).unwrap_or(0.0);
    let value = 1.0 - fraction_in(bounds, at, Axis::Vertical).unwrap_or(0.0);
    // The square only knows saturation and value, so the hue comes from the
    // color being edited.
    let Some(current) = view.settings.gradient.colors().get(index).copied() else {
        return;
    };
    let (hue, _, _) = current.to_hsv();
    view.settings
        .gradient
        .set(index, HexColor::from_hsv(hue, saturation, value));
}

fn apply_hue(
    view: &mut SettingsView,
    area: &Rc<RefCell<Option<Bounds<Pixels>>>>,
    at: Point<Pixels>,
    index: usize,
) {
    let Some(bounds) = *area.borrow() else {
        return;
    };
    let Some(current) = view.settings.gradient.colors().get(index).copied() else {
        return;
    };
    let hue = fraction_in(bounds, at, Axis::Horizontal).unwrap_or(0.0) * 360.0;
    let (_, saturation, value) = current.to_hsv();
    view.settings
        .gradient
        .set(index, HexColor::from_hsv(hue, saturation, value));
}

/// A new stop that is visibly different from the ones already there.
fn next_color(existing: &[HexColor]) -> HexColor {
    const CANDIDATES: [HexColor; 8] = [
        HexColor::new(0xff, 0xd1, 0x5b),
        HexColor::new(0x4d, 0xc4, 0xff),
        HexColor::new(0x8a, 0xf0, 0x6a),
        HexColor::new(0xc4, 0x8a, 0xff),
        HexColor::new(0xff, 0x6a, 0x8a),
        HexColor::new(0x6a, 0x8a, 0xff),
        HexColor::new(0xff, 0x8a, 0x3d),
        HexColor::new(0xff, 0x4d, 0x6d),
    ];
    CANDIDATES
        .iter()
        .find(|candidate| !existing.contains(candidate))
        .copied()
        .unwrap_or(HexColor::new(0xff, 0xff, 0xff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_stop_is_not_a_duplicate_of_an_existing_one() {
        let existing = vec![HexColor::new(0xff, 0xd1, 0x5b)];
        assert_ne!(next_color(&existing), existing[0]);
    }

    #[test]
    fn a_full_gradient_still_returns_something_drawable() {
        let all: Vec<HexColor> = (0..MAX_GRADIENT_COLORS)
            .map(|i| HexColor::new(i as u8 * 40, 0, 0))
            .collect();
        assert!(!next_color(&all).to_hex().is_empty());
    }
}
