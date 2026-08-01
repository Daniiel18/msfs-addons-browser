# Build de RELEASE limpio para SimFleet.
#
# Por qué: cada compilación deja artefactos en src-tauri/target (deps/ con
# hashes viejos, .lib/.pdb de 2GB+, bundles). Entre muchos builds eso acumula
# DECENAS de GB. Este script borra los artefactos viejos ANTES de compilar
# (conservando SIEMPRE las carpetas data/ con la DB del usuario), compila el
# instalador NSIS, y deja SÓLO el instalador de esta versión.
#
# Uso:  powershell -File scripts/build-release.ps1
# (Los builds de desarrollo con `npm run tauri dev` NO usan esto — mantienen el
#  caché incremental para compilar rápido.)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    Write-Host "→ Limpiando artefactos viejos (conservando data/)..." -ForegroundColor Cyan
    # Borra target_check si un check ad-hoc lo dejó.
    Remove-Item "$root/src-tauri/target_check" -Recurse -Force -ErrorAction SilentlyContinue
    # En cada perfil de target/, borra todo MENOS la carpeta data/ (DB + settings).
    Get-ChildItem "$root/src-tauri/target" -Directory -ErrorAction SilentlyContinue | ForEach-Object {
        Get-ChildItem $_.FullName -Force -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -ne "data" } |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "→ Compilando instalador NSIS..." -ForegroundColor Cyan
    npm run tauri:build -- --bundles nsis

    $nsis = "$root/src-tauri/target/release/bundle/nsis"
    $latest = Get-ChildItem "$nsis/*_x64-setup.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    # Borra instaladores viejos, deja sólo el más nuevo.
    Get-ChildItem "$nsis/*_x64-setup.exe" -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -ne $latest.FullName } |
        Remove-Item -Force -ErrorAction SilentlyContinue

    if ($latest) {
        Write-Host "✓ Instalador: $($latest.FullName)" -ForegroundColor Green
    }
}
finally {
    Pop-Location
}
