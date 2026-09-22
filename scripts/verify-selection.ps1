<#
.SYNOPSIS
  Repeatable end-to-end proof for the Explorer selection resolver
  (Linear SQU-20/21/22/23, roadmap Phase 0 exit).

.DESCRIPTION
  Drives REAL Explorer windows and asserts `zest --check-selection` output:

    single PNG   -> real path, ring 1 [Convert, Archive]
    single zip   -> real path, ring 1 [Extract]
    mixed select -> all paths, ring 1 [Archive]
    empty select -> Empty path (documented mock fallback)
    tabs         -> selection follows the VISIBLE tab, both directions
    2nd window   -> selection follows the FOREGROUNDED window
    desktop      -> selection follows the Windows Desktop shell view

  Foregrounding uses minimize-then-restore, which reliably activates the
  script's own windows even from a background shell (plain
  SetForegroundWindow is foreground-locked by design). Tab flips use SendKeys
  against the foreground window.

  Selection setup uses Shell.Application automation; the resolver reads it
  back through independent COM interfaces. Tab creation/flipping uses real
  Ctrl+T / Ctrl+Tab keystrokes against the foreground window you clicked.

  Only windows opened by this script are closed at cleanup (tracked by HWND
  snapshot); your own Explorer windows are never touched.

  Explorer COM is Windows-only; run on Win10 and Win11 per SQU-22.
  Requires the binary: `cargo build -p zest-app` first.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-selection.ps1
#>
param(
    [string]$ZestExe = (Join-Path $PSScriptRoot "..\target\debug\zest.exe"),
    # Subset to run: any of "basic" (single/mixed/empty), "tabs", "window".
    # Useful for targeted re-verification; default runs everything.
  [string[]]$Only = @("basic", "tabs", "window", "desktop")
)

# Normalize: powershell.exe -File may deliver "basic,tabs" as ONE string.
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne "" } | ForEach-Object { $_.Trim() })

$ErrorActionPreference = "Stop"

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ZV {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr hWndParent, IntPtr hWndChildAfter, string lpszClass, string lpszWindow);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr lParam);
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint Msg, UIntPtr W, IntPtr L);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
    public const uint WM_CLOSE = 0x0010;
    public const int SW_MINIMIZE = 6;
    public const int SW_RESTORE = 9;
}
"@

$script:Failures = 0

function Log($msg) { Write-Output "[verify] $msg" }
function Fail($msg) {
    Write-Output "[verify] FAIL: $msg"
    $script:Failures++
}
function Pass($msg) { Write-Output "[verify] pass: $msg" }

function ShellWindows { return @(New-Object -ComObject Shell.Application | ForEach-Object { $_.Windows() }) }

function FindEntries($folder) {
    $out = @()
    foreach ($w in (ShellWindows)) {
        try {
            if ($w.Document.Folder.Self.Path -ieq $folder) { $out += $w }
        } catch {}
    }
    return $out
}

function AllHwnds { return @(ShellWindows | ForEach-Object { $_.HWND }) }

# Foreground a script-owned window: minimize-then-restore is honored even
# from a background shell (plain SetForegroundWindow is foreground-locked).
# Aborts loudly when it does not stick, so results never mislead.
function Ensure-Foreground($hwnd, $what) {
    [ZV]::ShowWindowAsync([IntPtr]$hwnd, [ZV]::SW_MINIMIZE) | Out-Null
    Start-Sleep -Milliseconds 500
    [ZV]::ShowWindowAsync([IntPtr]$hwnd, [ZV]::SW_RESTORE) | Out-Null
    Start-Sleep -Milliseconds 800
    $fg = [ZV]::GetForegroundWindow().ToInt64()
    if ($fg -ne $hwnd) {
        throw "[verify] ABORT foregrounding $what (hwnd={0}, got fg={1})" -f $hwnd, $fg
    }
    Log ("foreground hwnd={0} ({1})" -f $hwnd, $what)
}

function SelectExact($entry, [string[]]$names, $caseName) {
    # Deselect everything first (by name; indices shift while mutating).
    $view = $entry.Document
    $current = @()
    $sel = $view.SelectedItems()
    for ($i = 0; $i -lt $sel.Count; $i++) { $current += $sel.Item($i).Name }
    foreach ($n in $current) { $view.SelectItem($view.Folder.ParseName($n), 0) }
    foreach ($n in $names) { $view.SelectItem($view.Folder.ParseName($n), 1) }
    Start-Sleep -Milliseconds 800
    # Read back: a stale/dead Explorer view silently ignores SelectItem, so
    # never proceed on an unverified selection (it would fake a product fail).
    $got = @()
    $sel = $view.SelectedItems()
    for ($i = 0; $i -lt $sel.Count; $i++) { $got += $sel.Item($i).Name }
    $a = ($got | Sort-Object) -join "|"
    $e = ($names | Sort-Object) -join "|"
    if ($a -cne $e) {
        Fail ("setup {0}: selection did not take (want [{1}] got [{2}]) -- stale view?" -f $caseName, $e, $a)
    }
}

function RunCheck {
    return (& $ZestExe --check-selection 2>&1 | Out-String)
}

# The resolver keys off the FOREGROUND window. If focus moved (e.g. the
# runner clicked elsewhere mid-run), every later result would mislead -- abort
# loudly instead of asserting product behavior on a broken harness.
function Assert-Foreground($hwnd, $caseName) {
    $fg = [ZV]::GetForegroundWindow().ToInt64()
    if ($fg -ne $hwnd) {
        throw "[verify] ABORT {0}: foreground moved (expected hwnd={1}, got {2}) -- rerun when the desktop is idle" -f $caseName, $hwnd, $fg
    }
}

function Ensure-Desktop-Foreground {
    # The desktop is hosted by Progman even when its icon view is rendered by
    # a WorkerW. Set focus to the host and verify it through the same Win32
    # foreground query used by the resolver.
    $desktopHost = [ZV]::FindWindow("Progman", $null)
    if ($desktopHost -eq [IntPtr]::Zero) {
        $desktopHost = [ZV]::FindWindow("WorkerW", $null)
    }
    if ($desktopHost -eq [IntPtr]::Zero) {
        throw "[verify] ABORT foregrounding Desktop (Progman/WorkerW not found)"
    }
    [ZV]::SetForegroundWindow($desktopHost) | Out-Null
    Start-Sleep -Milliseconds 800
    $fg = [ZV]::GetForegroundWindow()
    if (-not (Test-Desktop-Window $fg)) {
        throw "[verify] ABORT foregrounding Desktop (expected Progman/WorkerW, got fg={0})" -f $fg.ToInt64()
    }
    Log ("foreground Desktop host hwnd={0} (fg={1})" -f $desktopHost.ToInt64(), $fg.ToInt64())
    return $desktopHost
}

function Test-Desktop-Window($hwnd) {
    while ($hwnd -ne [IntPtr]::Zero) {
        $class = New-Object System.Text.StringBuilder 64
        [ZV]::GetClassName($hwnd, $class, $class.Capacity) | Out-Null
        if ($class.ToString() -in @("Progman", "WorkerW")) { return $true }
        $parent = [ZV]::GetParent($hwnd)
        if ($parent -eq $hwnd) { break }
        $hwnd = $parent
    }
    return $false
}

function Select-Desktop-Exact([string]$name) {
    # The Desktop Folder COM object exposes ParseName/Items but not the
    # ShellFolderView.SelectItem method. Use the Desktop DefView's UIA list
    # item pattern, which selects the real shell item without icon-coordinate
    # assumptions.
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $progman = [ZV]::FindWindow("Progman", $null)
    $defview = [ZV]::FindWindowEx($progman, [IntPtr]::Zero, "SHELLDLL_DefView", $null)
    if ($defview -eq [IntPtr]::Zero) {
        $script:DesktopDefView = [IntPtr]::Zero
        $callback = [ZV+EnumProc]{ param($hwnd, $unused)
            $class = New-Object System.Text.StringBuilder 64
            [ZV]::GetClassName($hwnd, $class, $class.Capacity) | Out-Null
            if ($class.ToString() -eq "WorkerW") {
                $candidate = [ZV]::FindWindowEx($hwnd, [IntPtr]::Zero, "SHELLDLL_DefView", $null)
                if ($candidate -ne [IntPtr]::Zero) {
                    $script:DesktopDefView = $candidate
                    return $false
                }
            }
            return $true
        }
        [ZV]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
        $defview = $script:DesktopDefView
    }
    if ($defview -eq [IntPtr]::Zero) {
        throw "[verify] setup Desktop: SHELLDLL_DefView not found under Progman"
    }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($defview)
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)
    $items = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    for ($i = 0; $i -lt $items.Count; $i++) {
        $item = $items.Item($i)
        if ($item.Current.Name -ieq $name) {
            $pattern = $item.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
            $pattern.Select()
            Start-Sleep -Milliseconds 800
            return
        }
    }
    throw "[verify] setup Desktop: fixture not visible in Desktop shell view: $name"
}

function Assert-SetEqual($actual, $expected, $caseName) {
    $a = ($actual | Sort-Object) -join "|"
    $e = ($expected | Sort-Object) -join "|"
    if ($a -ceq $e) { Pass $caseName }
    else { Fail ("{0}`n  expected: {1}`n  actual:   {2}" -f $caseName, $e, $a) }
}

function Assert-Contains($output, $needle, $caseName) {
    # NOTE: String.Contains, not -like -- ring lines contain [brackets].
    if ($output.Contains($needle)) { Pass $caseName }
    else { Fail ("{0}: missing <{1}> in:`n{2}" -f $caseName, $needle, $output) }
}

function ParseFilesLine($output) {
    $m = [regex]::Match($output, 'files: \[(.*?)\]')
    if (-not $m.Success) { return @() }
    # Rust Debug escapes backslashes (C:\\…); unescape before comparing.
    return ([regex]::Matches($m.Groups[1].Value, '"([^"]*)"') | ForEach-Object {
        $_.Groups[1].Value -replace '\\\\', '\'
    })
}

# ---------------------------------------------------------------- setup ---

if (-not (Test-Path $ZestExe)) {
    Write-Output "[verify] FAIL: binary not found: $ZestExe (run cargo build -p zest-app first)"
    exit 1
}

$dir1 = Join-Path ([IO.Path]::GetTempPath()) "zest-verify"
$dir2 = Join-Path ([IO.Path]::GetTempPath()) "zest-verify2"
New-Item -ItemType Directory -Force -Path $dir1, $dir2 | Out-Null
Set-Content -Path (Join-Path $dir1 "photo.png") -Value "fakepng" -NoNewline
Set-Content -Path (Join-Path $dir1 "notes.txt") -Value "hello" -NoNewline
Set-Content -Path (Join-Path $dir1 "archive.zip") -Value "fakezip" -NoNewline
Set-Content -Path (Join-Path $dir2 "tab2file.txt") -Value "tab two" -NoNewline
Set-Content -Path (Join-Path $dir2 "tab2pic.png") -Value "fakepng2" -NoNewline

$p1 = Join-Path $dir1 "photo.png"
$pz = Join-Path $dir1 "archive.zip"
$pn = Join-Path $dir1 "notes.txt"
$p2 = Join-Path $dir2 "tab2file.txt"
$p2b = Join-Path $dir2 "tab2pic.png"

# Purge stale windows first: an Explorer view whose folder was deleted out
# from under it (e.g. by a previous run's cleanup) silently ignores
# SelectItem, which would fake product failures. Fixture dirs are
# script-owned, so no user window can show them.
foreach ($d in @($dir1, $dir2)) {
    foreach ($w in @(FindEntries $d)) {
        try { [ZV]::PostMessage([IntPtr]$w.HWND, [ZV]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero) | Out-Null } catch {}
    }
}
Start-Sleep -Milliseconds 800
$hwndsBefore = AllHwnds

# Open one Explorer window on dir1 (reuse it if one is already open).
$entries1 = @(FindEntries $dir1)
if ($entries1.Count -eq 0) {
    explorer.exe $dir1
    Start-Sleep -Milliseconds 2000
    $entries1 = @(FindEntries $dir1)
}
if ($entries1.Count -eq 0) { Fail "setup: no Explorer window on $dir1"; exit 1 }
$hwnd1 = $entries1[0].HWND
Log ("window1 hwnd={0} on {1}" -f $hwnd1, $dir1)

# ------------------------------------------------------------- cases 1-4 ---

if ($Only -contains "basic") {
SelectExact $entries1[0] @("photo.png") "single PNG"
Ensure-Foreground $hwnd1 "zest-verify (single photo.png selected)"
Assert-Foreground $hwnd1 "single PNG" | Out-Null
$out = RunCheck
Assert-SetEqual (ParseFilesLine $out) @($p1) "single PNG paths"
Assert-Contains $out "ring 1: [Convert, Archive]" "single PNG ring 1"

SelectExact $entries1[0] @("archive.zip") "single zip"
Ensure-Foreground $hwnd1 "zest-verify (single archive.zip selected)"
Assert-Foreground $hwnd1 "single zip" | Out-Null
$out = RunCheck
Assert-SetEqual (ParseFilesLine $out) @($pz) "single zip paths"
Assert-Contains $out "ring 1: [Extract]" "single zip ring 1"

SelectExact $entries1[0] @("photo.png", "notes.txt", "archive.zip") "mixed"
Ensure-Foreground $hwnd1 "zest-verify (all three files selected)"
Assert-Foreground $hwnd1 "mixed selection" | Out-Null
$out = RunCheck
Assert-SetEqual (ParseFilesLine $out) @($p1, $pn, $pz) "mixed selection paths"
Assert-Contains $out "ring 1: [Archive]" "mixed selection ring 1"

SelectExact $entries1[0] @() "empty"
Ensure-Foreground $hwnd1 "zest-verify (nothing selected)"
Assert-Foreground $hwnd1 "empty selection" | Out-Null
$out = RunCheck
Assert-SetEqual (ParseFilesLine $out) @("photo.png") "empty selection falls back to mock example"
} # -Only basic

# ------------------------------------------------------------- tab cases ---

if ($Only -contains "tabs") {

# Open dir2 as a real second TAB (Ctrl+T) in window1, then navigate it by
# typing the path into the address bar (Alt+D) -- all genuine keystrokes to
# the foreground window. Fresh tabs activate on open, so dir2 is showing.
$wshell = New-Object -ComObject WScript.Shell
$tabFailuresBefore = $script:Failures

# Rerun-safe: need window1 with exactly one tab. If dir2 is already tabbed
# into it from an earlier run, start that window over (only if we opened it).
$stray = @(FindEntries $dir2 | Where-Object { $_.HWND -eq $hwnd1 })
if ($stray.Count -gt 0) {
    if ($hwndsBefore -contains $hwnd1) {
        Fail "tabs: this window already tabs dir2 and it is not script-opened; close that tab and rerun with -Only tabs"
    } else {
        try { [ZV]::PostMessage([IntPtr]$hwnd1, [ZV]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero) | Out-Null } catch {}
        Start-Sleep -Milliseconds 1200
        explorer.exe $dir1
        Start-Sleep -Milliseconds 2000
        $entries1 = @(FindEntries $dir1)
        if ($entries1.Count -eq 0) { Fail "tabs: reopen of window1 failed" }
        else { $hwnd1 = $entries1[0].HWND }
    }
}

if ($script:Failures -eq $tabFailuresBefore) {
Ensure-Foreground $hwnd1 "zest-verify (about to open a second tab)"
$wshell.SendKeys("^{t}")
Start-Sleep -Milliseconds 1200
$wshell.SendKeys("%d")
Start-Sleep -Milliseconds 500
$wshell.SendKeys($dir2 + "~")
Start-Sleep -Milliseconds 2000

$entries1 = @(FindEntries $dir1)
$entries2 = @(FindEntries $dir2)
if (($entries2.Count -eq 0) -or ($entries2[0].HWND -ne $hwnd1)) {
    Fail "setup: dir2 did not open as a second tab of window1; skipping tab cases"
} else {
    SelectExact ($entries1 | Select-Object -First 1) @("photo.png") "tab1"
    SelectExact ($entries2 | Select-Object -First 1) @("tab2file.txt") "tab2"

    Ensure-Foreground $hwnd1 "the two-tab window (zest-verify2 tab showing)"
    Assert-Foreground $hwnd1 "visible tab 2"
    $out = RunCheck
    Assert-SetEqual (ParseFilesLine $out) @($p2) "visible tab 2 selection"

    # Exactly 2 tabs: Ctrl+Tab flips to the other one (same window keeps focus).
    $wshell.SendKeys("^{TAB}")
    Start-Sleep -Milliseconds 1000
    Assert-Foreground $hwnd1 "visible tab 1 after flip"
    $out = RunCheck
    Assert-SetEqual (ParseFilesLine $out) @($p1) "visible tab 1 selection after flip"
} # else dir2 is a second tab
} # if no tab setup failures
} # -Only tabs

# ---------------------------------------------------------- second window ---

if ($Only -contains "window") {

# A separate foreground window must win over window1's tabs.
$win2 = @(FindEntries $dir2 | Where-Object { $_.HWND -ne $hwnd1 })
if ($win2.Count -eq 0) {
    explorer.exe $dir2
    Start-Sleep -Milliseconds 2000
    $win2 = @(FindEntries $dir2 | Where-Object { $_.HWND -ne $hwnd1 })
}
if ($win2.Count -eq 0) {
    Log "dir2 reused window1; separate-window case already covered by tabs above"
} else {
    $hwndW = $win2[0].HWND
    SelectExact $win2[0] @("tab2pic.png") "window2"
    Ensure-Foreground $hwndW "the NEW zest-verify2 window (single tab2pic.png selected)"
    Assert-Foreground $hwndW "second window"
    $out = RunCheck
    Assert-SetEqual (ParseFilesLine $out) @($p2b) "second window selection"
    Assert-Contains $out "ring 1: [Convert, Archive]" "second window ring 1"
} # else separate window found
} # -Only window

# --------------------------------------------------------------- desktop ---

if ($Only -contains "desktop") {
    $desktopDir = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    $desktopName = "zest-selection-verify-$PID.txt"
    $desktopPath = Join-Path $desktopDir $desktopName
    Set-Content -Path $desktopPath -Value "desktop selection fixture" -NoNewline
    try {
        Ensure-Desktop-Foreground | Out-Null
        Select-Desktop-Exact $desktopName
        $out = RunCheck
        Assert-SetEqual (ParseFilesLine $out) @($desktopPath) "Desktop selection paths"
    }
    finally {
        Remove-Item -LiteralPath $desktopPath -Force -ErrorAction SilentlyContinue
    }
} # -Only desktop

# ---------------------------------------------------------------- cleanup ---

# Close only windows this run opened (never the runner's own windows).
$hwndsAfter = AllHwnds
foreach ($h in $hwndsAfter) {
    if ($hwndsBefore -notcontains $h) {
        try { [ZV]::PostMessage([IntPtr]$h, [ZV]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero) | Out-Null } catch {}
    }
}
Remove-Item -Recurse -Force $dir1, $dir2 -ErrorAction SilentlyContinue

if ($script:Failures -gt 0) {
    Write-Output ("[verify] {0} FAILURE(S)" -f $script:Failures)
    exit 1
}
Write-Output "[verify] ALL GREEN"
