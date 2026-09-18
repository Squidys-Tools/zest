//! Auto-update: checks GitHub Releases on startup + schedule (daily/weekly/never),
//! prompts to update on next restart. MSI is installed per-user (no elevation).

use anyhow::Result;
use zest_core::UpdateFrequency;

pub const RELEASES_URL: &str = "https://api.github.com/repos/OWNER/zest/releases/latest";

pub async fn check_now(current: &str) -> Result<Option<String>> {
    // TODO(MVP-package): reqwest fetch + semver compare; prompt in tray/settings.
    let _ = (current, RELEASES_URL);
    Ok(None)
}

pub fn should_check(freq: UpdateFrequency) -> bool {
    !matches!(freq, UpdateFrequency::Never)
}
