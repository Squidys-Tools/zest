# Release and packaging

## MSI

Per-user install to `%LOCALAPPDATA%\Zest\`, no elevation, packaged with WiX
(`wix/Product.wxs`). Requires WiX Toolset v4+:

```powershell
dotnet tool install --global wix
cargo build --release -p zest-app
wix build wix/Product.wxs
```

The MSI lays down `zest.exe` and the bundled `ffmpeg\ffmpeg.exe`. Startup
opt-in writes the HKCU Run key; uninstall removes binaries and leaves user
settings in place.

## Updates

`zest-shell::updater` checks GitHub Releases on launch whenever the
configured cadence (daily / weekly / never) is due, recording state in
`update-state.json` next to `settings.json`. A newer release shows a toast
and adds an **Update to Zest …** tray-menu item that opens the release
page. Downloading and running the MSI stays manual — Zest never fetches
binaries or restarts itself mid-task.

Long-term hardening, in order: EV code-signing certificate, then submission to
major AV vendors for whitelisting (global hotkey hooks trigger heuristics).
