#Requires -Version 7.0
<#
.SYNOPSIS
  Manual release pipeline for cozo-lib-wasm-next (no wasm-pack).

.DESCRIPTION
  Guide §8 pipeline, Windows edition, crate-local:
    1. cargo build --target wasm32-unknown-unknown [--release]
       (nightly via rust-toolchain.toml; build-std + atomics/SIMD/shared-memory
       link args via .cargo/config.toml)
    2. .tools/bin/wasm-bindgen <raw.wasm> --out-dir pkg --target web
    3. .tools/bin/wasm-opt -O3 --enable-threads --enable-bulk-memory
       --enable-simd --enable-mutable-globals pkg/*_bg.wasm -o itself
    4. Best-effort size report (+ twiggy top if installed).

  Run from anywhere:  powershell -File cozo-lib-wasm-next/scripts/build.ps1
  Validation (fast):  .../build.ps1 -Configuration Debug
  Production:         .../build.ps1 -Configuration Release

  Requirements (checked, not installed): pinned nightly (rustc --version must
  say nightly), .tools/bin/wasm-bindgen.exe + wasm-opt.exe (see
  scripts/fetch-tools.ps1). RUSTC_WRAPPER must NOT be sccache: its 0.18.0
  cannot spawn the 50k-char web-sys rustc command on Windows (os error 206);
  kache is the verified wrapper (see WORKLOG session 4).

.PARAMETER Configuration
  Debug (fast validation, links the cdylib incl. new link args) or Release.

.PARAMETER SkipWasmOpt
  Skip the wasm-opt pass (debugging the bindgen output).

.PARAMETER NoTypescript
  Pass --no-typescript to wasm-bindgen.
#>
[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",
    [switch]$SkipWasmOpt,
    [switch]$NoTypescript
)

$ErrorActionPreference = "Stop"

$crateDir = Split-Path -Parent $PSScriptRoot
$toolsBin = Join-Path $crateDir ".tools\bin"
$wbgExe = Join-Path $toolsBin "wasm-bindgen.exe"
$optExe = Join-Path $toolsBin "wasm-opt.exe"
$pkgDir = Join-Path $crateDir "pkg"
$libName = "cozo_lib_wasm_next"

# REBUILD GUARANTEE: wasm-bindgen below writes ONLY to pkg/ (git-ignored scratch).
# Hand-written JS lives in js/ (capabilities.mjs, cozo_next.mjs) + demo/ + tests/probe/
# and is NEVER overwritten: bindgen regenerates pkg/*.js, pkg/*.d.ts,
# pkg/*_bg.wasm, pkg/snippets/** and nothing else. Do NOT put sources in pkg/.

# 0. Preconditions (read-only checks).
$rustcVer = (rustc --version 2>&1 | Out-String)
if ($rustcVer -notmatch "nightly") {
    throw "Must run under pinned nightly (rust-toolchain.toml). Got: $rustcVer"
}
foreach ($exe in @($wbgExe, $optExe)) {
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Missing $exe. Run scripts/fetch-tools.ps1 first."
    }
}
if ($env:RUSTC_WRAPPER -eq "sccache") {
    throw "RUSTC_WRAPPER=sccache cannot build this crate (os error 206 on web-sys; see WORKLOG). Use kache."
}
# wasm-bindgen CLI must equal the locked crate version.
$lock = Get-Content -LiteralPath (Join-Path $crateDir "Cargo.lock") -Raw
$want = ([regex]::Match($lock, 'name = "wasm-bindgen"\r?\nversion = "([^"]+)"')).Groups[1].Value
$have = (& $wbgExe --version 2>&1 | Out-String)
if ($have -notmatch [regex]::Escape($want)) {
    throw "wasm-bindgen CLI mismatch: have [$($have.Trim())], lock wants [$want]. Run scripts/fetch-tools.ps1."
}

# 1. Compile.
$cargoArgs = @("build", "--target", "wasm32-unknown-unknown")
$cfgDir = "debug"
if ($Configuration -eq "Release") { $cargoArgs += "--release"; $cfgDir = "release" }
Write-Host "== cargo $($cargoArgs -join ' ')  (in $crateDir)"
Push-Location $crateDir
try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
} finally { Pop-Location }

$rawWasm = Join-Path $crateDir "target\wasm32-unknown-unknown\$cfgDir\$libName.wasm"
if (-not (Test-Path -LiteralPath $rawWasm)) { throw "Expected output missing: $rawWasm" }
"Raw wasm: {0:N0} bytes" -f (Get-Item $rawWasm).Length

# 2. Bindgen.
New-Item -ItemType Directory -Path $pkgDir -Force | Out-Null
$bgArgs = @($rawWasm, "--out-dir", $pkgDir, "--target", "web")
if ($NoTypescript) { $bgArgs += "--no-typescript" }
Write-Host "== wasm-bindgen $($bgArgs -join ' ')"
& $wbgExe @bgArgs
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed ($LASTEXITCODE)" }

$bgWasm = Join-Path $pkgDir ($libName + "_bg.wasm")
"Bindgen wasm: {0:N0} bytes" -f (Get-Item $bgWasm).Length

# 2b. Rayon worker bootstrap fix (plain static servers, no bundler).
# wasm-bindgen 0.2.129 emits workerHelpers.js at
# snippets/wasm-bindgen-rayon-<hash>/src/workerHelpers.js, but its
# `await import('../../..')` assumes the pre-src layout (and a bundler that
# resolves a directory import). From the real location it resolves to <pkg>/,
# which serve.mjs 404s — every worker then dies with "Failed to fetch
# dynamically imported module". Rewrite to an explicit relative file import
# that any static server can satisfy.
$helpers = Get-ChildItem -LiteralPath (Join-Path $pkgDir "snippets") -Recurse -Filter "workerHelpers.js" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $helpers) { throw "workerHelpers.js not found under pkg/snippets — wasm-bindgen layout changed?" }
$helpersPath = $helpers.FullName
$helpersText = Get-Content -LiteralPath $helpersPath -Raw
$needle = "await import('../../..')"
$fixed = "await import('../../../$libName.js')"
if ($helpersText -notmatch [regex]::Escape($needle)) { throw "workerHelpers.js no longer contains [$needle] — re-check the rayon bootstrap assumption" }
$helpersText.Replace($needle, $fixed) | Set-Content -LiteralPath $helpersPath -NoNewline
Write-Host "== patched worker bootstrap: $needle -> $fixed"

# 3. wasm-opt (explicit feature flags: without them it strips/rejects
#    atomics + SIMD; bundled wasm-pack wasm-opt is too old, hence .tools).
#    NOTE: --enable-nontrapping-float-to-int is required: modern rustc emits
#    i64.trunc_sat_f64_s (saturating float-to-int), and without the flag the
#    validator fails with "all used features should be allowed".
if (-not $SkipWasmOpt) {
    Write-Host "== wasm-opt --flatten --rereloop -Oz -Oz --low-memory-unused --enable-threads --enable-bulk-memory --enable-simd --enable-mutable-globals --enable-nontrapping-float-to-int"
    & $optExe --flatten --rereloop -Oz -Oz --low-memory-unused --enable-threads --enable-bulk-memory --enable-simd --enable-mutable-globals --enable-nontrapping-float-to-int $bgWasm -o $bgWasm
    if ($LASTEXITCODE -ne 0) { throw "wasm-opt failed ($LASTEXITCODE)" }
    "Optimized wasm: {0:N0} bytes" -f (Get-Item $bgWasm).Length
}

# 4. Size report (twiggy best-effort; run BEFORE strip ideally — our release
#    profile strips at link, so treat names as approximate).
if (Get-Command twiggy -ErrorAction SilentlyContinue) {
    Write-Host "== twiggy top -n 15 $bgWasm"
    & twiggy top -n 15 $bgWasm 2>&1 | Select-Object -First 20
}

Write-Host "Build complete. Artifacts in: $pkgDir"
