//! Tray icon (stub: builds the icon + menu model; window loop lands with `app`).

use anyhow::Result;

pub fn build() -> Result<()> {
    // TODO(MVP-shell): tray_icon::TrayIconBuilder with show/hide/settings/quit.
    tracing::info!("tray stub: icon would appear here");
    Ok(())
}
