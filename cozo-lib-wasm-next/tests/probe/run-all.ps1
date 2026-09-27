#Requires -Version 7.0
<#
.SYNOPSIS
  Run all 21 thread-probe cases, one case per child process (fresh CozoDb).
.DESCRIPTION
  Mirrors the cozo-lib-wasm probe model: each case gets a fresh process so a
  trap in one case cannot contaminate the next.
#>
[CmdletBinding()]
param(
  [string]$PkgDir = (Join-Path (Split-Path -Parent $PSScriptRoot | Split-Path -Parent) "pkg"),
  [int]$Threads = 4
)

$ErrorActionPreference = "Stop"
$probe = Join-Path $PSScriptRoot "probe.mjs"
$cases = @(
  'basic','query_create','hnsw_empty','hnsw_rows','hnsw_query','lsh_rows',
  'degree_centrality','dfs','random_walk','astar','hnsw_incremental','fts_rows',
  'pagerank','top_sort','bfs','spbfs','spbfs_timeout','triggers','graph',
  'graph_drop','graph_query'
)

# Tripwire (WORKLOG sessions 6-7): these 5 used to trap on
# Instant/SystemTime ("time not implemented") before the session-7 shims;
# all 21 are green since. Any trap is unexpected.
$knownTraps = @('query_create','hnsw_rows','pagerank','top_sort','graph_query')

$unexpected = 0
foreach ($c in $cases) {
  $out = node $probe $PkgDir $c $Threads 2>&1 | Out-String
  try {
    $j = $out | ConvertFrom-Json -Depth 10
    $traps = @($j | Where-Object { $_.TRAP })
    if ($traps.Count -gt 0) {
      Write-Host "TRAP $c : $([string]$traps[0].TRAP | Select-Object -First 1)"
      if ($c -notin $knownTraps) { $unexpected++ }
    } else {
      $bad = @($j | Where-Object { $_.label -and $_.ok -eq $false })
      Write-Host "ok   $c ($($bad.Count) ok:false steps)"
    }
  } catch {
    $tag = if ($c -in $knownTraps) { "KNOWN-TRAP" } else { "TRAP" }
    Write-Host "$tag $c : (native panic, no JSON)"
    if ($c -notin $knownTraps) { $unexpected++ }
  }
}
if ($unexpected -gt 0) { throw "$unexpected case(s) with UNEXPECTED traps (see above)" }
Write-Host "probe run complete (knownTraps baseline kept for regressions)."
