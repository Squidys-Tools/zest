# Install Zest

Zest is a lightweight Windows tray utility. It runs in the background; there is
no main window. You interact with it through a hotkey and a radial menu.

## Requirements

- Windows 10 1809+ or Windows 11 (x64).
- No admin rights needed. Zest installs per-user to `%LOCALAPPDATA%\Zest\`.
- No external codecs or tools needed. FFmpeg ships inside the install for
  video and audio work.

## Install

Download the MSI from
[GitHub Releases](https://github.com/OWNER/zest/releases) and run it. It
installs to `%LOCALAPPDATA%\Zest\` without elevation.

| Task | How |
| --- | --- |
| Start Zest | Launch **Zest** from Start; it appears as a tray icon |
| Open settings | Right-click the tray icon → **Settings** |
| Quit | Right-click the tray icon → **Quit** |
| Start with Windows | Settings → **Launch at startup** (opt-in, HKCU Run key) |

## First launch

1. Confirm the tray icon is visible.
2. Select a file in Explorer or on the Desktop.
3. Press `Shift+F` (default). The radial menu appears at your cursor.
4. Pick a target. The menu closes and the job runs in the background.
5. A toast notification tells you when it is done.

The output lands next to the original with the new extension. On a name
collision it adds a number: `photo (1).jpg`, `photo (2).jpg`. Originals are
never modified or deleted.

## Next steps

- [First run](./first-run.md): the two-ring menu and what each ring shows.
- [Settings](./settings.md): hotkey, output location, quality, theme, colors.
- [Updating](./updating.md): release channel and check frequency.
