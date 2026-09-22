//! Explorer selection resolver — the riskiest MVP piece (PRD §What to Watch Out For).
//!
//! Chain (to prove first): focused Explorer window → COM `ShellWindows` →
//! selected items → filesystem paths. Must handle Win11 tabbed Explorer,
//! Desktop as a special shell view, and reject virtual folders
//! (Recycle Bin, This PC) gracefully.
//!
//! Implemented: STA COM init → foreground HWND → focused-view native read
//! → `IShellWindows` + tab-strip disambiguation → `SelectedItems()` → paths.
//! Proven live on Win11 (single, mixed, empty, multi-tab both directions,
//! second window); see `scripts/verify-selection.ps1` and SQU-22.

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
    let p = path.trim().to_ascii_lowercase().replace('/', "\\");
    let leaf = p.rsplit('\\').next().unwrap_or(&p);

    p.starts_with("::")
        || p.starts_with("shell:::")
        || matches!(p.as_str(), "this pc" | "recycle bin")
        || matches!(leaf, "recycle bin" | "$recycle.bin")
}

/// Test seam: build a Selection from explicit paths (bypasses COM).
pub fn resolve_mock(paths: Vec<PathBuf>) -> Result<Selection, SelectionError> {
    let saw_items = !paths.is_empty();
    let paths = paths
        .into_iter()
        .filter(|path| path.to_str().map_or(true, |s| !is_virtual_folder(s)))
        .collect();
    finish_selection(saw_items, paths)
}

/// Real entry point used by `zest-app` on hotkey.
///
/// Chain: `CoInitializeEx(STA)` → `GetForegroundWindow` → selection from the
/// foreground Explorer window, in two tiers:
/// 1. Native focused-view read: the focused control lives in the tab the user
///    sees, so walk up from it to the enclosing `SHELLDLL_DefView` and read
///    that view's selection via `AccessibleObjectFromWindow` (`OBJID_NATIVEOM`)
///    → `IShellView::GetItemObject(SVGIO_SELECTION)` →
///    `IShellItem::GetDisplayName(SIGDN_FILESYSPATH)` per item. Exact where
///    the native object model is served; fail-closed to tier 2 otherwise
///    (E_FAIL on current Win11 builds).
/// 2. (SQU-20) Automation over `IShellWindows` entries matching the foreground
///    HWND. Win11 keeps one entry per tab under a single HWND, so with several
///    candidates the visible tab is picked via the UI Automation tab strip
///    (selected `TabItem` index, verified against the entry's folder leaf
///    name; any drift falls back to enumeration order), then
///    `IWebBrowser.Document` → `IShellFolderViewDual2::SelectedItems()` →
///    `FolderItem.Path` per item.
///
/// Notes:
/// - Desktop (SQU-23): the Desktop has no Explorer HWND and never appears in
///   `IShellWindows`; when the foreground window is the desktop itself
///   (`Progman`/`WorkerW` host, or a `SHELLDLL_DefView`/`SysListView32` child
///   of one) selection is read from the desktop's own `SHELLDLL_DefView`
///   (`Progman` first, then any top-level `WorkerW` fallback) via the same
///   native `IShellView` → `SVGIO_SELECTION` read as tier 1.
/// - Virtual folders (SQU-24): `::`-style paths are dropped; when nothing
///   mappable remains we return `VirtualFolder`.
/// - `ZEST_SELECTION_DIAG=1` enables stderr diagnostics (serving tier,
///   tab strip, candidate folders) for per-release re-verification.
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
    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellWindows, IWebBrowserApp, ShellWindows};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    struct ComGuard {
        needs_uninit: bool,
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.needs_uninit {
                unsafe { CoUninitialize() };
            }
        }
    }

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(SelectionError::Com(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        // S_OK and S_FALSE are both success; balance either with CoUninitialize.
        let _guard = ComGuard {
            needs_uninit: hr.is_ok(),
        };

        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return Err(SelectionError::NoExplorerWindow);
        }
        let foreground_id = foreground.0 as isize;
        let diag = std::env::var_os("ZEST_SELECTION_DIAG").is_some();

        // Desktop (SQU-23): no Explorer HWND and no ShellWindows entry, so
        // handle it before the Explorer tiers. Only when the foreground
        // window *is* the desktop — never as a fallback for a focused
        // Explorer window with an empty/failed selection.
        if foreground_is_desktop(foreground) {
            if diag {
                eprintln!("[diag] foreground is desktop; reading Progman/WorkerW view");
            }
            return selected_paths_from_desktop(diag);
        }

        // Tier 1: selection from the focused (visible) tab's own view.
        // A definitive answer (paths, Empty, VirtualFolder) wins outright;
        // only a hard COM failure falls through to tier 2.
        match selected_paths_from_focused_view(foreground) {
            tier1 @ (Ok(_) | Err(SelectionError::Empty) | Err(SelectionError::VirtualFolder)) => {
                if diag {
                    eprintln!("[diag] served by TIER1-focused-view: {tier1:?}");
                }
                return tier1;
            }
            Err(e) => {
                if diag {
                    eprintln!("[diag] tier1 failed ({e}); trying automation fallback");
                }
            }
        }

        // Tier 2: automation fallback over HWND-matching ShellWindows entries.
        // Skips non-Explorer entries (e.g. Internet Explorer windows).
        let shell_windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)
            .map_err(|e| {
                SelectionError::Com(format!("CoCreateInstance(ShellWindows) failed: {e}"))
            })?;

        let count = shell_windows
            .Count()
            .map_err(|e| SelectionError::Com(format!("IShellWindows::Count failed: {e}")))?;

        // Collect every entry whose HWND matches the foreground window.
        // Skips non-Explorer entries (e.g. Internet Explorer windows).
        let mut candidates: Vec<windows::Win32::System::Com::IDispatch> = Vec::new();
        for i in 0..count {
            let index = variant_i4(i);
            let dispatch = match shell_windows.Item(&index) {
                Ok(d) => d,
                Err(_) => continue, // window went away; try the next one
            };

            let browser_app: IWebBrowserApp = match dispatch.cast() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let window_hwnd = match browser_app.HWND() {
                Ok(h) => h.0,
                Err(_) => continue,
            };
            if window_hwnd != foreground_id {
                continue;
            }
            candidates.push(dispatch);
        }

        // No ShellWindows entry matched the foreground HWND. This covers
        // "no Explorer focused" and the Desktop (no Explorer HWND — SQU-23).
        if candidates.is_empty() {
            return Err(SelectionError::NoExplorerWindow);
        }

        // Win11 hosts one tab per ShellWindows entry under a single HWND, so
        // the first match is not necessarily the visible tab. Ask the tab
        // strip which tab is selected and try that entry first; the
        // folder-name cross-check guards against entry/tab order drift and
        // falls back to enumeration order on any mismatch.
        let mut order: Vec<usize> = (0..candidates.len()).collect();
        if candidates.len() > 1 {
            if let Some(preferred) = preferred_tab_entry(foreground, &candidates, diag) {
                order.sort_by_key(|&i| (i != preferred) as u8);
            }
        }

        let mut last_error: Option<SelectionError> = None;
        for (n, &i) in order.iter().enumerate() {
            let dispatch = &candidates[i];
            if diag {
                let folder =
                    automation_folder_path(dispatch).unwrap_or_else(|e| format!("<err:{e}>"));
                eprintln!("[diag] candidate {n} (entry {i}): folder={folder}");
            }

            match selected_paths_from_automation(dispatch) {
                tier2 @ (Ok(_)
                | Err(SelectionError::Empty)
                | Err(SelectionError::VirtualFolder)) => {
                    if diag {
                        eprintln!(
                            "[diag] candidate {n} (entry {i}) served by TIER2-automation: {tier2:?}"
                        );
                    }
                    return tier2;
                }
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        match last_error {
            Some(e) => Err(e),
            None => Err(SelectionError::NoExplorerWindow),
        }
    }
}

/// Index of the visible tab's ShellWindows entry, if determinable (SQU-21).
///
/// Reads the frame's tab strip over UI Automation: the selected `TabItem`
/// index maps onto the HWND-matching entries, after verifying the entry's
/// folder leaf name matches the tab name (case-insensitive) so entry/tab
/// order drift degrades to enumeration order instead of a wrong tab.
/// Returns `None` when there is no tab strip (single-tab/Win10), no
/// selection, or any mismatch — callers then keep enumeration order.
#[cfg(windows)]
fn preferred_tab_entry(
    frame: windows::Win32::Foundation::HWND,
    candidates: &[windows::Win32::System::Com::IDispatch],
    diag: bool,
) -> Option<usize> {
    use std::path::Path;

    let (selected, names) = uia_tab_strip(frame)?;
    if diag {
        eprintln!("[diag] tab strip: selected={selected} names={names:?}");
    }
    if selected >= candidates.len() || selected >= names.len() {
        return None;
    }
    let folder = automation_folder_path(&candidates[selected]).ok()?;
    let leaf = Path::new(&folder)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if leaf.eq_ignore_ascii_case(&names[selected]) {
        Some(selected)
    } else {
        if diag {
            eprintln!(
                "[diag] tab strip: entry/tab mismatch (entry folder leaf={leaf:?}); keeping order"
            );
        }
        None
    }
}

/// Selected index + names of the frame's tab strip via UI Automation.
///
/// Finds `TabItem` elements under the frame and reads each one's
/// `SelectionItemPattern::IsSelected` and `Name`. `None` when the UIA walk
/// fails or nothing reports selected (e.g. no tab strip on Win10).
#[cfg(windows)]
fn uia_tab_strip(frame: windows::Win32::Foundation::HWND) -> Option<(usize, Vec<String>)> {
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, TreeScope_Descendants, UIA_ControlTypePropertyId,
        UIA_SelectionItemPatternId, UIA_TabItemControlTypeId,
    };

    unsafe {
        let automation: windows::Win32::UI::Accessibility::IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()?;
        let root = automation.ElementFromHandle(frame).ok()?;
        let condition = automation
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &variant_i4(UIA_TabItemControlTypeId.0),
            )
            .ok()?;
        let tabs = root.FindAll(TreeScope_Descendants, &condition).ok()?;
        let len = tabs.Length().ok()?;
        if len <= 0 {
            return None;
        }
        let mut names = Vec::with_capacity(len as usize);
        let mut selected = None;
        for i in 0..len {
            let el = tabs.GetElement(i).ok()?;
            names.push(el.CurrentName().ok()?.to_string());
            if selected.is_none() {
                if let Ok(pattern) = el.GetCurrentPatternAs::<
                    windows::Win32::UI::Accessibility::IUIAutomationSelectionItemPattern,
                >(UIA_SelectionItemPatternId)
                {
                    if let Ok(is) = pattern.CurrentIsSelected() {
                        if is.as_bool() {
                            selected = Some(i as usize);
                        }
                    }
                }
            }
        }
        Some((selected?, names))
    }
}

/// Read the selection from the focused view of the foreground frame (SQU-21).
///
/// The focused control lives in the tab the user sees, so the enclosing
/// `SHELLDLL_DefView` of the focus window *is* the visible tab — even when
/// Win11 hosts several tabs (and ShellWindows entries) under one HWND.
/// `AccessibleObjectFromWindow(OBJID_NATIVEOM)` yields that view's
/// `IShellView`; selection comes from `GetItemObject(SVGIO_SELECTION)`.
/// Returns `Com` when focus sits outside any file view (address bar, …) so
/// callers fall back to automation.
#[cfg(windows)]
fn selected_paths_from_focused_view(
    frame: windows::Win32::Foundation::HWND,
) -> Result<Selection, SelectionError> {
    use windows::core::Interface;
    use windows::Win32::UI::Shell::IShellView;

    unsafe {
        let defview = focused_defview(frame).ok_or_else(|| {
            SelectionError::Com("no focused file view under the foreground window".to_string())
        })?;
        let diag = std::env::var_os("ZEST_SELECTION_DIAG").is_some();
        if diag {
            eprintln!("[diag] focused defview=0x{:X}", defview.0 as usize);
        }

        // Attempt 1: the view object itself as IShellView.
        match accessible_object::<IShellView>(defview) {
            Ok(view) => {
                if diag {
                    eprintln!("[diag] focused tab: native OM is IShellView");
                }
                return selected_paths_from_shell_view(&view, "focused tab");
            }
            Err(e) => {
                if diag {
                    eprintln!("[diag] focused tab: IShellView OM failed: {e}");
                }
            }
        }

        // Attempt 2: the view's automation object (its own tab's
        // IShellFolderViewDual2) — same SelectedItems logic as the fallback,
        // but anchored at the focused DefView instead of an entry guess.
        match accessible_object::<windows::Win32::System::Com::IDispatch>(defview) {
            Ok(dispatch) => {
                use windows::Win32::UI::Shell::IShellFolderViewDual2;

                match dispatch.cast::<IShellFolderViewDual2>() {
                    Ok(view) => {
                        if diag {
                            eprintln!("[diag] focused tab: native OM is automation view");
                        }
                        return selected_paths_from_view(&view);
                    }
                    Err(e) => {
                        if diag {
                            eprintln!("[diag] focused tab: automation cast failed: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                if diag {
                    eprintln!("[diag] focused tab: IDispatch OM failed: {e}");
                }
            }
        }

        // Attempt 3: service provider scoped to the focused DefView — any
        // browser reached from here belongs to the visible tab.
        match accessible_object::<windows::Win32::System::Com::IServiceProvider>(defview) {
            Ok(provider) => {
                use windows::Win32::UI::Shell::{IShellBrowser, SID_STopLevelBrowser};

                match provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser) {
                    Ok(browser) => match browser.QueryActiveShellView() {
                        Ok(view) => {
                            if diag {
                                eprintln!("[diag] focused tab: OM via IServiceProvider");
                            }
                            return selected_paths_from_shell_view(&view, "focused tab");
                        }
                        Err(e) => {
                            if diag {
                                eprintln!("[diag] focused tab: provider QueryActiveShellView: {e}");
                            }
                        }
                    },
                    Err(e) => {
                        if diag {
                            eprintln!("[diag] focused tab: provider QueryService: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                if diag {
                    eprintln!("[diag] focused tab: IServiceProvider OM failed: {e}");
                }
            }
        }

        Err(SelectionError::Com(
            "focused tab: native object model unavailable".to_string(),
        ))
    }
}

/// Read `SVGIO_SELECTION` from a resolved shell view into paths.
#[cfg(windows)]
fn selected_paths_from_shell_view(
    view: &windows::Win32::UI::Shell::IShellView,
    what: &str,
) -> Result<Selection, SelectionError> {
    use windows::core::Interface;
    use windows::Win32::UI::Shell::{IFolderView, SIGDN_FILESYSPATH, SVGIO_SELECTION};

    unsafe {
        // Fail closed: the focused tab must be a folder view to yield paths.
        let _folder_view: IFolderView = view
            .cast()
            .map_err(|e| SelectionError::Com(format!("{what}: not a folder view: {e}")))?;
        let array = view
            .GetItemObject::<windows::Win32::UI::Shell::IShellItemArray>(SVGIO_SELECTION)
            .map_err(|e| SelectionError::Com(format!("{what}: selection unavailable: {e}")))?;
        let count = array.GetCount().map_err(|e| {
            SelectionError::Com(format!("{what}: IShellItemArray::GetCount failed: {e}"))
        })?;
        if count == 0 {
            return Err(SelectionError::Empty);
        }

        let mut saw_items = false;
        let mut paths: Vec<PathBuf> = Vec::new();
        for i in 0..count {
            let item = match array.GetItemAt(i) {
                Ok(item) => item,
                Err(_) => continue,
            };
            saw_items = true;

            // Virtual items (Recycle Bin, This PC, …) have no filesystem
            // path, so GetDisplayName fails for them — that is filtering,
            // not an error.
            let name = match item.GetDisplayName(SIGDN_FILESYSPATH) {
                Ok(name) => name,
                Err(_) => continue,
            };
            let path = pwstr_to_string_and_free(name);
            let Some(path) = path else { continue };
            if path.trim().is_empty() || is_virtual_folder(&path) {
                continue;
            }
            paths.push(PathBuf::from(path));
        }

        finish_selection(saw_items, paths)
    }
}

/// The `SHELLDLL_DefView` enclosing the focused control, if any.
///
/// Walks up from the focus window of the frame's GUI thread; the first
/// `SHELLDLL_DefView` ancestor is the visible tab's view. Returns `None`
/// when focus sits outside every file view (address bar, search box, …).
#[cfg(windows)]
fn focused_defview(
    frame: windows::Win32::Foundation::HWND,
) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetGUIThreadInfo, GetParent, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let tid = GetWindowThreadProcessId(frame, None);
        if tid == 0 {
            return None;
        }
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        GetGUIThreadInfo(tid, &mut info).ok()?;
        let mut hwnd = info.hwndFocus;
        if hwnd.is_invalid() {
            return None;
        }
        loop {
            if window_class_name(hwnd) == "SHELLDLL_DefView" {
                return Some(hwnd);
            }
            if hwnd == frame {
                return None;
            }
            hwnd = GetParent(hwnd).ok()?;
        }
    }
}

/// Window class name, e.g. `SHELLDLL_DefView`. Empty on failure.
#[cfg(windows)]
fn window_class_name(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

    unsafe {
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut buf);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// Read the selection from the desktop's own shell view (SQU-23).
///
/// The Desktop has no Explorer HWND and never appears in `IShellWindows`,
/// so callers route here only when the foreground window *is* the desktop
/// (see `foreground_is_desktop`). Selection comes from the desktop
/// `SHELLDLL_DefView` (`Progman` first, `WorkerW` fallback) via the same
/// native `IShellView` → `SVGIO_SELECTION` read as the focused tab.
#[cfg(windows)]
fn selected_paths_from_desktop(diag: bool) -> Result<Selection, SelectionError> {
    use windows::Win32::UI::Shell::IShellView;

    let defview = desktop_defview().ok_or_else(|| {
        SelectionError::Com("desktop: no SHELLDLL_DefView under Progman/WorkerW".to_string())
    })?;
    if diag {
        eprintln!("[diag] desktop defview=0x{:X}", defview.0 as usize);
    }
    let view: IShellView = accessible_object(defview).map_err(|e| {
        SelectionError::Com(format!("desktop: native object model unavailable: {e}"))
    })?;
    selected_paths_from_shell_view(&view, "desktop")
}

/// Whether the foreground window is the desktop itself.
///
/// True when the window or any ancestor up to the top level is a `Progman`
/// or `WorkerW` host. Desktop icon views (`SHELLDLL_DefView`,
/// `SysListView32`) report their host on the walk up, so a click on a
/// desktop icon still counts as desktop foreground.
#[cfg(windows)]
fn foreground_is_desktop(foreground: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetParent;

    unsafe {
        let mut hwnd = foreground;
        loop {
            let class = window_class_name(hwnd);
            if class == "Progman" || class == "WorkerW" {
                return true;
            }
            if hwnd == windows::Win32::Foundation::HWND(std::ptr::null_mut()) {
                return false;
            }
            match GetParent(hwnd) {
                Ok(parent) => {
                    if parent.is_invalid() || parent == hwnd {
                        return false;
                    }
                    hwnd = parent;
                }
                Err(_) => return false,
            }
        }
    }
}

/// The desktop's `SHELLDLL_DefView`: `Progman` child first, then any
/// top-level `WorkerW` fallback (Win11 hosts wallpaper in `WorkerW`; the
/// icon view stays under `Progman`, but both are checked).
#[cfg(windows)]
fn desktop_defview() -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW};

    unsafe {
        if let Ok(progman) = FindWindowW(w!("Progman"), None) {
            if !progman.is_invalid() {
                if let Ok(defview) =
                    FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None)
                {
                    if !defview.is_invalid() {
                        return Some(defview);
                    }
                }
            }
        }
        find_workerw_defview()
    }
}

/// First `SHELLDLL_DefView` found under any top-level `WorkerW`.
#[cfg(windows)]
fn find_workerw_defview() -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::w;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW};

    struct Ctx {
        found: Option<HWND>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        if window_class_name(hwnd) == "WorkerW" {
            if let Ok(defview) = FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) {
                if !defview.is_invalid() {
                    ctx.found = Some(defview);
                    return BOOL(0); // stop enumeration
                }
            }
        }
        BOOL(1) // continue enumeration
    }

    unsafe {
        let mut ctx = Ctx { found: None };
        let _ = EnumWindows(Some(visit), LPARAM(&mut ctx as *mut Ctx as isize));
        ctx.found
    }
}
/// A DefView window's native object model via oleacc.
///
/// `OBJID_NATIVEOM` is defined in oleacc.h but not exposed by windows-rs
/// 0.61, hence the local constant.
#[cfg(windows)]
fn accessible_object<T: windows::core::Interface>(
    defview: windows::Win32::Foundation::HWND,
) -> Result<T, SelectionError> {
    use windows::Win32::UI::Accessibility::AccessibleObjectFromWindow;

    const OBJID_NATIVEOM: u32 = 0xFFFFFFF0;

    unsafe {
        let mut obj = std::ptr::null_mut();
        AccessibleObjectFromWindow(defview, OBJID_NATIVEOM, &T::IID, &mut obj)
            .map_err(|e| SelectionError::Com(format!("AccessibleObjectFromWindow failed: {e}")))?;
        if obj.is_null() {
            return Err(SelectionError::Com(
                "AccessibleObjectFromWindow returned null".to_string(),
            ));
        }
        // COM interfaces are transparent pointer wrappers.
        Ok(std::mem::transmute_copy(&obj))
    }
}

/// Folder path of one ShellWindows entry via automation.
///
/// Used by the tab-strip cross-check (`preferred_tab_entry`) and the
/// `ZEST_SELECTION_DIAG` trace.
#[cfg(windows)]
fn automation_folder_path(
    dispatch: &windows::Win32::System::Com::IDispatch,
) -> Result<String, SelectionError> {
    use windows::core::Interface;
    use windows::Win32::UI::Shell::{Folder2, IShellFolderViewDual2, IWebBrowser};

    unsafe {
        let browser: IWebBrowser = dispatch
            .cast()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        let document = browser
            .Document()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        let view: IShellFolderViewDual2 = document
            .cast()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        let folder = view
            .Folder()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        let folder2: Folder2 = folder
            .cast()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        let self_item = folder2
            .Self_()
            .map_err(|e| SelectionError::Com(format!("{e}")))?;
        Ok(self_item
            .Path()
            .map_err(|e| SelectionError::Com(format!("{e}")))?
            .to_string())
    }
}

/// Convert a `PWSTR` from `IShellItem::GetDisplayName` into a `String`,
/// freeing the COM-allocated buffer on every path.
#[cfg(windows)]
unsafe fn pwstr_to_string_and_free(name: windows::core::PWSTR) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;

    let result = unsafe { name.to_string().ok() };
    unsafe { CoTaskMemFree(Some(name.0 as *const core::ffi::c_void)) };
    result
}

/// Read the selection via the automation `Document` of an HWND-matched
/// Explorer window (SQU-20 fallback tier).
#[cfg(windows)]
fn selected_paths_from_automation(
    dispatch: &windows::Win32::System::Com::IDispatch,
) -> Result<Selection, SelectionError> {
    use windows::core::Interface;
    use windows::Win32::UI::Shell::{IShellFolderViewDual2, IWebBrowser};

    unsafe {
        let browser: IWebBrowser = dispatch.cast().map_err(|e| {
            SelectionError::Com(format!("matched Explorer window has no IWebBrowser: {e}"))
        })?;
        let document = browser
            .Document()
            .map_err(|e| SelectionError::Com(format!("Explorer Document unavailable: {e}")))?;
        let view: IShellFolderViewDual2 = document.cast().map_err(|e| {
            SelectionError::Com(format!("Explorer view is not a shell folder view: {e}"))
        })?;
        selected_paths_from_view(&view)
    }
}

/// Build a `VARIANT` of type `VT_I4` for `IShellWindows::Item` /
/// `FolderItems::Item` index arguments.
#[cfg(windows)]
fn variant_i4(value: i32) -> windows::Win32::System::Variant::VARIANT {
    use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};

    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { lVal: value },
            }),
        },
    }
}

/// Read `SelectedItems()` from an already-matched shell view into paths.
///
/// Drops `::`-prefixed virtual items; returns `VirtualFolder` when items
/// existed but nothing mappable remained, `Empty` when nothing was selected.
#[cfg(windows)]
fn selected_paths_from_view(
    view: &windows::Win32::UI::Shell::IShellFolderViewDual2,
) -> Result<Selection, SelectionError> {
    unsafe {
        let items = view
            .SelectedItems()
            .map_err(|e| SelectionError::Com(format!("SelectedItems() failed: {e}")))?;
        let count = items
            .Count()
            .map_err(|e| SelectionError::Com(format!("FolderItems::Count failed: {e}")))?;
        if count <= 0 {
            return Err(SelectionError::Empty);
        }

        let mut saw_items = false;
        let mut paths: Vec<PathBuf> = Vec::new();
        for i in 0..count {
            let item = match items.Item(&variant_i4(i)) {
                Ok(item) => item,
                Err(_) => continue,
            };
            saw_items = true;

            // Prefer filesystem-backed items; fall back to Path filtering so
            // a failing IsFileSystem never hides a real selection.
            if let Ok(is_fs) = item.IsFileSystem() {
                if is_fs.0 == 0 {
                    continue;
                }
            }
            let path = match item.Path() {
                Ok(p) => p.to_string(),
                Err(_) => continue,
            };
            if path.trim().is_empty() || is_virtual_folder(&path) {
                continue;
            }
            paths.push(PathBuf::from(path));
        }

        finish_selection(saw_items, paths)
    }
}

/// Map filtered paths onto the selection contract shared by both tiers:
/// paths win; items that existed but left nothing mappable mean a virtual
/// folder; no items at all mean an empty selection.
fn finish_selection(saw_items: bool, paths: Vec<PathBuf>) -> Result<Selection, SelectionError> {
    if !paths.is_empty() {
        return Ok(Selection::new(paths));
    }
    if saw_items {
        return Err(SelectionError::VirtualFolder);
    }
    Err(SelectionError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_virtual_folder_paths_without_rejecting_real_names() {
        for path in [
            "::{645FF040-5081-101B-9F08-00AA002F954E}",
            "shell:::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
            "This PC",
            "Recycle Bin",
            r"C:\Users\me\Desktop\$Recycle.Bin",
        ] {
            assert!(is_virtual_folder(path), "expected virtual: {path}");
        }

        for path in [
            r"C:\Users\me\recycle-notes.txt",
            r"C:\Users\me\recycle\notes.txt",
            r"C:\Users\me\photo.png",
        ] {
            assert!(!is_virtual_folder(path), "expected filesystem path: {path}");
        }
    }

    #[test]
    fn resolve_mock_filters_virtual_items_but_keeps_files() {
        let files = resolve_mock(vec![
            PathBuf::from("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            PathBuf::from(r"C:\Users\me\photo.png"),
        ])
        .unwrap()
        .files;
        assert_eq!(files, vec![PathBuf::from(r"C:\Users\me\photo.png")]);

        assert!(matches!(
            resolve_mock(vec![PathBuf::from("This PC")]),
            Err(SelectionError::VirtualFolder)
        ));
        assert!(matches!(resolve_mock(vec![]), Err(SelectionError::Empty)));
    }

    #[test]
    fn finish_selection_maps_paths_first() {
        let sel = finish_selection(true, vec![PathBuf::from(r"C:\a.png")]).unwrap();
        assert_eq!(sel.len(), 1);

        // Items existed but nothing mappable remained → virtual folder.
        assert!(matches!(
            finish_selection(true, vec![]),
            Err(SelectionError::VirtualFolder)
        ));
        // Nothing selected at all → empty.
        assert!(matches!(
            finish_selection(false, vec![]),
            Err(SelectionError::Empty)
        ));
    }
}
