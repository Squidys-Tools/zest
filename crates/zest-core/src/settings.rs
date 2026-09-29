//! Persisted settings (PRD §Settings). Stored at `%LOCALAPPDATA%\Zest\settings.json`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

pub const DEFAULT_HOTKEY: &str = "Shift+F";
pub const DEFAULT_CONVERT_HOTKEY: &str = "Shift+C";
/// The platform UI font. Resolved by the text stack, so it tracks the OS
/// instead of naming a family that only exists on some Windows builds.
pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputLocation {
    BesideOriginal,
    Folder(PathBuf),
    AskEachTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VideoPreset {
    Low,
    #[default]
    Medium,
    High,
    Lossless,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum UpdateFrequency {
    Daily,
    #[default]
    Weekly,
    Never,
}

/// An 8-bit RGB color. Parsed once, at the settings-file boundary, so nothing
/// downstream has to defend against a half-typed `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// `#rrggbb` is malformed.
#[derive(Debug, thiserror::Error)]
#[error("not a #rrggbb color: {0:?}")]
pub struct InvalidHexColor(pub String);

impl HexColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#rrggbb`, the on-disk and display form.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Components in 0..=1, for Direct2D and DirectWrite.
    pub fn to_unit_rgb(self) -> (f32, f32, f32) {
        (
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        )
    }

    /// From HSV, the space the picker edits in. `h` in degrees, `s`/`v` in 0..=1.
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(360.0);
        let s = s.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u16 / 60 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let to_byte = |channel: f32| ((channel + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(to_byte(r), to_byte(g), to_byte(b))
    }

    /// Back to HSV, so reopening a swatch starts where the color actually is.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let (r, g, b) = self.to_unit_rgb();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        (h, if max == 0.0 { 0.0 } else { delta / max }, max)
    }
}

impl FromStr for HexColor {
    type Err = InvalidHexColor;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let digits = value.strip_prefix('#').unwrap_or(value);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InvalidHexColor(value.to_string()));
        }
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).expect("checked hex digits");
        Ok(Self::new(byte(0), byte(2), byte(4)))
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for HexColor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// The active sector gradient: 1–3 colors, first to last.
///
/// The length is an invariant of the type, so no consumer re-checks it, and the
/// color is typed, so nothing has to parse hex defensively. On disk it is still
/// a list of `#rrggbb` strings, so existing `settings.json` files load unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gradient {
    colors: Vec<HexColor>,
}

pub const MAX_GRADIENT_COLORS: usize = 3;
/// Tangerine nod: warm orange → pink.
pub const DEFAULT_GRADIENT: [HexColor; 2] = [
    HexColor::new(0xff, 0x8a, 0x3d),
    HexColor::new(0xff, 0x4d, 0x6d),
];

impl Default for Gradient {
    fn default() -> Self {
        Self {
            colors: DEFAULT_GRADIENT.to_vec(),
        }
    }
}

impl Gradient {
    /// Trims to at most 3 colors; an empty list becomes the default.
    pub fn new(colors: Vec<HexColor>) -> Self {
        let mut colors = colors;
        colors.truncate(MAX_GRADIENT_COLORS);
        if colors.is_empty() {
            return Self::default();
        }
        Self { colors }
    }

    /// Always 1–3 colors, in paint order.
    pub fn colors(&self) -> &[HexColor] {
        &self.colors
    }

    /// Appends a color; reports false when the gradient is already full.
    pub fn push(&mut self, color: HexColor) -> bool {
        if self.colors.len() >= MAX_GRADIENT_COLORS {
            return false;
        }
        self.colors.push(color);
        true
    }

    /// Removes one color, never leaving the gradient empty.
    pub fn remove(&mut self, index: usize) -> Option<HexColor> {
        if self.colors.len() <= 1 || index >= self.colors.len() {
            return None;
        }
        Some(self.colors.remove(index))
    }

    /// Replaces the color at `index`; ignores an out-of-range index.
    pub fn set(&mut self, index: usize, color: HexColor) {
        if let Some(slot) = self.colors.get_mut(index) {
            *slot = color;
        }
    }
}

impl Serialize for Gradient {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.colors.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Gradient {
    /// Lenient on purpose: a hand-edited or half-written `settings.json` should
    /// cost the user their gradient, not every other setting in the file.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw: Vec<String> = Vec::deserialize(deserializer)?;
        let colors: Vec<HexColor> = raw
            .iter()
            .filter_map(|value| value.parse().ok())
            .take(MAX_GRADIENT_COLORS)
            .collect();
        Ok(Self::new(colors))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// Record-style hotkey string. Default `Shift+F`.
    pub hotkey: String,
    pub output: OutputLocation,
    pub jpeg_quality: u8,
    pub video_preset: VideoPreset,
    pub audio_bitrate_kbps: u32,
    pub theme: Theme,
    /// Universal app font, by installed family name. `.SystemUIFont` is the
    /// sentinel for "whatever this platform's UI font is".
    pub font_family: String,
    pub gradient: Gradient,
    pub launch_at_startup: bool,
    pub update_frequency: UpdateFrequency,
    /// Gentle lossy→lossy warning toggle.
    pub warn_lossy_to_lossy: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            output: OutputLocation::BesideOriginal,
            jpeg_quality: 90,
            video_preset: VideoPreset::Medium,
            audio_bitrate_kbps: 192,
            theme: Theme::System,
            font_family: SYSTEM_UI_FONT.to_string(),
            gradient: Gradient::default(),
            launch_at_startup: false,
            update_frequency: UpdateFrequency::Weekly,
            warn_lossy_to_lossy: true,
        }
    }
}

impl Settings {
    pub fn config_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|b| b.data_local_dir().join("Zest").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("no local data dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors_in_both_forms() {
        assert_eq!("#ff8a3d".parse::<HexColor>().unwrap(), HexColor::new(255, 138, 61));
        assert_eq!("ff8a3d".parse::<HexColor>().unwrap(), HexColor::new(255, 138, 61));
        assert_eq!("#FF8A3D".parse::<HexColor>().unwrap(), HexColor::new(255, 138, 61));
        for bad in ["", "#fff", "#gggggg", "#ff8a3dd", "segoe"] {
            assert!(bad.parse::<HexColor>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn hex_round_trips_through_json_as_a_hash_string() {
        let color = HexColor::new(1, 2, 3);
        let json = serde_json::to_string(&color).unwrap();
        assert_eq!(json, "\"#010203\"");
        assert_eq!(serde_json::from_str::<HexColor>(&json).unwrap(), color);
    }

    #[test]
    fn hsv_round_trips_through_every_hue() {
        for step in 0..24 {
            let hue = step as f32 * 15.0;
            for (s, v) in [(1.0, 1.0), (0.5, 0.5), (0.8, 0.3)] {
                let color = HexColor::from_hsv(hue, s, v);
                let (h2, s2, v2) = color.to_hsv();
                assert!((h2 - hue).abs() < 2.0, "hue {hue} became {h2}");
                assert!((s2 - s).abs() < 0.02, "sat {s} became {s2}");
                assert!((v2 - v).abs() < 0.02, "val {v} became {v2}");
            }
        }
    }

    #[test]
    fn grayscale_has_no_saturation() {
        let (h, s, v) = HexColor::new(128, 128, 128).to_hsv();
        assert_eq!(h, 0.0);
        assert_eq!(s, 0.0);
        assert!((v - 128.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn gradient_keeps_one_to_three_colors_in_paint_order() {
        let mut gradient = Gradient::default();
        assert_eq!(gradient.colors().len(), 2);
        assert!(gradient.push(HexColor::new(0, 0, 0)));
        assert!(
            !gradient.push(HexColor::new(1, 1, 1)),
            "a fourth color is refused, not silently dropped"
        );
        assert_eq!(gradient.colors().len(), MAX_GRADIENT_COLORS);
        assert_eq!(gradient.remove(0), Some(HexColor::new(0xff, 0x8a, 0x3d)));
        assert_eq!(gradient.remove(0), Some(HexColor::new(0xff, 0x4d, 0x6d)));
        assert_eq!(gradient.remove(0), None, "the last color stays");
        assert_eq!(gradient.colors(), [HexColor::new(0, 0, 0)]);
    }

    #[test]
    fn an_empty_or_over_long_gradient_falls_back_to_the_default() {
        assert_eq!(Gradient::new(Vec::new()), Gradient::default());
        let too_long = vec![HexColor::new(0, 0, 0); MAX_GRADIENT_COLORS + 2];
        assert_eq!(Gradient::new(too_long).colors().len(), MAX_GRADIENT_COLORS);
    }

    #[test]
    fn a_bad_color_costs_the_gradient_not_the_whole_file() {
        let mut raw = serde_json::to_value(Settings::default()).unwrap();
        raw["gradient"] = serde_json::json!(["#ff8a3d", "nonsense", "#ff4d6d"]);
        raw["jpeg_quality"] = serde_json::json!(71);
        let settings: Settings = serde_json::from_value(raw).unwrap();
        assert_eq!(settings.gradient.colors().len(), 2);
        assert_eq!(settings.jpeg_quality, 71);
    }

    #[test]
    fn an_unreadable_gradient_entry_uses_the_default_ramp() {
        let gradient: Gradient = serde_json::from_str(r#"["oops"]"#).unwrap();
        assert_eq!(gradient, Gradient::default());
    }

    #[test]
    fn the_gradient_keeps_its_shape_across_a_save_and_load() {
        let mut settings = Settings::default();
        settings.gradient.push(HexColor::new(0x11, 0x22, 0x33));
        settings.gradient.set(0, HexColor::new(0x44, 0x55, 0x66));
        let round_tripped: Gradient =
            serde_json::from_str(&serde_json::to_string(&settings.gradient).unwrap()).unwrap();
        assert_eq!(round_tripped, settings.gradient);
    }
}
