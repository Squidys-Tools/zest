# Settings

Right-click the tray icon → **Settings**, or launch `zest.exe --settings`. It
is a single scrollable form. Open it once, set it, forget it. **Save** at the
bottom writes the file; changes are live in the ring as you make them.

## Menu shortcut

| Setting | Options | Default |
| --- | --- | --- |
| Menu shortcut | Press-record — the form captures the combination, it is not typed | `Shift+F` |

## Conversion

| Setting | Options | Default |
| --- | --- | --- |
| JPEG quality | Slider 1–100 | 90 |
| Video preset | Low / medium / high / lossless | Medium |
| Audio bitrate | Stepper, 64–320 kbps | 192 kbps |
| Warn before lossy-to-lossy conversion | Toggle | On |

## Appearance

| Setting | Options | Default |
| --- | --- | --- |
| Theme | System / light / dark | System |
| Interface font | Dropdown of the fonts installed on this machine | Your Windows UI font |

**System** follows the app theme setting in Windows, so the form matches the
machine whether it is in light or dark mode.

## Startup & updates

| Setting | Options | Default |
| --- | --- | --- |
| Launch at startup | Opt-in, writes HKCU Run key | Off |
| Update frequency | Daily / weekly / never | Weekly |

## Colors

The active radial sector is filled with a ramp of 1–3 colors, so you can add or
remove stops as you like (minimum 1, maximum 3). Pick a stop, then drag in the
color field or along the hue slider; the swatch list, the ramp bar, and the
ring preview all update as you drag. The ring uses the ramp the next time it
opens.

Settings persist to `%LOCALAPPDATA%\Zest\settings.json`.

## Next steps

- [Hotkey](./hotkey.md): what the shortcut does and its second-ring variant.
- [Updating](./updating.md): how the update schedule uses this page.

