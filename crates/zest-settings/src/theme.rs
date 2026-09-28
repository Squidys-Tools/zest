//! The settings window palette.
//!
//! `Theme::System` reads the Windows app-mode preference rather than guessing,
//! so "System" means something the user can see. Dark is the default, matching
//! the overlay.

use gpui::{hsla, Hsla};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD,
};

use zest_core::Theme;

#[derive(Debug, Clone)]
pub struct Palette {
    pub window: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub field: Hsla,
    pub track: Hsla,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            window: hsla(0.0, 0.0, 0.0, 0.98),
            surface: hsla(240.0, 0.04, 0.11, 1.0),
            surface_raised: hsla(240.0, 0.04, 0.16, 1.0),
            border: hsla(240.0, 0.05, 0.22, 1.0),
            border_strong: hsla(240.0, 0.06, 0.32, 1.0),
            text: hsla(0.0, 0.0, 0.93, 1.0),
            text_muted: hsla(240.0, 0.05, 0.62, 1.0),
            accent: hsla(24.0, 0.95, 0.62, 1.0),
            danger: hsla(352.0, 0.78, 0.62, 1.0),
            warning: hsla(45.0, 0.92, 0.62, 1.0),
            field: hsla(240.0, 0.04, 0.19, 1.0),
            track: hsla(240.0, 0.04, 0.26, 1.0),
        }
    }

    pub fn light() -> Self {
        Self {
            window: hsla(0.0, 0.0, 1.0, 0.99),
            surface: hsla(240.0, 0.02, 0.96, 1.0),
            surface_raised: hsla(0.0, 0.0, 1.0, 1.0),
            border: hsla(240.0, 0.03, 0.84, 1.0),
            border_strong: hsla(240.0, 0.04, 0.70, 1.0),
            text: hsla(240.0, 0.10, 0.10, 1.0),
            text_muted: hsla(240.0, 0.04, 0.40, 1.0),
            accent: hsla(24.0, 0.90, 0.48, 1.0),
            danger: hsla(352.0, 0.72, 0.45, 1.0),
            warning: hsla(38.0, 0.88, 0.40, 1.0),
            field: hsla(0.0, 0.0, 1.0, 1.0),
            track: hsla(240.0, 0.03, 0.90, 1.0),
        }
    }
}
pub fn palette_for(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette::dark(),
        Theme::Light => Palette::light(),
        Theme::System => {
            if system_prefers_light() {
                Palette::light()
            } else {
                Palette::dark()
            }
        }
    }
}

/// A translucent version of a palette color. `Hsla::fade_out` mutates, so
/// backgrounds need their own copy.
pub fn tint(color: Hsla, opacity: f32) -> Hsla {
    let mut tinted = color;
    tinted.fade_out(1.0 - opacity);
    tinted
}

/// `HKCU\...\Themes\Personalize\AppsUseLightTheme`. Absent or unreadable means
/// dark, which is the documented default.
fn system_prefers_light() -> bool {
    use windows::core::PCWSTR;

    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    unsafe {
        let mut handle = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ,
            &mut handle,
        )
        .is_err()
        {
            return false;
        }
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let read = RegGetValueW(
            handle,
            PCWSTR::null(),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        );
        let _ = RegCloseKey(handle);
        read.is_ok() && data != 0
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_resolves_to_a_palette() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            let palette = palette_for(theme);
            assert!(palette.text.a > 0.5, "text must be readable");
            assert!(palette.field.a > 0.5, "fields must be visible");
        }
    }

    #[test]
    fn explicit_themes_do_not_read_the_registry() {
        let dark = palette_for(Theme::Dark);
        assert_eq!(dark.text.l, Palette::dark().text.l);
        let light = palette_for(Theme::Light);
        assert_eq!(light.text.l, Palette::light().text.l);
    }
}
