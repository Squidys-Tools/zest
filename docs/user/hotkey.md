# Hotkey

One shortcut opens the menu. A second shortcut jumps straight to the Convert
ring.

| Action | Default | Change it in |
| --- | --- | --- |
| Open menu (ring 1) | `Shift+F` | Settings → Hotkey |
| Open Convert ring directly | `Shift+C` | Built-in; reserved |

The MVP field accepts a record-style string such as `Shift+F`. Settings rejects
malformed shortcuts and `Shift+C`, so saving a shortcut can’t disable startup.
If an existing settings file contains an invalid or unavailable shortcut, Zest
falls back to `Shift+F` and opens Settings so you can correct it. Save the new
shortcut and restart Zest to apply it.

If another app already uses `Shift+C`, Zest keeps the menu shortcut working and
reports that direct Convert access is unavailable until the other app releases
the shortcut.

If the hotkey stops working, another app usually claimed it first. Pick a
different combination in Settings. Global hotkeys can occasionally trip
antivirus heuristics; a signed build is the long-term fix (see
[Roadmap](../ROADMAP.md)).

## Next steps

- [First run](./first-run.md): what appears after you press it.
- [Troubleshooting](./troubleshooting.md): selection and hotkey failures.
