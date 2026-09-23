//! Tray icon: Show / Settings / Quit on a dedicated Win32 message-loop thread.
//!
//! `tray-icon` dispatches menu and icon events from the message loop of the
//! thread that created the icon, so the icon, menu, and loop all live on one
//! `zest-tray` thread. User choices cross back to the app over an unbounded
//! tokio channel; `build()` only returns once the icon is up (or reports why
//! it is not).

use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// Menu item ids, matched in the `MenuEvent` handler.
const ID_SHOW: &str = "show";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

/// A user choice from the tray icon or its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Reveal the radial overlay for the current selection.
    Show,
    /// Open the settings window.
    Settings,
    /// Shut Zest down.
    Quit,
}

/// Spawn the tray thread (icon + menu + Win32 loop) and wait until it is
/// ready. Returns the receiver that delivers [`TrayAction`]s to the app.
pub fn build() -> Result<UnboundedReceiver<TrayAction>> {
    let (tx, rx) = unbounded_channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("zest-tray".into())
        .spawn(move || tray_thread(tx, ready_tx))
        .context("spawn tray thread")?;

    // The tray thread sends exactly once: Ok after the icon is built, Err if
    // setup failed. A disconnected channel means the thread died uncleanly.
    let ready = ready_rx
        .recv()
        .map_err(|_| anyhow!("tray thread exited before becoming ready"))?;
    ready?;
    Ok(rx)
}

/// Tray thread body: install event handlers, build icon + menu, then pump
/// Win32 messages until `WM_QUIT`.
fn tray_thread(tx: UnboundedSender<TrayAction>, ready: std::sync::mpsc::Sender<Result<()>>) {
    match build_tray(tx) {
        Ok(tray) => {
            let _ = ready.send(Ok(()));
            // Keep the icon alive for the lifetime of the loop.
            let _tray = tray;
            tracing::info!("tray ready (Show / Settings / Quit)");
            run_message_loop();
            tracing::info!("tray message loop exited");
        }
        Err(e) => {
            tracing::error!("tray setup failed: {e:#}");
            let _ = ready.send(Err(e));
        }
    }
}

/// Install handlers and build the menu + icon. The returned icon must be kept
/// alive by the caller for as long as the message loop runs.
fn build_tray(tx: UnboundedSender<TrayAction>) -> Result<tray_icon::TrayIcon> {
    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id().as_ref() {
            ID_SHOW => TrayAction::Show,
            ID_SETTINGS => TrayAction::Settings,
            ID_QUIT => TrayAction::Quit,
            other => {
                tracing::debug!(id = other, "unhandled tray menu id");
                return;
            }
        };
        let _ = menu_tx.send(action);
    }));

    TrayIconEvent::set_event_handler(Some(move |event| {
        // Left-click (menu on left click is off) reveals the overlay.
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = tx.send(TrayAction::Show);
        }
    }));

    let menu = build_menu()?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(tray_icon())
        .with_tooltip("Zest")
        .with_menu_on_left_click(false)
        .build()
        .context("create tray icon")?;
    Ok(tray)
}

/// Show / Settings / separator / Quit.
fn build_menu() -> Result<Menu> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id(ID_SHOW, "Show", true, None))
        .context("append Show")?;
    menu.append(&MenuItem::with_id(ID_SETTINGS, "Settings…", true, None))
        .context("append Settings")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("append separator")?;
    menu.append(&MenuItem::with_id(ID_QUIT, "Quit", true, None))
        .context("append Quit")?;
    Ok(menu)
}

/// Blocking Win32 message pump for the tray thread's window/menu messages.
#[cfg(windows)]
fn run_message_loop() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, MSG,
    };

    unsafe {
        let mut msg = MSG::default();
        loop {
            // >0 dispatched, 0 = WM_QUIT, -1 = error; both stop the loop.
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
fn run_message_loop() {
    tracing::warn!("tray message loop unsupported on this platform; tray is inert");
}

/// Procedural 32×32 tangerine disc so the scaffold ships without an asset.
fn tray_icon() -> Icon {
    let (rgba, w, h) = icon_rgba(32);
    Icon::from_rgba(rgba, w, h).expect("generated tray icon is valid RGBA")
}

fn icon_rgba(size: u32) -> (Vec<u8>, u32, u32) {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let center = (size as f64 - 1.0) / 2.0;
    let radius = center;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - center;
            let dy = y as f64 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            // 1px feather at the rim keeps the disc from looking jagged.
            let alpha = (radius - dist).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let i = ((y * size + x) * 4) as usize;
            pixels[i] = 0xff; // R — zest orange #ff8a3d
            pixels[i + 1] = 0x8a; // G
            pixels[i + 2] = 0x3d; // B
            pixels[i + 3] = (alpha * 255.0) as u8;
        }
    }
    (pixels, size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_rgba_is_full_buffer_with_center_pixel() {
        let (rgba, w, h) = icon_rgba(32);
        assert_eq!((w, h), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);

        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(&rgba[center..center + 3], &[0xff, 0x8a, 0x3d]);
        assert_eq!(rgba[center + 3], 255);

        // Corner is outside the disc → fully transparent.
        assert_eq!(rgba[3], 0);
    }

    #[test]
    fn menu_ids_match_handler_constants() {
        assert_eq!(ID_SHOW, "show");
        assert_eq!(ID_SETTINGS, "settings");
        assert_eq!(ID_QUIT, "quit");
    }
}
