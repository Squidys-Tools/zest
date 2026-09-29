//! Settings window: a gpui form (PRD §How It Looks and Feels).
//! Hotkey press recorder, JPEG quality, video preset, audio bitrate, theme,
//! font dropdown, 1–3 gradient colors with a live sector preview, startup
//! checkbox, update cadence, lossy-warning toggle.
//!
//! Runs on its own thread; the app loop and the Direct2D overlay are untouched.

mod gradient;
mod hotkey;
mod theme;
mod view;
mod widgets;

use gpui::{div, px, prelude::*};
use zest_core::{
    Settings, Theme, UpdateFrequency, VideoPreset, DEFAULT_CONVERT_HOTKEY, SYSTEM_UI_FONT,
};

use view::{Menu, SettingsView};

/// Open the settings window with no notice.
pub fn run(settings: Settings) -> anyhow::Result<()> {
    view::run_window(settings, None)
}

pub fn run_with_notice(settings: Settings, startup_notice: Option<String>) -> anyhow::Result<()> {
    view::run_window(settings, startup_notice)
}

// ── the form ────────────────────────────────────────────────────────────

impl SettingsView {
    pub fn compose(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut page = div().flex().flex_col().gap(px(20.0));
        page = page.child(self.header());
        if let Some(notice) = self.notice() {
            page = page.child(notice);
        }
        page = page
            .child(self.hotkey_section(cx))
            .child(self.conversion_section(cx))
            .child(self.appearance_section(cx))
            .child(self.updates_section(cx))
            .child(self.footer(cx));

        let palette = self.palette.clone();
        let font = self.interface_font();
        div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(palette.window)
            .text_color(palette.text)
            .font_family(font)
            .child(
                div()
                    .mx_auto()
                    .w(px(660.0))
                    .px(px(28.0))
                    .py(px(24.0))
                    .child(page),
            )
            .into_any_element()
    }

    fn hotkey_section(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let record = self.record_button(cx);
        let readout = self.hotkey_readout();
        let capture = self.field(
            "Presses are recorded, not typed",
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(record)
                .child(readout)
                .into_any_element(),
        );
        self.section("Menu shortcut", vec![capture])
    }

    fn conversion_section(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let quality = self.settings.jpeg_quality;
        let slider = self.slider(
            cx,
            "jpeg-quality",
            f32::from(quality - 1) / 99.0,
            quality.to_string(),
            |fraction, this| {
                this.settings.jpeg_quality = (1 + (fraction * 99.0).round() as u16) as u8;
            },
        );
        let preset = self.dropdown(
            cx,
            "video-preset",
            Menu::VideoPreset,
            vec![
                ("Low".to_string(), VideoPreset::Low),
                ("Medium".to_string(), VideoPreset::Medium),
                ("High".to_string(), VideoPreset::High),
                ("Lossless".to_string(), VideoPreset::Lossless),
            ],
            self.settings.video_preset,
            |value, this| this.settings.video_preset = value,
        );
        let bitrate = self.stepper(cx, self.settings.audio_bitrate_kbps);
        let lossy = self.checkbox(
            cx,
            "Warn before lossy-to-lossy conversion",
            self.settings.warn_lossy_to_lossy,
            |this| this.settings.warn_lossy_to_lossy = !this.settings.warn_lossy_to_lossy,
        );
        let rows = vec![
            self.field("JPEG quality (1–100)", slider),
            self.field("Video preset", preset),
            self.field("Audio bitrate", bitrate),
            lossy,
        ];
        self.section("Conversion", rows)
    }

    fn appearance_section(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = self.dropdown(
            cx,
            "theme",
            Menu::Theme,
            vec![
                ("System".to_string(), Theme::System),
                ("Light".to_string(), Theme::Light),
                ("Dark".to_string(), Theme::Dark),
            ],
            self.settings.theme,
            // The dropdown re-renders after `on_select`, so rebuilding the
            // palette here is enough.
            |value, this| this.apply_theme(value),
        );
        let font = self.font_control(cx);
        let gradient = self.gradient_editor(cx);
        let rows = vec![
            self.field("Theme", theme),
            self.field("Interface font", font),
            gradient,
        ];
        self.section("Appearance", rows)
    }

    fn updates_section(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let startup = self.checkbox(
            cx,
            "Launch at startup",
            self.settings.launch_at_startup,
            |this| this.settings.launch_at_startup = !this.settings.launch_at_startup,
        );
        let frequency = self.dropdown(
            cx,
            "update-frequency",
            Menu::UpdateFrequency,
            vec![
                ("Daily".to_string(), UpdateFrequency::Daily),
                ("Weekly".to_string(), UpdateFrequency::Weekly),
                ("Never".to_string(), UpdateFrequency::Never),
            ],
            self.settings.update_frequency,
            |value, this| this.settings.update_frequency = value,
        );
        self.section("Startup & updates", vec![startup, frequency])
    }

    fn footer(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let invalid = crate::validate_hotkey(&self.settings.hotkey).err();
        let save = self.save_button(invalid.is_some(), cx);
        let status = match invalid {
            Some(message) => self.hint(message, true),
            None => div().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(8.0))
            .child(status)
            .child(save)
            .into_any_element()
    }
}

fn _system_font() -> &'static str {
    SYSTEM_UI_FONT
}

fn validate_hotkey(value: &str) -> Result<(), &'static str> {
    use global_hotkey::hotkey::HotKey;
    let hotkey = value
        .parse::<HotKey>()
        .map_err(|_| "Enter a valid shortcut, such as Shift+F.")?;
    let convert_hotkey = DEFAULT_CONVERT_HOTKEY
        .parse::<HotKey>()
        .expect("built-in Convert shortcut is valid");
    if hotkey == convert_hotkey {
        return Err("Shift+C is reserved for opening the Convert ring.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_default_hotkey() {
        assert!(validate_hotkey("Shift+F").is_ok());
    }

    #[test]
    fn rejects_invalid_and_reserved_hotkeys() {
        assert!(validate_hotkey("Shift+").is_err());
        assert!(validate_hotkey(DEFAULT_CONVERT_HOTKEY).is_err());
    }

    #[test]
    fn accepts_what_the_recorder_can_produce() {
        for recorded in ["Ctrl+Shift+K", "Alt+SPACE", "Ctrl+Alt+SUPER+SHIFT+A", "Ctrl+F7"] {
            assert!(
                validate_hotkey(recorded).is_ok(),
                "{recorded} should be accepted"
            );
        }
    }
}
