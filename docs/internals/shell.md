# Shell notes: startup, toast, updater

## HKCU Run key

`startup::set_enabled` writes `"…\zest.exe"` (quoted) to
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Zest`. `zest-app`
reconciles the registry with `settings.json` at two points: every launch
(catches hand-edited files) and whenever the settings window closes. There
is no watcher, so a save only lands on window close — acceptable for a
checkbox that only matters at sign-in.

Deleting a registry *value* requires `KEY_SET_VALUE` on the open key; a
`KEY_QUERY_VALUE` handle gets `Access is denied`. `delete_value` opens for
write, and treats missing keys/values (`ERROR_FILE_NOT_FOUND`) as success —
the goal is the value's absence, however it got there.

## Toasts and the AppUserModelID

Unpackaged exes have no registered AUMID, so `notify` calls
`SetCurrentProcessExplicitAppUserModelID("Zest")` once and then
`CreateToastNotifierWithId("Zest")`. That is the best an unpackaged app can
do: the toast shows, but attribution is the bare "Zest" name — no icon, and
activation does not deep-link. Proper attribution needs the MSI's Start-menu
shortcut carrying `System.AppUserModel.ID`; that is packaging work (`wix/`),
not code. `CreateToastNotifier()` (no id) is the fallback inside `notify`.

## update-state.json

`Settings` is user-edited config, so updater bookkeeping lives in a sibling
file, `%LOCALAPPDATA%\Zest\update-state.json`: `last_check_unix` (the
*attempt* time — writing it before the fetch throttles retries on a flaky
network, not just successes) and `pending_version` (a newer release seen,
surfaced in the tray menu until the build catches up). Cadence gating and
the semver compare are pure functions; `fetch_latest_tag` is the only
network seam.
