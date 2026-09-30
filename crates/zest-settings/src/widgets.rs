//! The controls: what each one does, and which handler mutates which setting.
//!
//! Every method here is a thin wrapper. The drawing lives in [`crate::ui`], and
//! these wrappers do the two things that genuinely belong to the view: reading
//! the state that decides what to draw, and turning a click into a state change
//! plus a `notify`. Keeping that boundary explicit is the point — a control's
//! look can now be changed without touching the code that changes its value.

use gpui::{
    AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    div, px,
};
use gpui::prelude::*;

use zest_core::{HexColor, SYSTEM_UI_FONT};

use crate::ui;
use crate::view::{Drag, Menu, SettingsView};

const AUDIO_MIN: u32 = 32;
const AUDIO_MAX: u32 = 320;
const AUDIO_STEP: u32 = 8;

impl SettingsView {
    // ── chrome ───────────────────────────────────────────────────────────

    pub fn header(&mut self) -> AnyElement {
        ui::header(&self.palette)
    }

    /// Prefers the save error over the startup notice, because an error is the
    /// one the user has to act on.
    pub fn notice(&mut self) -> Option<AnyElement> {
        if let Some(error) = &self.save_error {
            return Some(ui::notice(&self.palette, error, ui::Tone::Error));
        }
        let notice = self.startup_notice.clone()?;
        let saved = notice == crate::view::SAVED_NOTICE;
        let tone = if saved { ui::Tone::Saved } else { ui::Tone::Warning };
        Some(ui::notice(&self.palette, &notice, tone))
    }

    pub fn section(&mut self, title: &str, children: Vec<AnyElement>) -> AnyElement {
        ui::section(&self.palette, title, children)
    }

    pub fn field(&mut self, label: &str, control: AnyElement) -> AnyElement {
        ui::field(&self.palette, label, control)
    }

    pub fn hint(&mut self, message: &str, danger: bool) -> AnyElement {
        ui::hint(&self.palette, message, danger)
    }

    // ── buttons ──────────────────────────────────────────────────────────

    pub fn save_button(&mut self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let control = ui::save_button(&self.palette, disabled);
        if disabled {
            control
                .cursor(gpui::CursorStyle::OperationNotAllowed)
                .into_any_element()
        } else {
            control
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.save();
                    cx.notify();
                }))
                .into_any_element()
        }
    }

    // ── checkbox ─────────────────────────────────────────────────────────

    pub fn checkbox(
        &mut self,
        cx: &mut Context<Self>,
        label: &str,
        value: bool,
        on_toggle: impl Fn(&mut Self) + Clone + 'static,
    ) -> AnyElement {
        ui::checkbox(&self.palette, label, value)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                on_toggle(this);
                cx.notify();
            }))
            .into_any_element()
    }

    // ── slider ───────────────────────────────────────────────────────────

    /// `drag_tag` is the [`Drag`] value this slider claims while the mouse is
    /// down, so several sliders can share one drag state machine.
    pub fn slider(
        &mut self,
        cx: &mut Context<Self>,
        id: &'static str,
        drag_tag: Drag,
        fraction: f32,
        readout: String,
        on_change: impl Fn(f32, &mut Self) + Clone + 'static,
    ) -> AnyElement {
        let area = self.slider_area.clone();
        let (mut track, scale) = ui::slider(&self.palette, id, fraction, readout, area);

        let on_down = on_change.clone();
        let down_area = self.slider_area.clone();
        track = track
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.drag = Some(drag_tag);
                    if let Some(fraction) = ui::fraction_at(&down_area, event.position) {
                        on_down(fraction, this);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(
                move |this, event: &MouseMoveEvent, _, cx| {
                    if this.drag != Some(drag_tag) {
                        return;
                    }
                    if let Some(fraction) = ui::fraction_at(&this.slider_area, event.position) {
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
            .child(scale)
            .into_any_element()
    }

    // ── hotkey recorder ──────────────────────────────────────────────────

    /// Arms the recorder. The next keystroke carrying a modifier becomes the
    /// shortcut, so the user never types one.
    pub fn record_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let recording = self.recording;
        ui::record_button(&self.palette, recording)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.recording = !this.recording;
                this.editor.editing = None;
                this.menu = Menu::Closed;
                cx.notify();
            }))
            .into_any_element()
    }

    pub fn hotkey_readout(&self) -> AnyElement {
        ui::hotkey_readout(&self.palette, self.recording, &self.settings.hotkey).into_any_element()
    }

    // ── font ─────────────────────────────────────────────────────────────

    /// Trigger plus, when open, the installed families. An unknown family in
    /// `settings.json` is surfaced rather than silently dropped.
    pub fn font_control(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.menu == Menu::Font;
        let current = if self.settings.font_family == SYSTEM_UI_FONT {
            "System UI font".to_string()
        } else {
            self.settings.font_family.clone()
        };

        let trigger = ui::font_trigger(&self.palette, current, open, self.font_missing)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_menu(Menu::Font, cx)
            }))
            .into_any_element();

        let mut list: Vec<AnyElement> = Vec::new();
        if open {
            if self.font_missing {
                list.push(ui::font_missing_note(
                    &self.palette,
                    &self.settings.font_family,
                ));
            }
            if self.settings.font_family == SYSTEM_UI_FONT {
                list.push(
                    ui::font_system_row(&self.palette)
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

        ui::font_body(trigger, list).into_any_element()
    }

    fn font_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let names = self.font_names.clone();
        let current = self.settings.font_family.clone();
        let missing = self.font_missing;
        let mut rows = Vec::with_capacity(names.len());
        for (index, name) in names.into_iter().enumerate() {
            let selected = !missing && name.eq_ignore_ascii_case(&current);
            let row_palette = self.palette.clone();
            rows.push(
                ui::font_row(&row_palette, index, name.clone(), selected)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings.font_family = name.to_string();
                        this.font_missing = false;
                        this.menu = Menu::Closed;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        ui::font_list(&self.palette, rows).into_any_element()
    }

    // ── stepper ──────────────────────────────────────────────────────────

    pub fn stepper(&mut self, cx: &mut Context<Self>, value: u32) -> AnyElement {
        let parts = ui::stepper(&self.palette, value, "kbps");
        let step = |this: &mut Self, next: u32, cx: &mut Context<Self>| {
            this.settings.audio_bitrate_kbps = next.clamp(AUDIO_MIN, AUDIO_MAX);
            cx.notify();
        };
        let decrease = parts
            .decrease
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                step(this, value.saturating_sub(AUDIO_STEP), cx)
            }));
        let increase = parts
            .increase
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                step(this, value + AUDIO_STEP, cx)
            }));
        div()
            .id("audio-stepper")
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(decrease)
            .child(parts.value)
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
        let open = self.menu == menu;
        let current = options
            .iter()
            .find(|(_, value)| value == &selected)
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| "—".to_string());

        let trigger = ui::menu_trigger(&self.palette, id, current, open, false)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_menu(menu, cx)
            }))
            .into_any_element();

        let list = if open {
            let mut rows = Vec::with_capacity(options.len());
            for (index, (label, value)) in options.into_iter().enumerate() {
                let is_selected = value == selected;
                let row_palette = self.palette.clone();
                let on_select = on_select.clone();
                rows.push(
                    ui::option_row(&row_palette, id, index, label, is_selected)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            on_select(value.clone(), this);
                            this.menu = Menu::Closed;
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            ui::menu_list(&self.palette, id, rows).into_any_element()
        } else {
            div().into_any_element()
        };

        ui::dropdown_body(trigger, list).into_any_element()
    }

    // ── colour chips ──────────────────────────────────────────────────────

    pub fn gradient_bar(&self, colors: &[HexColor], height: f32) -> AnyElement {
        ui::gradient_bar(colors, height)
    }

    pub fn swatch(&self, color: HexColor, selected: bool) -> AnyElement {
        ui::swatch(&self.palette, color, selected)
    }
}
