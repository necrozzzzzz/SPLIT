$ErrorActionPreference = "SilentlyContinue"

$repoRoot = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $repoRoot "hyperline-scan"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$report = Join-Path $outDir "hyperline-freestyle-focused.txt"

$roots = @(
  "$env:APPDATA\Hyperline\mods\freestyle\0.2.240",
  "$env:APPDATA\Hyperline\mods\freestyle\0.2.239",
  "$env:APPDATA\Hyperline\mods\hyperline-ui"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

$lines = New-Object System.Collections.Generic.List[string]
function Add-Line([string]$text = "") { $lines.Add($text) }

function Add-ContextBlock {
  param(
    [string]$Path,
    [int]$LineNumber,
    [int]$Before = 30,
    [int]$After = 80
  )
  $all = Get-Content -LiteralPath $Path
  $start = [Math]::Max(1, $LineNumber - $Before)
  $end = [Math]::Min($all.Count, $LineNumber + $After)

  Add-Line ""
  Add-Line "===== $Path :: lines $start-$end (hit $LineNumber) ====="
  for ($n = $start; $n -le $end; $n++) {
    Add-Line ("{0,6}: {1}" -f $n, $all[$n - 1])
  }
}

Add-Line "=== SPLIT: focused Hyperline Freestyle source extraction ==="
Add-Line "Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Add-Line ""

$targets = foreach ($root in $roots) {
  Get-ChildItem -LiteralPath $root -Recurse -File -Force |
    Where-Object {
      $_.Name -in @("mod.js","mod.json","main.js") -or
      $_.Extension.ToLowerInvariant() -in ".js",".json",".ts"
    }
}

$patterns = @(
  "createParticle\s*\(",
  "CreateParticle",
  "ParticleOptions",
  "ParticlePoint",
  "ParticleAnchor",
  "tracer",
  "route.?line",
  "followed the route",
  "lineParticle",
  "laser",
  "vpcf",
  "resource\s*:",
  "control.?point",
  "anchors",
  "point\s*:",
  "ghost",
  "gates",
  "samples",
  "recording",
  "techniques\.ts",
  "var labels\s*=\s*\{",
  "var rules\s*=\s*\[",
  "cornerBoost",
  "mantleSlide",
  "slideHop",
  "dashCancel",
  "superglide",
  "mantleWallJump",
  "mantleStrafe",
  "edgeBoost"
)

$seen = @{}

# Capture the complete movement recognizer section, not only nearby pattern
# windows. This avoids dropping the middle of techniques.ts where the composed
# move predicates and rule table live.
foreach ($file in ($targets | Sort-Object FullName -Unique)) {
  if ($file.Name -ne "mod.js") { continue }
  $all = Get-Content -LiteralPath $file.FullName
  $startHit = Select-String -LiteralPath $file.FullName -Pattern "// techniques\.ts" |
    Select-Object -First 1
  if (-not $startHit) { continue }

  $endHit = Select-String -LiteralPath $file.FullName -Pattern "// boxes\.ts" |
    Where-Object { $_.LineNumber -gt $startHit.LineNumber } |
    Select-Object -First 1
  if (-not $endHit) { continue }

  Add-Line ""
  Add-Line "################################################################"
  Add-Line "# COMPLETE TECHNIQUES BLOCK: $($file.FullName)"
  Add-Line "################################################################"
  for ($n = $startHit.LineNumber; $n -lt $endHit.LineNumber; $n++) {
    Add-Line ("{0,6}: {1}" -f $n, $all[$n - 1])
  }
}

foreach ($file in ($targets | Sort-Object FullName -Unique)) {
  $matches = Select-String -LiteralPath $file.FullName -Pattern $patterns -CaseSensitive:$false
  if (-not $matches) { continue }

  Add-Line ""
  Add-Line "################################################################"
  Add-Line "# FILE: $($file.FullName)"
  Add-Line "################################################################"

  foreach ($m in $matches) {
    # Avoid near-duplicate windows by bucketing hits into 80-line regions.
    $bucket = [Math]::Floor(($m.LineNumber - 1) / 80)
    $key = "$($file.FullName)|$bucket"
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    Add-ContextBlock -Path $file.FullName -LineNumber $m.LineNumber -Before 35 -After 100
  }
}

Add-Line ""
Add-Line "=== Focused extraction complete ==="
$lines | Set-Content -LiteralPath $report -Encoding UTF8

Write-Host ""
Write-Host "=== Focused Hyperline Freestyle extraction complete ==="
Write-Host "Full focused report:"
Write-Host "  $report"
Write-Host ""
Write-Host "Send that file to ChatGPT."
