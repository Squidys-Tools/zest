//! The controls: rows, buttons, a checkbox, a slider, a stepper, dropdowns.
//!
//! Each is an `impl SettingsView` method because every one of them mutates the
//! view, so a control and its states are read in one place.

use gpui::{
    AnyElement, BorderStyle, Bounds, ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, canvas, div, point, px, quad, size,
    transparent_black,
};
use gpui::prelude::*;

use zest_core::{HexColor, SYSTEM_UI_FONT};

use crate::theme::tint;
use crate::view::{Drag, Menu, SettingsView};

/// A dropdown is taller than this and scrolls.
const MENU_MAX_HEIGHT: f32 = 248.0;
const MENU_ROW_HEIGHT: f32 = 28.0;
const AUDIO_MIN: u32 = 32;
const AUDIO_MAX: u32 = 320;
const AUDIO_STEP: u32 = 8;

impl SettingsView {
    // ── chrome ───────────────────────────────────────────────────────────

    pub fn header(&mut self) -> AnyElement {
        let palette = self.palette.clone();
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

    pub fn notice(&mut self) -> Option<AnyElement> {
        if let Some(error) = &self.save_error {
            let palette = self.palette.clone();
            return Some(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded_md()
                    .border_1()
                    .border_color(palette.danger)
                    .bg(tint(palette.danger, 0.15))
                    .text_sm()
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let notice = self.startup_notice.clone()?;
        let saved = notice == crate::view::SAVED_NOTICE;
        let palette = self.palette.clone();
        Some(
            div()
                .px(px(12.0))
                .py(px(10.0))
                .rounded_md()
                .border_1()
                .border_color(if saved { palette.border } else { palette.warning })
                .bg(if saved {
                    palette.surface_raised
                } else {
                    tint(palette.warning, 0.15)
                })
                .text_sm()
                .child(notice)
                .into_any_element(),
        )
    }

    /// A titled group. `children` are stacked inside it.
    pub fn section(&mut self, title: &str, children: Vec<AnyElement>) -> AnyElement {
        let palette = self.palette.clone();
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
    pub fn field(&mut self, label: &str, control: AnyElement) -> AnyElement {
        let palette = self.palette.clone();
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

    pub fn hint(&mut self, message: &str, danger: bool) -> AnyElement {
        let palette = self.palette.clone();
        div()
            .text_sm()
            .text_color(if danger { palette.danger } else { palette.text_muted })
            .child(message.to_string())
            .into_any_element()
    }

    // ── buttons ──────────────────────────────────────────────────────────

    pub fn save_button(&mut self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let label = div()
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
            .child("Save");
        let control = if disabled {
            label.cursor(gpui::CursorStyle::OperationNotAllowed)
        } else {
            label
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.save();
                    cx.notify();
                }))
        };
        control.into_any_element()
    }

    // ── checkbox ─────────────────────────────────────────────────────────

    pub fn checkbox(
        &mut self,
        cx: &mut Context<Self>,
        label: &str,
        value: bool,
        on_toggle: impl Fn(&mut Self) + Clone + 'static,
    ) -> AnyElement {
        let palette = self.palette.clone();
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
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                on_toggle(this);
                cx.notify();
            }))
            .into_any_element()
    }

    // ── slider ───────────────────────────────────────────────────────────

    /// A track, a fill, and a knob. The canvas records its own bounds so the
    /// drag resolves against the track, not the window.
    pub fn slider(
        &mut self,
        cx: &mut Context<Self>,
        id: &'static str,
        fraction: f32,
        readout: String,
        on_change: impl Fn(f32, &mut Self) + Clone + 'static,
    ) -> AnyElement {
        let palette = self.palette.clone();
        let fraction = fraction.clamp(0.0, 1.0);
        let area = self.slider_area.clone();

        let paint_area = area.clone();
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

        let on_down = on_change.clone();
        let down_area = area.clone();
        let track = div()
            .id(SharedString::from(format!("{id}-track")))
            .h(px(24.0))
            .w_full()
            .cursor_pointer()
            .child(canvas_element)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag::JpegQuality);
                    if let Some(fraction) = fraction_at(&down_area, event.position) {
                        on_down(fraction, this);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(
                move |this, event: &MouseMoveEvent, _, cx| {
                    if this.drag != Some(Drag::JpegQuality) {
                        return;
                    }
                    if let Some(fraction) = fraction_at(&area, event.position) {
                        on_change(fraction, this);
                    }
                    cx.notify();
                },
            ))
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

        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(track)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child("1")
                    .child(readout)
                    .child("100"),
            )
            .into_any_element()
    }

    // ── hotkey recorder ──────────────────────────────────────────────────

    /// Arms the recorder. The next keystroke carrying a modifier becomes the
    /// shortcut, so the user never types one.
    pub fn record_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let recording = self.recording;
        let control = div()
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
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.recording = !this.recording;
                this.editor.editing = None;
                this.menu = Menu::Closed;
                cx.notify();
            }));
        control.into_any_element()
    }

    /// The recorded shortcut, in a field that looks like one.
    pub fn hotkey_readout(&self) -> AnyElement {
        let palette = self.palette.clone();
        div()
            .px(px(12.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(if self.recording {
                palette.accent
            } else {
                palette.border
            })
            .bg(palette.field)
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .child(if self.recording {
                "press a shortcut".to_string()
            } else {
                self.settings.hotkey.clone()
            })
            .into_any_element()
    }

    // ── font ─────────────────────────────────────────────────────────────

    /// Trigger plus, when open, the installed families. An unknown family in
    /// `settings.json` is surfaced rather than silently dropped.
    pub fn font_control(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let open = self.menu == Menu::Font;
        let current = if self.settings.font_family == SYSTEM_UI_FONT {
            "System UI font".to_string()
        } else {
            self.settings.font_family.clone()
        };

        let trigger = div()
            .id("font-trigger")
            .h(px(32.0))
            .w_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .rounded_md()
            .border_1()
            .border_color(if open {
                palette.accent
            } else if self.font_missing {
                palette.danger
            } else {
                palette.border_strong
            })
            .bg(palette.field)
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(palette.surface_raised))
            .child(current)
            .child(
                div()
                    .text_color(palette.text_muted)
                    .child(if open { "▲" } else { "▼" }),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_menu(Menu::Font, cx)
            }));

        let mut list: Vec<gpui::AnyElement> = Vec::new();
        if open {
            if self.font_missing {
                list.push(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .text_xs()
                        .text_color(palette.danger)
                        .child(format!(
                            "{} is not installed on this PC. Pick a family below.",
                            self.settings.font_family
                        ))
                        .into_any_element(),
                );
            }
            if self.settings.font_family == SYSTEM_UI_FONT {
                list.push(
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
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.settings.font_family = SYSTEM_UI_FONT.to_string();
                            this.font_missing = false;
                            this.menu = Menu::Closed;
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            list.push(self.font_list(cx));
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(trigger)
            .child(div().flex().flex_col().children(list).mt(px(4.0)))
            .into_any_element()
    }

    // ── stepper ──────────────────────────────────────────────────────────

    pub fn stepper(&mut self, cx: &mut Context<Self>, value: u32) -> AnyElement {
        let palette = self.palette.clone();
        let step = |this: &mut Self, next: u32, cx: &mut Context<Self>| {
            this.settings.audio_bitrate_kbps = next.clamp(AUDIO_MIN, AUDIO_MAX);
            cx.notify();
        };
        let decrease = div()
            .id("step-down")
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(palette.border)
            .bg(palette.field)
            .child("−")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                step(this, value.saturating_sub(AUDIO_STEP), cx)
            }));
        let increase = div()
            .id("step-up")
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(palette.border)
            .bg(palette.field)
            .child("+")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                step(this, value + AUDIO_STEP, cx)
            }));
        div()
            .id("audio-stepper")
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(decrease)
            .child(
                div()
                    .id("audio-value")
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
                    .child(format!("{value} kbps")),
            )
            .child(increase)
            .into_any_element()
    }

    // ── dropdown ─────────────────────────────────────────────────────────

    pub fn dropdown<T: Clone + PartialEq + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        id: &'static str,
        menu: Menu,
        options: Vec<(String, T)>,
        selected: T,
        on_select: impl Fn(T, &mut Self) + Clone + 'static,
    ) -> AnyElement {
        let palette = self.palette.clone();
        let open = self.menu == menu;
        let current = options
            .iter()
            .find(|(_, value)| value == &selected)
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| "—".to_string());

        let trigger = div()
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
            .border_color(if open { palette.accent } else { palette.border_strong })
            .bg(palette.field)
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(palette.surface_raised))
            .child(current)
            .child(
                div()
                    .text_color(palette.text_muted)
                    .child(if open { "▲" } else { "▼" }),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_menu(menu, cx)
            }));

        let list = if open {
            let mut column = div()
                .id(SharedString::from(format!("{id}-list")))
                .flex()
                .flex_col()
                .mt(px(4.0))
                .max_h(px(MENU_MAX_HEIGHT))
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(palette.border_strong)
                .bg(palette.surface_raised);
            for (index, (label, value)) in options.into_iter().enumerate() {
                let is_selected = value == selected;
                let row_palette = palette.clone();
                let on_select = on_select.clone();
                column = column.child(
                    div()
                        .id((SharedString::from(format!("{id}-option")), index))
                        .h(px(MENU_ROW_HEIGHT))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .bg(if is_selected {
                            tint(row_palette.accent, 0.3)
                        } else {
                            row_palette.surface_raised
                        })
                        .hover(|style| style.bg(row_palette.surface))
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            on_select(value.clone(), this);
                            this.menu = Menu::Closed;
                            cx.notify();
                        })),
                );
            }
            column.into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(trigger)
            .child(list)
            .into_any_element()
    }

    /// Installed families, each shown in its own typeface. A plain scrollable
    /// column rather than a virtualized list: the panel is only on screen while
    /// the user is picking a font, and every row needs its own click listener.
    pub fn font_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.clone();
        let names = self.font_names.clone();
        let current = self.settings.font_family.clone();
        let missing = self.font_missing;
        let mut rows = Vec::with_capacity(names.len());
        for (index, name) in names.into_iter().enumerate() {
            let selected = !missing && name.eq_ignore_ascii_case(&current);
            let row_palette = palette.clone();
            rows.push(
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
                        tint(row_palette.accent, 0.3)
                    } else {
                        row_palette.surface_raised
                    })
                    .hover(|style| style.bg(row_palette.surface))
                    .child(name.clone())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings.font_family = name.to_string();
                        this.font_missing = false;
                        this.menu = Menu::Closed;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
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
            .into_any_element()
    }

    // ── color chips ──────────────────────────────────────────────────────

    /// gpui backgrounds hold exactly two stops, so a 3-color ramp is painted as
    /// adjacent segments. The overlay draws all stops in one D2D brush; the
    /// seam between segments is the only difference.
    pub fn gradient_bar(&self, colors: &[HexColor], height: f32) -> AnyElement {
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

    pub fn swatch(&self, color: HexColor, selected: bool) -> AnyElement {
        let palette = self.palette.clone();
        div()
            .size(px(20.0))
            .rounded_sm()
            .border_1()
            .border_color(if selected { palette.accent } else { palette.window })
            .bg(to_color(color))
            .into_any_element()
    }
}

/// Where the pointer sits along a recorded track, 0..=1.
pub fn fraction_at(
    area: &std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    at: Point<Pixels>,
) -> Option<f32> {
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

pub enum Axis {
    Horizontal,
    Vertical,
}

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
