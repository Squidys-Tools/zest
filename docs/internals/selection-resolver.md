# Selection resolver

The riskiest piece of Zest. Everything downstream (menu, conversion) depends on
correctly reading what is selected in Explorer.

## The chain

1. Find the focused Explorer window (`GetForegroundWindow`).
2. Walk COM `ShellWindows` to the matching shell view.
3. Read selected items (`Folder.SelectedItems()` → `FolderItem.Path`).
4. Drop `::`-prefixed virtual items; if nothing mappable remains, decline.

Implemented in `zest-selection` in two tiers: the native active-tab path
(`IServiceProvider` → `IShellBrowser::QueryActiveShellView` → `IFolderView`
gate → `IShellItemArray` → `SIGDN_FILESYSPATH`) with the automation
`Document` → `SelectedItems()` chain as fallback. Desktop selection is routed
before those tiers through the desktop `SHELLDLL_DefView` hosted by `Progman`
or `WorkerW`; the same native selection read then yields filesystem paths.

## Traps the source does not show

- **Tabbed Explorer (Win11):** one HWND hosts multiple tabs (one ShellWindows
  entry each), so HWND matching alone is ambiguous. The selection comes from
  the visible tab: focused-view native read first, else the entry picked by
  the UI Automation tab strip (selected `TabItem`, folder-name verified).
  Dead ends, verified live: per-entry `QueryActiveShellView` yields the
  entry's *own* tab, inactive views stay `WS_VISIBLE`, and `OBJID_NATIVEOM`
  on the DefView is `E_FAIL` on current builds.
- **Desktop:** there is no Explorer HWND. Query the `Progman` / `WorkerW`
  shell view instead.
- **Virtual folders:** Recycle Bin, This PC, and similar locations expose
  `::`-style paths with no filesystem target. Filter them from mixed
  selections; if nothing filesystem-backed remains, return `VirtualFolder` so
  the caller declines without showing a menu or error dialog.
- **Semi-documented COM:** the automation chain works but Microsoft can shift
  Explorer internals across major Windows updates. Prove this on Win10 and
  Win11 first, before any other milestone, and keep the `resolve_mock` test
  seam so menu logic stays testable when COM is unavailable.

The repeatable Windows harness in `scripts/verify-selection.ps1` includes a
Desktop fixture (`-Only desktop`) in addition to Explorer, tab, and
foreground-window cases.

## Contract

`resolve()` returns `Selection { files }` or a typed `SelectionError`
(`NoExplorerWindow`, `VirtualFolder`, `Com`, `Empty`). Callers never parse COM
themselves.
