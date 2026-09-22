//! Shared, platform-independent types for Zest.
//!
//! No Windows dependencies here by design — everything is unit-testable.
//! See `docs/ARCHITECTURE.md`.

pub mod file_kind;
pub mod menu;
pub mod naming;
pub mod settings;

pub use file_kind::FileKind;
pub use menu::{categories_for_selection, convert_targets, ActionCategory};
pub use naming::unique_sibling_path;
pub use settings::{OutputLocation, Settings, Theme, UpdateFrequency, VideoPreset};

use std::path::PathBuf;

/// Files the user had selected when they pressed the hotkey.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub files: Vec<PathBuf>,
}

impl Selection {
    pub fn new(files: Vec<PathBuf>) -> Self {
        Self { files }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}
