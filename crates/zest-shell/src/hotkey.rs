//! Global hotkey listener. Default `Shift+F`; `Shift+C` jumps to Convert.

use anyhow::{anyhow, Context, Result};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
pub use zest_core::{DEFAULT_CONVERT_HOTKEY, DEFAULT_HOTKEY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    OpenMenu,
    OpenConvert,
}

pub struct HotkeyListener {
    receiver: UnboundedReceiver<HotkeyAction>,
    convert_unavailable_reason: Option<String>,
    #[cfg(windows)]
    thread_id: u32,
}

impl HotkeyListener {
    pub async fn recv(&mut self) -> Option<HotkeyAction> {
        self.receiver.recv().await
    }

    pub fn convert_unavailable_reason(&self) -> Option<&str> {
        self.convert_unavailable_reason.as_deref()
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::{
                Foundation::{LPARAM, WPARAM},
                UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT},
            };

            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn register(hotkey_str: &str) -> Result<HotkeyListener> {
    let (menu_hotkey, convert_hotkey) = parse_hotkeys(hotkey_str)?;

    let (action_tx, receiver) = unbounded_channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("zest-hotkey".into())
        .spawn(move || {
            let setup = (|| -> Result<(GlobalHotKeyManager, Option<String>)> {
                let manager = GlobalHotKeyManager::new().context("create global hotkey manager")?;
                manager
                    .register(menu_hotkey)
                    .context("register menu hotkey")?;
                let convert_error = manager
                    .register(convert_hotkey)
                    .err()
                    .map(|error| format!("{error:#}"));
                Ok((manager, convert_error))
            })();

            match setup {
                Ok((manager, convert_error)) => {
                    #[cfg(windows)]
                    let thread_id = current_thread_id();
                    #[cfg(not(windows))]
                    let thread_id = 0;
                    let convert_id = convert_error.is_none().then_some(convert_hotkey.id());
                    if ready_tx
                        .send(Ok((thread_id, convert_error.clone())))
                        .is_ok()
                    {
                        run_message_loop(manager, menu_hotkey.id(), convert_id, action_tx);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("{error:#}")));
                }
            }
        })
        .context("spawn global hotkey listener")?;

    let (thread_id, convert_unavailable_reason) = ready_rx
        .recv()
        .context("global hotkey listener exited before registering")?
        .map_err(anyhow::Error::msg)?;
    #[cfg(not(windows))]
    let _thread_id = thread_id;

    Ok(HotkeyListener {
        receiver,
        convert_unavailable_reason,
        #[cfg(windows)]
        thread_id,
    })
}

pub fn uses_default_hotkey(value: &str) -> bool {
    match (parse_hotkey(value), parse_hotkey(DEFAULT_HOTKEY)) {
        (Ok(value), Ok(default)) => value == default,
        _ => false,
    }
}

fn parse_hotkeys(menu_hotkey_str: &str) -> Result<(HotKey, HotKey)> {
    let menu_hotkey = parse_hotkey(menu_hotkey_str)?;
    let convert_hotkey = parse_hotkey(DEFAULT_CONVERT_HOTKEY)?;
    if menu_hotkey == convert_hotkey {
        return Err(anyhow!(
            "menu hotkey and Convert hotkey must be different: {menu_hotkey_str}"
        ));
    }
    Ok((menu_hotkey, convert_hotkey))
}

fn parse_hotkey(value: &str) -> Result<HotKey> {
    value
        .parse()
        .with_context(|| format!("invalid global hotkey: {value}"))
}

#[cfg(windows)]
fn current_thread_id() -> u32 {
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

fn action_for_event(
    event: GlobalHotKeyEvent,
    menu_id: u32,
    convert_id: Option<u32>,
) -> Option<HotkeyAction> {
    if event.state() != HotKeyState::Pressed {
        return None;
    }
    match event.id() {
        id if id == menu_id => Some(HotkeyAction::OpenMenu),
        id if Some(id) == convert_id => Some(HotkeyAction::OpenConvert),
        _ => None,
    }
}

#[cfg(windows)]
fn run_message_loop(
    manager: GlobalHotKeyManager,
    menu_id: u32,
    convert_id: Option<u32>,
    action_tx: tokio::sync::mpsc::UnboundedSender<HotkeyAction>,
) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, MSG,
    };

    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);

            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if let Some(action) = action_for_event(event, menu_id, convert_id) {
                    if action_tx.send(action).is_err() {
                        return;
                    }
                }
            }
        }
    }
    drop(manager);
}

#[cfg(not(windows))]
fn run_message_loop(
    manager: GlobalHotKeyManager,
    menu_id: u32,
    convert_id: Option<u32>,
    action_tx: tokio::sync::mpsc::UnboundedSender<HotkeyAction>,
) {
    while !action_tx.is_closed() {
        if let Ok(event) =
            GlobalHotKeyEvent::receiver().recv_timeout(std::time::Duration::from_millis(250))
        {
            if let Some(action) = action_for_event(event, menu_id, convert_id) {
                if action_tx.send(action).is_err() {
                    return;
                }
            }
        }
    }
    drop(manager);
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::Code;

    #[test]
    fn parses_record_style_default_hotkey() {
        let hotkey = parse_hotkey(DEFAULT_HOTKEY).unwrap();

        assert_eq!(hotkey.key, Code::KeyF);
        assert!(hotkey
            .mods
            .contains(global_hotkey::hotkey::Modifiers::SHIFT));
    }

    #[test]
    fn parses_convert_hotkey() {
        let hotkey = parse_hotkey(DEFAULT_CONVERT_HOTKEY).unwrap();

        assert_eq!(hotkey.key, Code::KeyC);
        assert!(hotkey
            .mods
            .contains(global_hotkey::hotkey::Modifiers::SHIFT));
    }

    #[test]
    fn rejects_invalid_hotkey_strings() {
        assert!(parse_hotkey("Shift+").is_err());
    }

    #[test]
    fn rejects_duplicate_menu_and_convert_hotkeys() {
        assert!(parse_hotkeys(DEFAULT_CONVERT_HOTKEY).is_err());
    }

    #[test]
    fn recognizes_the_default_menu_binding_independent_of_formatting() {
        assert!(uses_default_hotkey(DEFAULT_HOTKEY));
        assert!(uses_default_hotkey(" shift + f "));
        assert!(!uses_default_hotkey("Shift+G"));
        assert!(!uses_default_hotkey("invalid"));
    }

    #[test]
    fn maps_only_pressed_registered_hotkeys() {
        let menu = parse_hotkey(DEFAULT_HOTKEY).unwrap();
        let convert = parse_hotkey(DEFAULT_CONVERT_HOTKEY).unwrap();

        assert_eq!(
            action_for_event(
                GlobalHotKeyEvent {
                    id: menu.id(),
                    state: HotKeyState::Pressed,
                },
                menu.id(),
                Some(convert.id())
            ),
            Some(HotkeyAction::OpenMenu)
        );
        assert_eq!(
            action_for_event(
                GlobalHotKeyEvent {
                    id: convert.id(),
                    state: HotKeyState::Pressed,
                },
                menu.id(),
                Some(convert.id())
            ),
            Some(HotkeyAction::OpenConvert)
        );
        assert_eq!(
            action_for_event(
                GlobalHotKeyEvent {
                    id: menu.id(),
                    state: HotKeyState::Released,
                },
                menu.id(),
                Some(convert.id())
            ),
            None
        );
        assert_eq!(
            action_for_event(
                GlobalHotKeyEvent {
                    id: convert.id(),
                    state: HotKeyState::Pressed,
                },
                menu.id(),
                None
            ),
            None
        );
    }
}
