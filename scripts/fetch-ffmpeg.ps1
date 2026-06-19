# Descarga el build estático de ffmpeg y deja ffmpeg.exe en
# src-tauri/resources/ffmpeg/ para que el instalador lo embeba (Best Landings).
#
# Uso (desde la raíz del repo):  pwsh scripts/fetch-ffmpeg.ps1
$ErrorActionPreference = "Stop"

$dest = Join-Path $PSScriptRoot "..\src-tauri\resources\ffmpeg"
$exe = Join-Path $dest "ffmpeg.exe"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

if (Test-Path $exe) {
    Write-Host "ffmpeg.exe ya existe en $dest — nada que hacer." -ForegroundColor Green
    exit 0
}

$url = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
$tmpZip = Join-Path $env:TEMP "ffmpeg-simfleet.zip"
$tmpDir = Join-Path $env:TEMP "ffmpeg-simfleet"

Write-Host "==> Descargando ffmpeg…" -ForegroundColor Cyan
Invoke-WebRequest -Uri $url -OutFile $tmpZip

Write-Host "==> Extrayendo…" -ForegroundColor Cyan
if (Test-Path $tmpDir) { Remove-Item -Recurse -Force $tmpDir }
Expand-Archive -Path $tmpZip -DestinationPath $tmpDir -Force

$found = Get-ChildItem -Path $tmpDir -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
if (-not $found) { throw "no se encontró ffmpeg.exe dentro del zip" }
Copy-Item $found.FullName $exe -Force

Remove-Item $tmpZip -Force -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue

Write-Host "==> Listo: $exe" -ForegroundColor Green
