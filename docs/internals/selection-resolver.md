# Selection resolver

The riskiest piece of Zest. Everything downstream (menu, conversion) depends on
correctly reading what is selected in Explorer.

## The chain

1. Find the focused Explorer window (`GetForegroundWindow`).
2. Walk COM `ShellWindows` to the matching shell view.
3. Read selected items (`Folder.SelectedItems()` → `FolderItem.Path`).
4. Drop `::`-prefixed virtual items; if nothing mappable remains, decline.

Implemented in `zest-selection`; currently a COM skeleton with the enumeration
marked `TODO(MVP-1)`.

## Traps the source does not show

- **Tabbed Explorer (Win11):** one HWND hosts multiple tabs. Match the active
  tab's `IShellBrowser` / `IFolderView`, not just the window.
- **Desktop:** there is no Explorer HWND. Query the `Progman` / `WorkerW`
  shell view instead.
- **Virtual folders:** Recycle Bin, This PC, and similar locations expose
  `::`-style paths with no filesystem target. Reject them gracefully (no menu,
  no error dialog).
- **Semi-documented COM:** the automation chain works but Microsoft can shift
  Explorer internals across major Windows updates. Prove this on Win10 and
  Win11 first, before any other milestone, and keep the `resolve_mock` test
  seam so menu logic stays testable when COM is unavailable.

## Contract

`resolve()` returns `Selection { files }` or a typed `SelectionError`
(`NoExplorerWindow`, `VirtualFolder`, `Com`, `Empty`). Callers never parse COM
themselves.
