//! Auto-update: checks GitHub Releases on a daily/weekly/never cadence and
//! surfaces "update available" as a toast plus a tray-menu link. Installing
//! the MSI is deliberately manual for now — the menu item opens the release
//! page. State (`last check`, `pending version`) lives in
//! `%LOCALAPPDATA%\Zest\update-state.json`, next to `settings.json`.
//!
//! The only network call is [`fetch_latest_tag`]; compare and cadence logic
//! are pure and unit-tested below.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use zest_core::{Settings, UpdateFrequency};

pub const RELEASES_URL: &str = "https://api.github.com/repos/Squidys-Tools/zest/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/Squidys-Tools/zest/releases/latest";

const DAY_SECS: u64 = 24 * 60 * 60;
const WEEK_SECS: u64 = 7 * DAY_SECS;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Persisted updater state. `serde(default)` keeps hand-edited and older
/// files loadable.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct UpdateState {
    /// Unix time of the last check attempt (attempts count, so a flaky
    /// network does not retry on every launch).
    last_check_unix: u64,
    /// A release newer than the running build, awaiting manual install.
    pending_version: Option<String>,
}

/// The cadence in seconds; `Never` has no interval and never checks.
fn interval(freq: UpdateFrequency) -> Option<u64> {
    match freq {
        UpdateFrequency::Daily => Some(DAY_SECS),
        UpdateFrequency::Weekly => Some(WEEK_SECS),
        UpdateFrequency::Never => None,
    }
}

/// Cadence gate: check when the interval has elapsed since `last_check`
/// (or when there has never been one). Pure — the clock is a parameter.
pub fn should_check(freq: UpdateFrequency, last_check: Option<u64>, now: u64) -> bool {
    let Some(interval) = interval(freq) else {
        return false;
    };
    match last_check {
        None => true,
        Some(last) => now.saturating_sub(last) >= interval,
    }
}

/// `latest` (a tag like `v1.2.3`) strictly newer than `current`. Anything
/// unparseable is not an update: a dev build should never nag, and a weird
/// tag should never install.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// Accept `1.2.3` and the `v1.2.3` tag form GitHub releases conventionally use.
fn parse_version(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version.trim().trim_start_matches(['v', 'V'])).ok()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `update-state.json` beside `settings.json`; `None` when there is no
/// per-user data dir (nothing can be persisted, checks still run).
fn state_path() -> Option<PathBuf> {
    Settings::config_path()?
        .parent()
        .map(|dir| dir.join("update-state.json"))
}

fn load_state() -> UpdateState {
    state_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(state: &UpdateState) {
    let Some(path) = state_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(state) {
        Ok(raw) => {
            if let Err(e) = std::fs::write(&path, raw) {
                tracing::warn!(path = %path.display(), "could not write update state: {e}");
            }
        }
        Err(e) => tracing::warn!("could not serialise update state: {e}"),
    }
}

/// A newer release waiting to be installed, if it is still newer than this
/// build — an upgraded build drops its stale pending entry.
pub fn pending_version() -> Option<String> {
    let pending = load_state().pending_version?;
    is_newer(env!("CARGO_PKG_VERSION"), &pending).then_some(pending)
}

/// The only network call. Kept tiny so the compare/cadence logic above can be
/// tested without touching GitHub.
async fn fetch_latest_tag(url: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .context("build http client")?;
    let release: Release = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("zest/", env!("CARGO_PKG_VERSION")),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("GET latest release")?
        .error_for_status()
        .context("latest release status")?
        .json()
        .await
        .context("parse latest release")?;
    Ok(release.tag_name)
}

/// One check attempt: honours the cadence, fetches, semver-compares, and
/// records both the attempt time and any pending version. When the cadence
/// gates the fetch out, a stale pending version still surfaces — the prompt
/// is on restart, not only on check day.
pub async fn check_for_updates(freq: UpdateFrequency, current: &str) -> Result<Option<String>> {
    let now = now_unix();
    let mut state = load_state();

    let last_check = (state.last_check_unix > 0).then_some(state.last_check_unix);
    if !should_check(freq, last_check, now) {
        return Ok(state
            .pending_version
            .filter(|pending| is_newer(current, pending)));
    }

    // Record the attempt before fetching so failures are throttled too.
    state.last_check_unix = now;
    let latest = match fetch_latest_tag(RELEASES_URL).await {
        Ok(tag) => tag,
        Err(e) => {
            save_state(&state);
            return Err(e);
        }
    };

    let newer = is_newer(current, &latest)
        .then(|| latest.trim().trim_start_matches(['v', 'V']).to_string());
    state.pending_version = newer.clone();
    save_state(&state);
    Ok(newer)
}

/// Open the releases page in the default browser (the tray "update" item).
#[cfg(windows)]
pub fn open_releases_page() -> Result<()> {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let url = HSTRING::from(RELEASES_PAGE);
    let result = unsafe {
        // w!("open") + a URL hands off to the default browser.
        ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL)
    };
    // The legacy API returns an instance handle; values ≤32 are error codes.
    anyhow::ensure!(
        (result.0 as usize) > 32,
        "ShellExecuteW failed with code {}",
        result.0 as usize
    );
    Ok(())
}

#[cfg(not(windows))]
pub fn open_releases_page() -> Result<()> {
    anyhow::bail!("opening URLs is only wired on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_semver_through_tag_noise() {
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "v0.2.0"), "same version is not newer");
        assert!(!is_newer("1.0.0", "v0.9.9"), "a downgrade is not an update");
        // Prereleases order below their release.
        assert!(is_newer("0.2.0-beta", "v0.2.0"));
        assert!(!is_newer("0.2.0", "v0.2.0-beta"));
    }

    #[test]
    fn is_newer_rejects_unparseable_versions() {
        assert!(!is_newer("0.1.0", "not-a-version"));
        assert!(!is_newer("dev-build", "v9.9.9"), "dev builds never nag");
    }

    #[test]
    fn should_check_honours_the_cadence() {
        const NOW: u64 = 1_000_000;
        // Never checked → check, for any non-Never cadence.
        assert!(should_check(UpdateFrequency::Daily, None, NOW));
        assert!(should_check(UpdateFrequency::Weekly, None, NOW));
        assert!(!should_check(UpdateFrequency::Never, None, NOW));
        // Inside the interval → skip.
        let hour_ago = NOW - 3600;
        assert!(!should_check(UpdateFrequency::Daily, Some(hour_ago), NOW));
        assert!(!should_check(UpdateFrequency::Weekly, Some(hour_ago), NOW));
        // Past the interval → check.
        let two_days_ago = NOW - 2 * DAY_SECS;
        assert!(should_check(
            UpdateFrequency::Daily,
            Some(two_days_ago),
            NOW
        ));
        assert!(!should_check(
            UpdateFrequency::Weekly,
            Some(two_days_ago),
            NOW
        ));
        let eight_days_ago = NOW - 8 * DAY_SECS;
        assert!(should_check(
            UpdateFrequency::Weekly,
            Some(eight_days_ago),
            NOW
        ));
        // Never never checks, however stale the record.
        assert!(!should_check(UpdateFrequency::Never, Some(0), NOW));
        // A clock set back does not underflow.
        assert!(!should_check(
            UpdateFrequency::Daily,
            Some(NOW + DAY_SECS),
            NOW
        ));
    }

    #[test]
    fn parse_version_accepts_bare_and_tag_forms() {
        assert_eq!(
            parse_version("v1.2.3"),
            semver::Version::parse("1.2.3").ok()
        );
        assert_eq!(parse_version("1.2.3"), semver::Version::parse("1.2.3").ok());
        assert!(parse_version("zest-1.2.3").is_none());
    }
}
