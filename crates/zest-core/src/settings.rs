//! Persisted settings (PRD §Settings). Stored at `%LOCALAPPDATA%\Zest\settings.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// 1–3 gradient colors (hex `#rrggbb`). Active sector gradient.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gradient(pub Vec<String>);

impl Default for Gradient {
    fn default() -> Self {
        // Tangerine nod: warm orange → pink.
        Self(vec!["#ff8a3d".to_string(), "#ff4d6d".to_string()])
    }
}

impl Gradient {
    pub fn is_valid(&self) -> bool {
        (1..=3).contains(&self.0.len()) && self.0.iter().all(|c| c.len() == 7 && c.starts_with('#'))
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
    /// Universal app font. Default Segoe UI Variable (Win11).
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
            hotkey: "Shift+F".to_string(),
            output: OutputLocation::BesideOriginal,
            jpeg_quality: 90,
            video_preset: VideoPreset::Medium,
            audio_bitrate_kbps: 192,
            theme: Theme::System,
            font_family: "Segoe UI Variable".to_string(),
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
