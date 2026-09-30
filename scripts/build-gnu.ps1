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

  It also pins the whole toolchain. cargo takes rustc from PATH, and a machine
  with a second, non-rustup Rust install ahead of the shims otherwise mixes two
  toolchains. Worse, that install may be an MSVC one with no GNU std at all. And
  the gcc must be the toolchain's own bundled driver: it was built against that
  toolchain's libgcc, so a separately installed MinGW's driver fails the link
  with "cannot find -lgcc_eh".

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

# Passed to cargo explicitly, never left to the host default. Nothing in the
# repo overrides the default target, so omitting this would build MSVC and
# leave every line of linker setup below doing nothing.
$target = "x86_64-pc-windows-gnu"

# The library whose absence is the whole problem. Everything else the linker
# wants, rustup's self-contained directory already has.
$Missing = "libktmw32.a"

function Test-LibDir([string]$dir) {
    if ([string]::IsNullOrWhiteSpace($dir)) { return $false }
    return Test-Path (Join-Path $dir $Missing)
}

function Resolve-GccRoot([string]$gccExe) {
    if (-not $gccExe) { return $null }
    return (Split-Path (Split-Path $gccExe -Parent) -Parent)
}

# Where a complete MinGW keeps its import libraries, relative to the gcc root.
$libSubdirs = @(
    "x86_64-w64-mingw32\lib",
    "lib"
)

function Find-MingwLibDir {
    $candidates = @()
    if ($env:ZEST_MINGW_LIB) { $candidates += $env:ZEST_MINGW_LIB }

    $gccRoot = Resolve-GccRoot $script:gccExe
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

# --------------------------------------------------------------- toolchain ---

# The toolchain has to be one that can actually link $target, and the default
# one usually cannot. `rust-toolchain.toml` pins an MSVC toolchain, and an
# MSVC-host toolchain has no gnu std, no bundled gcc, and no self-contained
# import libraries — the build dies with E0463 or "linker not found" before any
# of the setup below matters.
#
# cargo takes rustc from PATH, so a second, non-rustup install ahead of the
# shims makes it worse: mixing toolchains leaves a wall of E0514s, and a
# standalone MSVC install has no GNU std at all.
#
# So resolve the toolchain through rustup and pick one whose host is $target.
$rustup = (Get-Command "rustup" -ErrorAction SilentlyContinue)
if (-not $rustup) {
    throw "[gnu] rustup is not on PATH. This script needs it to find a $target toolchain; a bare MSVC cargo cannot cross-compile to it."
}

function Get-Toolchain([string]$name, [string]$tool) {
    # `rustup which --toolchain` exits non-zero for a toolchain that is not
    # installed, so an unknown name simply yields $null.
    $out = (& $rustup.Source which --toolchain $name $tool 2>$null)
    if ($LASTEXITCODE -ne 0) { return $null }
    $path = @($out)[0]
    if (-not $path) { return $null }
    return $path.Trim()
}

# Prefer the toolchain rust-toolchain.toml asks for, but only if it can link the
# target; otherwise take the first installed one that can.
$pinned = @(& $rustup.Source show active-toolchain 2>$null)[0]
# That line reads "1.92.0-... (overridden by '...rust-toolchain.toml')"; keep the
# toolchain name only.
$pinned = if ($pinned) { ($pinned.Trim() -split '\s+')[0] } else { $null }
$installed = @(& $rustup.Source toolchain list 2>$null | ForEach-Object { ($_ -split '\s+')[0] })

$ordered = @()
if ($pinned) { $ordered += $pinned }
foreach ($tc in $installed) { if ($tc -ne $pinned) { $ordered += $tc } }

$rustcExe = $null
$cargoExe = $null
$chosen = $null
foreach ($tc in $ordered) {
    $tcRustc = Get-Toolchain $tc "rustc"
    if (-not $tcRustc) { continue }
    $hostTriple = ((& $tcRustc -vV 2>$null | Select-String "^host:") -replace "^host:\s*", "")
    if ($hostTriple -ne $target) { continue }
    $tcCargo = Get-Toolchain $tc "cargo"
    if (-not $tcCargo) { continue }
    $rustcExe = $tcRustc
    $cargoExe = $tcCargo
    $chosen = $tc
    break
}

if (-not $chosen) {
    throw "[gnu] no installed rustup toolchain targets $target. Install one: rustup toolchain install stable-x86_64-pc-windows-gnu"
}

if ($pinned -and $chosen -ne $pinned) {
    Log "rust-toolchain.toml pins '$pinned', which cannot build $target; using '$chosen' instead"
}
$env:RUSTC = $rustcExe
Log "toolchain = $chosen"

$rustdocExe = Get-Toolchain $chosen "rustdoc"
if ($rustdocExe) { $env:RUSTDOC = $rustdocExe }

# That toolchain carries its own gcc, kept beside the import libraries and crt
# objects it was built against:
#
#   <toolchain>\lib\rustlib\<target>\bin\self-contained\x86_64-w64-mingw32-gcc.exe
#   <toolchain>\lib\rustlib\<target>\lib\self-contained
#
# Point cargo at that driver rather than any gcc on PATH. A separately installed
# MinGW is a different build with a different libgcc, and pairing its driver with
# the toolchain's own libraries fails at the link with "cannot find -lgcc_eh".
$sysroot = (& $rustcExe --print sysroot 2>$null)
$bundledGcc = Join-Path $sysroot "lib\rustlib\$target\bin\self-contained\x86_64-w64-mingw32-gcc.exe"
if (-not (Test-Path $bundledGcc)) {
    throw "[gnu] toolchain '$chosen' has no bundled gcc at $bundledGcc. rustup toolchain install $chosen --component rust-mingw"
}
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $bundledGcc
Log "linker = $bundledGcc"

# Only used to guess where a complete MinGW might live.
$script:gccExe = (Get-Command "x86_64-w64-mingw32-gcc" -ErrorAction SilentlyContinue).Source

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
    $cargoArgs = @("build", "--target", $target)
    if ($Release) { $cargoArgs += "--release" }
    & $cargoExe @cargoArgs
    $code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($code -ne 0) { exit $code }

# cargo nests cross-target output under target\<triple>\<profile>.
$profile = if ($Release) { "release" } else { "debug" }
$built = Join-Path $repoRoot "target\$target\$profile\zest.exe"
if (Test-Path $built) { Log "built $built" }
else { throw "[gnu] cargo reported success but $built is missing" }
exit 0
