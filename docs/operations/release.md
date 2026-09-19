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

`zest-shell::updater` checks GitHub Releases on startup and on the configured
schedule (daily / weekly / never). It prompts to install on the next restart
and never restarts the app mid-task.

Long-term hardening, in order: EV code-signing certificate, then submission to
major AV vendors for whitelisting (global hotkey hooks trigger heuristics).
