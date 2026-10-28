$ErrorActionPreference = "SilentlyContinue"

$roots = @(
  "$env:APPDATA\Hyperline",
  "$env:LOCALAPPDATA\Hyperline",
  "C:\Program Files\Hyperline",
  "C:\Program Files (x86)\Steam\steamapps\common\Deadlock\game\citadel\addons"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

Write-Host "=== SPLIT: Hyperline/Freestyle asset scan ==="
Write-Host "Roots:"
$roots | ForEach-Object { Write-Host "  $_" }

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

Write-Host ""
Write-Host "=== Candidate files ==="
$files |
  Where-Object {
    $_.FullName -match "freestyle|tracer|route|movement|particle" -or
    $_.Extension -in ".vpcf", ".vpcf_c"
  } |
  ForEach-Object {
    Write-Host $_.FullName
  }

Write-Host ""
Write-Host "=== Text hits: tracer / particle / route / vpcf ==="

$textFiles = $files | Where-Object {
  $_.Extension.ToLowerInvariant() -in ".json", ".js", ".ts", ".mjs", ".cjs", ".txt", ".cfg", ".kv3", ".vdata"
}

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
    Write-Host ""
    Write-Host "--- $($file.FullName) ---"
    $matches | Select-Object -First 40 | ForEach-Object {
      Write-Host ("{0}:{1}: {2}" -f $_.Path, $_.LineNumber, $_.Line.Trim())
    }
  }
}

Write-Host ""
Write-Host "=== Archive/package candidates ==="
$files |
  Where-Object { $_.Extension.ToLowerInvariant() -in ".vpk", ".zip" } |
  ForEach-Object { Write-Host $_.FullName }

Write-Host ""
Write-Host "=== Scan complete ==="
