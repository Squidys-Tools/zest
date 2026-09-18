//! Toast notifications ("done" signal after background conversion).

use anyhow::Result;

pub fn notify(title: &str, body: &str) -> Result<()> {
    // TODO(MVP-polish): Win32_UI_Notifications toast via `windows` crate.
    tracing::info!(title, body, "toast stub");
    Ok(())
}
