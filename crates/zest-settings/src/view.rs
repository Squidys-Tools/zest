//! The settings window: a gpui view over `zest_core::Settings`.
//!
//! State is a plain struct, edits mutate it in place, and Save writes the whole
//! thing — the same contract the egui form had.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    px, App, Application, Bounds, Context, FocusHandle, Focusable, Pixels, Render, SharedString,
    Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, size, prelude::*,
};

use zest_core::{Settings, Theme, SYSTEM_UI_FONT};

use crate::gradient::GradientEditor;
use crate::hotkey::{has_modifier, hotkey_from_keystroke};
use crate::theme::{palette_for, Palette};

/// Shown after a successful save.
pub const SAVED_NOTICE: &str =
    "Saved. Restart Zest to pick up changes that only load at startup.";

/// Which menu is expanded. One at a time keeps the layout honest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Closed,
    VideoPreset,
    Theme,
    Font,
    UpdateFrequency,
}

/// What the left mouse button is currently dragging.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    JpegQuality,
    SaturationValue(usize),
    Hue(usize),
}

pub struct SettingsView {
    pub settings: Settings,
    pub palette: Palette,
    pub startup_notice: Option<String>,
    pub save_error: Option<String>,
    pub font_names: Vec<SharedString>,
    pub font_missing: bool,
    pub menu: Menu,
    pub recording: bool,
    pub drag: Option<Drag>,
    pub editor: GradientEditor,
    /// The slider canvas records its own bounds during prepaint, so a drag can
    /// be resolved against the track rather than the window.
    pub slider_area: Rc<RefCell<Option<Bounds<Pixels>>>>,
    focus_handle: FocusHandle,
}

impl SettingsView {
    pub fn new(
        settings: Settings,
        startup_notice: Option<String>,
        font_names: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        // The recorder listens app-wide, so `recording` is what stops a stray
        // keypress elsewhere in the window from becoming a shortcut.
        cx.observe_keystrokes(|this, event: &gpui::KeystrokeEvent, _window, cx| {
            if !this.recording || !has_modifier(&event.keystroke) {
                return;
            }
            if let Some(hotkey) = hotkey_from_keystroke(&event.keystroke) {
                this.settings.hotkey = hotkey;
            }
            this.recording = false;
            cx.notify();
        })
        .detach();

        let palette = palette_for(settings.theme);
        let installed: Vec<SharedString> = font_names
            .into_iter()
            // `.SystemUIFont` and friends are sentinels, not installed families.
            .filter(|name| !name.starts_with('.'))
            .map(SharedString::from)
            .collect();
        let font_missing = settings.font_family != SYSTEM_UI_FONT
            && !installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&settings.font_family));

        Self {
            settings,
            palette,
            startup_notice,
            save_error: None,
            font_names: installed,
            font_missing,
            menu: Menu::Closed,
            recording: false,
            drag: None,
            editor: GradientEditor::new(),
            slider_area: Rc::new(RefCell::new(None)),
            focus_handle: cx.focus_handle(),
        }
    }

    /// The family actually rendered. An unknown name in `settings.json` falls
    /// back to the text stack's own resolution instead of a blank window.
    pub fn interface_font(&self) -> SharedString {
        SharedString::from(self.settings.font_family.clone())
    }

    pub fn apply_theme(&mut self, theme: Theme) {
        self.settings.theme = theme;
        self.palette = palette_for(theme);
    }

    pub fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == menu {
            Menu::Closed
        } else {
            menu
        };
        cx.notify();
    }

    pub fn save(&mut self) {
        match self.settings.save() {
            Ok(()) => {
                self.save_error = None;
                self.startup_notice = Some(SAVED_NOTICE.to_string());
            }
            Err(error) => {
                self.save_error = Some(format!("Could not save settings: {error:#}"));
            }
        }
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _app: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(16.0));
        window.set_background_appearance(WindowBackgroundAppearance::Opaque);
        self.compose(cx)
    }
}

/// Window + view bootstrap. Blocks until the window closes, like `run_native`.
pub(crate) fn run_window(
    settings: Settings,
    startup_notice: Option<String>,
) -> anyhow::Result<()> {
    let failure: Rc<RefCell<Option<anyhow::Error>>> = Rc::new(RefCell::new(None));
    let report = Rc::clone(&failure);
    Application::new().run(move |app: &mut App| {
        let font_names = app.text_system().all_font_names();
        match app.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(700.0), px(880.0)),
                    app,
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Zest Settings".into()),
                    ..Default::default()
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: Some(size(px(460.0), px(420.0))),
                focus: true,
                show: true,
                ..Default::default()
            },
            |window, cx| {
                window.on_window_should_close(cx, |_, _| true);
                cx.new(|cx| SettingsView::new(settings, startup_notice, font_names, cx))
            },
        ) {
            Ok(view) => {
                // The settings window is the whole app here, so closing it ends
                // the run loop too.
                app.on_window_closed(|app| {
                    if app.windows().is_empty() {
                        app.quit();
                    }
                })
                .detach();
                if let Err(error) = view.update(app, |_, window, _cx| window.activate_window()) {
                    *report.borrow_mut() = Some(error);
                }
                app.activate(true);
            }
            Err(error) => *report.borrow_mut() = Some(error),
        }
    });
    let outcome = failure.borrow_mut().take();
    match outcome {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
