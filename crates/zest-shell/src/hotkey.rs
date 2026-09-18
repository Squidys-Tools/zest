//! Global hotkey listener. Default `Shift+F`; second hotkey jumps to Convert ring.

use anyhow::Result;

pub const DEFAULT_HOTKEY: &str = "Shift+F";

pub fn register(hotkey_str: &str) -> Result<u32> {
    // TODO(MVP-shell): parse `hotkey_str` into global_hotkey::HotKey + register.
    tracing::info!(hotkey = hotkey_str, "hotkey stub: registered (no-op)");
    Ok(1)
}
