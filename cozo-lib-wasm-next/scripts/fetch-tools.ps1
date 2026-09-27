#Requires -Version 7.0
<#
.SYNOPSIS
  Fetch pinned helper binaries into cozo-lib-wasm-next/.tools/bin/.

.DESCRIPTION
  Downloads (via wget) and unpacks:
    - wasm-bindgen-cli, version taken from cozo-lib-wasm-next/Cargo.lock
      (must match the wasm-bindgen crate: bindgen refuses on mismatch);
    - wasm-opt (binaryen), pinned via -BinaryenVersion.
  Layout: .tools/bin/wasm-bindgen.exe, .tools/bin/wasm-opt.exe.
  Safe to re-run: skips binaries whose --version already matches.

.PARAMETER BinaryenVersion
  Binaryen release tag, e.g. "version_133". Minimum per guide: version_118.

.PARAMETER ToolsDir
  Defaults to <crate>/.tools. Binaries land in <ToolsDir>/bin.

.PARAMETER DownloadDir
  Scratch dir for archives. Defaults to <repo>/tmp (git-ignored).
#>
[CmdletBinding()]
param(
    [string]$BinaryenVersion = "version_133",
    [string]$ToolsDir = (Join-Path $PSScriptRoot "..\\.tools"),
    [string]$DownloadDir = (Join-Path $PSScriptRoot "..\\..\\tmp")
)

$ErrorActionPreference = "Stop"

$crateDir = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $ToolsDir "bin"
New-Item -ItemType Directory -Path $binDir, $DownloadDir -Force | Out-Null

function Get-LockedWasmBindgenVersion {
    $lock = Get-Content -LiteralPath (Join-Path $crateDir "Cargo.lock") -Raw
    $m = [regex]::Match($lock, 'name = "wasm-bindgen"\r?\nversion = "([^"]+)"')
    if (-not $m.Success) { throw "wasm-bindgen entry not found in Cargo.lock" }
    return $m.Groups[1].Value
}

$wbgVersion = Get-LockedWasmBindgenVersion
$wbgExe = Join-Path $binDir "wasm-bindgen.exe"
$optExe = Join-Path $binDir "wasm-opt.exe"

function Test-BinVersion([string]$exe, [string]$want) {
    if (-not (Test-Path -LiteralPath $exe)) { return $false }
    return ((& $exe --version 2>&1 | Out-String) -match [regex]::Escape($want))
}

$jobs = @()

if (-not (Test-BinVersion $wbgExe $wbgVersion)) {
    $tgz = Join-Path $DownloadDir "wasm-bindgen-$wbgVersion.tar.gz"
    if (-not (Test-Path -LiteralPath $tgz)) {
        $url = "https://github.com/rustwasm/wasm-bindgen/releases/download/$wbgVersion/wasm-bindgen-$wbgVersion-x86_64-pc-windows-msvc.tar.gz"
        Write-Host "Downloading $url"
        wget -O $tgz $url
    }
    $stage = Join-Path $DownloadDir "wbg-stage"
    if (Test-Path -LiteralPath $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    tar -xzf $tgz -C $stage
    $src = Get-ChildItem -Recurse -Filter "wasm-bindgen.exe" -Path $stage | Select-Object -First 1
    if (-not $src) { throw "wasm-bindgen.exe not found in archive" }
    Copy-Item $src.FullName $wbgExe -Force
    Write-Host "Installed $wbgExe"
} else {
    Write-Host "wasm-bindgen $wbgVersion already present, skipping."
}

if (-not (Test-BinVersion $optExe $BinaryenVersion.Replace("version_", ""))) {
    $tgz = Join-Path $DownloadDir "binaryen-$BinaryenVersion.tar.gz"
    if (-not (Test-Path -LiteralPath $tgz)) {
        $url = "https://github.com/WebAssembly/binaryen/releases/download/$BinaryenVersion/binaryen-$BinaryenVersion-x86_64-windows.tar.gz"
        Write-Host "Downloading $url"
        wget -O $tgz $url
    }
    $stage = Join-Path $DownloadDir "binaryen-stage"
    if (Test-Path -LiteralPath $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    tar -xzf $tgz -C $stage
    $src = Get-ChildItem -Recurse -Filter "wasm-opt.exe" -Path $stage | Select-Object -First 1
    if (-not $src) { throw "wasm-opt.exe not found in archive" }
    Copy-Item $src.FullName $optExe -Force
    Write-Host "Installed $optExe"
} else {
    Write-Host "wasm-opt ($BinaryenVersion) already present, skipping."
}

& $wbgExe --version
& $optExe --version
