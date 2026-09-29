<#
.SYNOPSIS
  Build Zest for x86_64-pc-windows-gnu, working around the one import library
  rustup's bundled MinGW does not ship.

.DESCRIPTION
  `cargo build` fails on the GNU target with:

      ld: cannot find -lktmw32: No such file or directory

  That is not a broken gcc. rustup's `rust-mingw` component links against its
  own self-contained MinGW:

      <sysroot>\lib\rustlib\x86_64-pc-windows-gnu\lib\self-contained

  which carries 40-odd import libraries plus crt2.o and libgcc_eh.a, but not
  `libktmw32.a`. The bundled `x86_64-w64-mingw32-gcc` only ever searches that
  directory, so a perfectly good MinGW installed elsewhere is never consulted.
  The `windows_x86_64_gnu` crates on the search path carry a single combined
  `libwindows.0.5x.0.a` rather than per-DLL libraries, so they do not help
  either.

  `-lktmw32` reaches the link line because `gpui` enables the `windows` feature
  `Win32_Storage_FileSystem`, whose generated bindings carry
  `#[link("ktmw32.dll")]` on the kernel transaction manager calls. Zest never
  calls into KTM; the import library only has to exist at link time.

  So this script finds a MinGW import directory that has `libktmw32.a` and puts
  it on the linker's search path with `-L native=`. Failing that it mints a
  two-line stub, which is equally safe for the same reason.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\build-gnu.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\build-gnu.ps1 -Release
#>
param(
    # Import-library directory to use. Autodetected when omitted.
    [string]$MingwLibDir,

    # Extra directories to put on the linker search path, same as -L native=.
    [string[]]$ExtraLibDir = @(),

    # Set to skip autodetection and always mint the ktmw32 stub.
    [switch]$ForceStub,

    # Build the optimised binary instead of the debug one.
    [switch]$Release
)

$ErrorActionPreference = "Stop"

function Log($msg) { Write-Output "[gnu] $msg" }

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

# The library whose absence is the whole problem. Everything else the linker
# wants, rustup's self-contained directory already has.
$Missing = "libktmw32.a"

function Test-LibDir([string]$dir) {
    if ([string]::IsNullOrWhiteSpace($dir)) { return $false }
    return Test-Path (Join-Path $dir $Missing)
}

function Resolve-Gcc {
    # Ask PATH, then the usual MinGW install layouts.
    $onPath = (Get-Command "x86_64-w64-mingw32-gcc" -ErrorAction SilentlyContinue)
    if ($onPath) { return (Split-Path (Split-Path $onPath.Source -Parent) -Parent) }
    $gcc = (Get-Command "gcc" -ErrorAction SilentlyContinue)
    if ($gcc) { return (Split-Path (Split-Path $gcc.Source -Parent) -Parent) }
    return $null
}

# Where a complete MinGW keeps its import libraries, relative to the gcc root.
$libSubdirs = @(
    "x86_64-w64-mingw32\lib",
    "lib"
)

function Find-MingwLibDir {
    $candidates = @()
    if ($env:ZEST_MINGW_LIB) { $candidates += $env:ZEST_MINGW_LIB }

    $gccRoot = Resolve-Gcc
    if ($gccRoot) {
        foreach ($sub in $libSubdirs) { $candidates += (Join-Path $gccRoot $sub) }
    }

    $candidates += @(
        "$env:USERPROFILE\scoop\apps\gcc\current\x86_64-w64-mingw32\lib",
        "C:\msys64\mingw64\lib",
        "C:\msys64\ucrt64\lib",
        "C:\msys64\x86_64-w64-mingw32\lib",
        "C:\ProgramData\chocolatey\lib\mingw\tools\install\mingw64\x86_64-w64-mingw32\lib",
        "C:\ProgramData\chocolatey\bin\x86_64-w64-mingw32\lib",
        "C:\mingw64\x86_64-w64-mingw32\lib"
    )

    foreach ($c in $candidates) { if (Test-LibDir $c) { return (Resolve-Path $c).Path } }
    return $null
}

function New-Ktmw32Stub {
    # `target/` is gitignored, so the stub lives with build output rather than
    # in the source tree. Emitted once and reused after that.
    $stubDir = Join-Path $repoRoot "target\gnu-stubs"
    $archive = Join-Path $stubDir $Missing
    if (Test-Path $archive) { return $stubDir }

    $dlltool = (Get-Command "x86_64-w64-mingw32-dlltool" -ErrorAction SilentlyContinue)
    if (-not $dlltool) { $dlltool = (Get-Command "dlltool" -ErrorAction SilentlyContinue) }
    if (-not $dlltool) {
        throw "[gnu] no $Missing and no dlltool to mint one. Install a MinGW-w64 GCC (scoop install gcc / choco install mingw / pacman -S mingw-w64-x86_64-gcc), or pass -MingwLibDir <dir>."
    }

    New-Item -ItemType Directory -Force -Path $stubDir | Out-Null
    $def = Join-Path $stubDir "ktmw32.def"
    # An empty export list: the symbols resolve at link time against the real
    # DLL and are never called, because nothing in Zest uses KTM.
    Set-Content -Path $def -Value @("LIBRARY Ktmw32.dll", "EXPORTS") -Encoding ascii

    & $dlltool.Source -d $def -l $archive
    if ($LASTEXITCODE -ne 0) { throw "[gnu] dlltool failed to write $archive" }
    if (-not (Test-Path $archive)) { throw "[gnu] dlltool reported success but $archive is missing" }
    return $stubDir
}

# --------------------------------------------------------------- resolve ---

if ($MingwLibDir -and -not (Test-LibDir $MingwLibDir)) {
    throw "[gnu] -MingwLibDir '$MingwLibDir' does not contain $Missing"
}

$searchDirs = @()
if ($MingwLibDir) {
    $searchDirs += (Resolve-Path $MingwLibDir).Path
    Log "using -MingwLibDir $MingwLibDir"
}
elseif ($ForceStub) {
    $searchDirs += (New-Ktmw32Stub)
    Log "minted a $Missing stub in $($searchDirs[-1])"
}
else {
    $found = Find-MingwLibDir
    if ($found) {
        $searchDirs += $found
        Log "found $Missing in $found"
    }
    else {
        $searchDirs += (New-Ktmw32Stub)
        Log "no MinGW import directory with $Missing; minted a stub in $($searchDirs[-1])"
    }
}

foreach ($extra in $ExtraLibDir) {
    if (-not (Test-Path $extra)) { throw "[gnu] -ExtraLibDir '$extra' does not exist" }
    $searchDirs += (Resolve-Path $extra).Path
}

# ------------------------------------------------------------- environment ---

# cargo resolves rustc from PATH, which on a machine with a second, non-rustup
# Rust install can be a different toolchain. Mixed versions fail with a wall of
# "found crate X compiled by an incompatible version of rustc" (E0514), so pin
# both to whatever cargo is about to use.
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue)
if (-not $cargo) { throw "[gnu] cargo is not on PATH" }
$cargoBin = Split-Path $cargo.Source -Parent
$rustcExe = Join-Path $cargoBin "rustc.exe"
if (Test-Path $rustcExe) {
    $env:RUSTC = $rustcExe
    $rustdocExe = Join-Path $cargoBin "rustdoc.exe"
    if (Test-Path $rustdocExe) { $env:RUSTDOC = $rustdocExe }
}
else {
    Log "cargo has no sibling rustc.exe; relying on rustup to resolve the toolchain"
}

# Append rather than replace, so a caller who set RUSTFLAGS for their own reasons
# keeps them. Note that setting RUSTFLAGS at all disables cargo's default
# behaviour of reading `[build] rustflags` from .cargo/config.toml, which is why
# that file is not the home for this setting.
$linkArgs = ($searchDirs | ForEach-Object { "-L native=$_" }) -join " "
$env:RUSTFLAGS = if ([string]::IsNullOrWhiteSpace($env:RUSTFLAGS)) { $linkArgs }
                  else { "$($env:RUSTFLAGS) $linkArgs" }
Log "RUSTFLAGS += $linkArgs"

# ------------------------------------------------------------------- build ---

Push-Location $repoRoot
try {
    # Splat an array rather than passing a possibly-empty string, which cargo
    # rejects as an unexpected positional argument.
    $cargoArgs = @("build")
    if ($Release) { $cargoArgs += "--release" }
    & $cargo.Source @cargoArgs
    $code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($code -ne 0) { exit $code }

$built = Join-Path $repoRoot "target\debug\zest.exe"
if ($Release) { $built = Join-Path $repoRoot "target\release\zest.exe" }
if (Test-Path $built) { Log "built $built" }
exit 0
