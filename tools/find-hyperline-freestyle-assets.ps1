$ErrorActionPreference = "SilentlyContinue"

$repoRoot = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $repoRoot "hyperline-scan"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$report = Join-Path $outDir "hyperline-freestyle-scan.txt"

$roots = @(
  "$env:APPDATA\Hyperline",
  "$env:LOCALAPPDATA\Hyperline",
  "C:\Program Files\Hyperline",
  "C:\Program Files (x86)\Hyperline",
  "C:\Program Files (x86)\Steam\steamapps\common\Deadlock\game\citadel\addons"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

$lines = New-Object System.Collections.Generic.List[string]
function Add-Line([string]$text = "") {
  $lines.Add($text)
}

Add-Line "=== SPLIT: Hyperline/Freestyle asset scan ==="
Add-Line "Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Add-Line ""
Add-Line "Roots:"
$roots | ForEach-Object { Add-Line "  $_" }

$extensions = @(
  ".vpcf", ".vpcf_c", ".vpk", ".zip",
  ".json", ".js", ".ts", ".mjs", ".cjs",
  ".txt", ".cfg", ".kv3", ".vdata", ".vdata_c"
)

$files = foreach ($root in $roots) {
  Get-ChildItem -LiteralPath $root -Recurse -File -Force |
    Where-Object {
      $name = $_.Name.ToLowerInvariant()
      $ext = $_.Extension.ToLowerInvariant()
      $extensions -contains $ext -or
      $name -match "freestyle|tracer|route|movement|particle|hyperline"
    }
}

$files = $files | Sort-Object FullName -Unique

$candidates = $files | Where-Object {
  $_.FullName -match "freestyle|tracer|route|movement|particle" -or
  $_.Extension -in ".vpcf", ".vpcf_c"
}

Add-Line ""
Add-Line "=== Candidate files ==="
$candidates | ForEach-Object { Add-Line $_.FullName }

$textFiles = $files | Where-Object {
  $_.Extension.ToLowerInvariant() -in ".json", ".js", ".ts", ".mjs", ".cjs", ".txt", ".cfg", ".kv3", ".vdata"
}

Add-Line ""
Add-Line "=== Text hits: tracer / particle / route / vpcf ==="

$hitCount = 0
foreach ($file in $textFiles) {
  $matches = Select-String -LiteralPath $file.FullName -Pattern @(
    "tracer",
    "vpcf",
    "particle",
    "route",
    "ghost",
    "control.?point",
    "world.?effect"
  ) -CaseSensitive:$false

  if ($matches) {
    Add-Line ""
    Add-Line "--- $($file.FullName) ---"
    foreach ($m in ($matches | Select-Object -First 80)) {
      Add-Line ("{0}:{1}: {2}" -f $m.Path, $m.LineNumber, $m.Line.Trim())
      $hitCount++
    }
  }
}

Add-Line ""
Add-Line "=== Archive/package candidates ==="
$archives = $files | Where-Object { $_.Extension.ToLowerInvariant() -in ".vpk", ".zip" }
$archives | ForEach-Object { Add-Line $_.FullName }

Add-Line ""
Add-Line "=== Summary ==="
Add-Line "Candidate files: $($candidates.Count)"
Add-Line "Text hits: $hitCount"
Add-Line "Archives: $($archives.Count)"
Add-Line ""
Add-Line "=== Scan complete ==="

$lines | Set-Content -LiteralPath $report -Encoding UTF8

Write-Host ""
Write-Host "=== SPLIT Hyperline scan complete ==="
Write-Host "Candidate files : $($candidates.Count)"
Write-Host "Text hits       : $hitCount"
Write-Host "Archives        : $($archives.Count)"
Write-Host ""
Write-Host "Full report saved to:"
Write-Host "  $report"
Write-Host ""
Write-Host "Open it with:"
Write-Host "  notepad \"$report\""
