param(
    [Parameter(Mandatory = $true)]
    [string]$DeadlockDir
)

$ErrorActionPreference = "Stop"

$RuntimeDir = Split-Path -Parent $PSScriptRoot
$ToolsDir = Join-Path $RuntimeDir "build-tools"
$WorkDir = Join-Path $RuntimeDir "panorama-build"
$OutputDir = Join-Path $RuntimeDir "panorama-package"
$Modlock = Join-Path $ToolsDir "modlock.exe"
$ModlockUrl = "https://github.com/paralin/modlock/releases/download/v0.1.55/modlock-windows-amd64.exe"

New-Item -ItemType Directory -Force -Path $ToolsDir | Out-Null
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

if (-not (Test-Path $Modlock)) {
    Write-Host "[SPLIT][Panorama] downloading Modlock Panorama compiler..."
    Invoke-WebRequest -Uri $ModlockUrl -OutFile $Modlock
}

Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $WorkDir "panorama/layout/split_movement") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $WorkDir "panorama/scripts/split_movement") | Out-Null

$Archive = Join-Path $DeadlockDir "game/citadel/pak01_dir.vpk"
$LatestNews = Join-Path $WorkDir "panorama/layout/latest_news.xml"
$HeroTesting = Join-Path $WorkDir "panorama/layout/hud_hero_testing.xml"

Write-Host "[SPLIT][Panorama] decompiling current Deadlock latest_news layout..."
$xml = & $Modlock panorama decompile --archive $Archive panorama/layout/latest_news.vxml_c
if ($LASTEXITCODE -ne 0 -or -not $xml) {
    throw "Could not decompile latest_news.vxml_c"
}
$xmlText = ($xml -join [Environment]::NewLine)

$bootInclude = '<include src="file://{resources}/scripts/split_movement/boot.js" />'
$tab = [char]9
if ($xmlText.Contains("<scripts>")) {
    $xmlText = $xmlText.Replace("<scripts>", "<scripts>" + [Environment]::NewLine + $tab + $tab + $bootInclude)
} elseif ($xmlText.Contains("<root>")) {
    $xmlText = $xmlText.Replace("<root>", "<root>" + [Environment]::NewLine + $tab + "<scripts>" + $bootInclude + "</scripts>")
} else {
    throw "latest_news.xml has no root/scripts element"
}

Set-Content -Path $LatestNews -Value $xmlText -Encoding UTF8

Write-Host "[SPLIT][Panorama] hiding stock Hero Testing tools for Movement Lab..."
$heroXml = & $Modlock panorama decompile --archive $Archive panorama/layout/hud_hero_testing.vxml_c
if ($LASTEXITCODE -eq 0 -and $heroXml) {
    $heroText = ($heroXml -join [Environment]::NewLine)
    $heroText = $heroText.Replace(
        '<CitadelHudHeroTesting class="hud_hero_testing_root">',
        '<CitadelHudHeroTesting class="hud_hero_testing_root" style="visibility: collapse;">'
    )
    Set-Content -Path $HeroTesting -Value $heroText -Encoding UTF8
} else {
    Write-Host "[SPLIT][Panorama] warning: could not patch hud_hero_testing; stock tools may remain visible"
}

Copy-Item (Join-Path $RuntimeDir "panorama-src/layout/split_movement/bootstrap.xml") (Join-Path $WorkDir "panorama/layout/split_movement/bootstrap.xml")
Copy-Item (Join-Path $RuntimeDir "panorama-src/layout/split_movement/movement_lab.xml") (Join-Path $WorkDir "panorama/layout/split_movement/movement_lab.xml")
Copy-Item (Join-Path $RuntimeDir "panorama-src/scripts/split_movement/boot.js") (Join-Path $WorkDir "panorama/scripts/split_movement/boot.js")
Copy-Item (Join-Path $RuntimeDir "panorama-src/scripts/split_movement/movement_lab.js") (Join-Path $WorkDir "panorama/scripts/split_movement/movement_lab.js")

$Package = Join-Path $OutputDir "pak01_dir.vpk"
Remove-Item -Force $Package -ErrorAction SilentlyContinue

Write-Host "[SPLIT][Panorama] compiling temporary session package..."
& $Modlock panorama compile $WorkDir -o $Package
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $Package)) {
    throw "Panorama compilation failed"
}

Write-Host "[SPLIT][Panorama] package ready: $Package"
