//! Settings window: plain eframe/egui form (PRD §How It Looks and Feels).
//! Hotkey recorder, quality presets, output location, theme, font dropdown,
//! 1–3 gradient color pickers, startup checkbox, update cadence,
//! lossy-warning toggle. (Reactor migration is post-MVP.)

use eframe::egui;
use global_hotkey::hotkey::HotKey;
use zest_core::{Settings, Theme, UpdateFrequency, VideoPreset, DEFAULT_CONVERT_HOTKEY};

pub fn run(settings: Settings) -> anyhow::Result<()> {
    run_with_notice(settings, None)
}

pub fn run_with_notice(settings: Settings, startup_notice: Option<String>) -> anyhow::Result<()> {
    let app = SettingsApp {
        settings,
        startup_notice,
    };
    eframe::run_native(
        "Zest Settings",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(app))),
    )
    .map_err(|e| anyhow::anyhow!("settings window: {e:?}"))?;
    Ok(())
}

struct SettingsApp {
    settings: Settings,
    startup_notice: Option<String>,
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let hotkey_error = validate_hotkey(&self.settings.hotkey).err();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Zest Settings");
            ui.separator();
            if let Some(notice) = &self.startup_notice {
                ui.colored_label(egui::Color32::YELLOW, notice);
            }

            ui.horizontal(|ui| {
                ui.label("Hotkey");
                ui.text_edit_singleline(&mut self.settings.hotkey);
            });
            ui.label("Press-record lands in MVP (currently type e.g. Shift+F).");
            if let Some(error) = hotkey_error {
                ui.colored_label(egui::Color32::RED, error);
            }

            ui.horizontal(|ui| {
                ui.label("JPEG quality");
                ui.add(egui::Slider::new(&mut self.settings.jpeg_quality, 1..=100));
            });

            egui::ComboBox::from_label("Video preset")
                .selected_text(format!("{:?}", self.settings.video_preset))
                .show_ui(ui, |ui| {
                    for p in [
                        VideoPreset::Low,
                        VideoPreset::Medium,
                        VideoPreset::High,
                        VideoPreset::Lossless,
                    ] {
                        ui.selectable_value(&mut self.settings.video_preset, p, format!("{p:?}"));
                    }
                });

            ui.horizontal(|ui| {
                ui.label("Audio bitrate (kbps)");
                ui.add(egui::DragValue::new(&mut self.settings.audio_bitrate_kbps));
            });

            egui::ComboBox::from_label("Theme")
                .selected_text(format!("{:?}", self.settings.theme))
                .show_ui(ui, |ui| {
                    for t in [Theme::System, Theme::Light, Theme::Dark] {
                        ui.selectable_value(&mut self.settings.theme, t, format!("{t:?}"));
                    }
                });

            ui.horizontal(|ui| {
                ui.label("Font");
                ui.text_edit_singleline(&mut self.settings.font_family);
            });

            ui.label(format!(
                "Gradient ({} of max 3)",
                self.settings.gradient.0.len()
            ));
            let mut remove = None;
            for (i, color) in self.settings.gradient.0.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Color {}", i + 1));
                    ui.text_edit_singleline(color);
                    if ui.button("−").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                if self.settings.gradient.0.len() > 1 {
                    self.settings.gradient.0.remove(i);
                }
            }
            if self.settings.gradient.0.len() < 3 && ui.button("Add color").clicked() {
                self.settings.gradient.0.push("#ffffff".to_string());
            }

            ui.checkbox(&mut self.settings.launch_at_startup, "Launch at startup");
            egui::ComboBox::from_label("Update frequency")
                .selected_text(format!("{:?}", self.settings.update_frequency))
                .show_ui(ui, |ui| {
                    for f in [
                        UpdateFrequency::Daily,
                        UpdateFrequency::Weekly,
                        UpdateFrequency::Never,
                    ] {
                        ui.selectable_value(
                            &mut self.settings.update_frequency,
                            f,
                            format!("{f:?}"),
                        );
                    }
                });
            ui.checkbox(
                &mut self.settings.warn_lossy_to_lossy,
                "Warn before lossy-to-lossy conversion",
            );

            ui.separator();
            if ui
                .add_enabled(hotkey_error.is_none(), egui::Button::new("Save"))
                .clicked()
            {
                if let Err(e) = self.settings.save() {
                    tracing::warn!("save failed: {e:#}");
                } else {
                    self.startup_notice = None;
                }
            }
        });
    }
}

fn validate_hotkey(value: &str) -> Result<(), &'static str> {
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
}
