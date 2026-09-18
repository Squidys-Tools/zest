//! Launch-at-startup (opt-in): writes HKCU `...\Run\Zest` (no admin needed).

use anyhow::Result;

const RUN_VALUE: &str = "Zest";

pub fn set_enabled(enabled: bool) -> Result<()> {
    // TODO(MVP-shell): windows::Win32::System::Registry HKCU write/delete.
    tracing::info!(enabled, "{} startup stub", RUN_VALUE);
    Ok(())
}
