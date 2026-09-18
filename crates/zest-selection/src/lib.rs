//! Explorer selection resolver — the riskiest MVP piece (PRD §What to Watch Out For).
//!
//! Chain (to prove first): focused Explorer window → COM `ShellWindows` →
//! selected items → filesystem paths. Must handle Win11 tabbed Explorer,
//! Desktop as a special shell view, and reject virtual folders
//! (Recycle Bin, This PC) gracefully.
//!
//! Current status: COM skeleton that compiles; full item enumeration is
//! `TODO(MVP-1)` with the exact automation interfaces noted inline.

use std::path::PathBuf;
use thiserror::Error;
use zest_core::Selection;

#[derive(Debug, Error)]
pub enum SelectionError {
    #[error("no Explorer window is focused")]
    NoExplorerWindow,
    #[error("selection is a virtual folder (Recycle Bin, This PC, …) and has no filesystem paths")]
    VirtualFolder,
    #[error("COM automation unavailable: {0}")]
    Com(String),
    #[error("selection is empty")]
    Empty,
}

pub fn is_virtual_folder(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("::")
        || p.contains("recycle")
        || p == "this pc"
        || p.starts_with("shell:::")
}

/// Test seam: build a Selection from explicit paths (bypasses COM).
pub fn resolve_mock(paths: Vec<PathBuf>) -> Result<Selection, SelectionError> {
    if paths.is_empty() {
        return Err(SelectionError::Empty);
    }
    for p in &paths {
        if let Some(s) = p.to_str() {
            if is_virtual_folder(s) {
                return Err(SelectionError::VirtualFolder);
            }
        }
    }
    Ok(Selection::new(paths))
}

/// Real entry point used by `zest-app` on hotkey.
///
/// TODO(MVP-1): implement with `windows` crate:
/// 1. `CoInitializeEx(COINIT_APARTMENTTHREADED)`
/// 2. `IShellWindows` → iterate windows → match `HWND` from `GetForegroundWindow`
///    (Win11: one HWND hosts multiple tabs — pick the active tab's
///    `IShellBrowser` / `IFolderView`).
/// 3. Desktop: no Explorer HWND — query the `Progman`/`WorkerW` shell view instead.
/// 4. `Folder.SelectedItems()` → `FolderItem.Path` per item; drop `::`-prefixed
///    virtual items; return `VirtualFolder` if nothing mappable remains.
pub fn resolve() -> Result<Selection, SelectionError> {
    #[cfg(windows)]
    {
        windows_resolve()
    }
    #[cfg(not(windows))]
    {
        Err(SelectionError::Com(
            "Explorer COM is Windows-only".to_string(),
        ))
    }
}

#[cfg(windows)]
fn windows_resolve() -> Result<Selection, SelectionError> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    // Prove COM init works; enumeration lands in MVP-1.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // TODO(MVP-1): ShellWindows enumeration here.
        CoUninitialize();
    }
    Err(SelectionError::NoExplorerWindow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_virtual_folders() {
        assert!(is_virtual_folder("::{645FF040-5081-101B-9F08-00AA002F954E}"));
        assert!(resolve_mock(vec![]).is_err());
    }
}
