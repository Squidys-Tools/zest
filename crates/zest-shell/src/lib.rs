//! Background shell: tray icon, global hotkey, toast, startup, auto-update.
//!
//! Lives in the system tray; hotkey default `Shift+F` (record-style,
//! configurable in settings). AV note: global hooks can trip heuristics —
//! plan is EV code-signing + vendor whitelisting (PRD §What to Watch Out For).

pub mod hotkey;
pub mod startup;
pub mod toast;
pub mod tray;
pub mod updater;
