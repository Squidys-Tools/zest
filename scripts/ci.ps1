<#
.SYNOPSIS
  The single CI gate for Zest: clippy, workspace tests, and the app build.
  Runs identically on a dev machine and in GitHub Actions
  (.github/workflows/ci.yml), which is a thin wrapper around this script.

.DESCRIPTION
  Runs three steps in order, failing fast with the first failing step's exit
  code and printing a header per step so the failure is obvious in a log:

    1. cargo clippy --workspace --all-targets -- -D warnings
    2. cargo test --workspace
    3. cargo build -p zest-app

  PowerShell does not fail on native exit codes by itself, so $LASTEXITCODE
  is checked after every cargo invocation.

  Uses the pinned MSVC toolchain from rust-toolchain.toml. The GNU target
  needs scripts\build-gnu.ps1 instead and is not part of CI.

.EXAMPLE
  pwsh scripts/ci.ps1
#>

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

function Invoke-Step([string]$Name, [scriptblock]$Command) {
    Write-Output ""
    Write-Output "=============================================================="
    Write-Output "== [ci] $Name"
    Write-Output "=============================================================="
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Command
    $code = $LASTEXITCODE
    $sw.Stop()
    $elapsed = $sw.Elapsed.ToString("mm\:ss")
    if ($code -ne 0) {
        Write-Output "[ci] FAILED: $Name (exit $code, $elapsed)"
        exit $code
    }
    Write-Output "[ci] ok: $Name ($elapsed)"
}

Push-Location $repoRoot
try {
    Invoke-Step "clippy (warnings denied)" { cargo clippy --workspace --all-targets -- -D warnings }
    Invoke-Step "test (workspace)" { cargo test --workspace }
    Invoke-Step "build (zest-app)" { cargo build -p zest-app }
}
finally {
    Pop-Location
}

Write-Output "[ci] ALL STEPS PASSED"
exit 0
