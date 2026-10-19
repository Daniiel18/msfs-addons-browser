# Lanza SimFleet como ENTORNO DE PRUEBAS, junto a la instancia principal
# (la instalada) sin mezclar datos con ella. Ver src-tauri/src/test_env.rs.
#
# SIMFLEET_SAFE_MODE → activa el entorno de pruebas y SALTA el single-instance
#                      (por eso SÍ abre aunque la app principal esté corriendo).
#
# Cada recurso compartido tiene su propio espacio de pruebas:
#   · Datos (DB, logs, temp): %LOCALAPPDATA%\SimFleet-Pruebas\data
#     (marcado con ENTORNO_DE_PRUEBAS.txt; la principal se niega a abrirlo)
#   · WebView2 (localStorage/cookies): <datos>\webview2
#   · Google Drive: msfs-addons-data.PRUEBAS.json  · Supabase: tablas *_test
#   · Autoarranque: «SimFleet Pruebas»  · Clips: Videos\SimFleet Landings - PRUEBAS
# NO aislados (a propósito): la carpeta Community real, SimConnect y Discord.
#
# SimConnect (FlightBook / Best Landings) está ACTIVO por defecto.
# Uso:  .\dev-safe.ps1               (SimConnect activo)
#       .\dev-safe.ps1 -SinSimConnect (no abre un 2º cliente de SimConnect)
param([switch]$SinSimConnect)

$env:SIMFLEET_SAFE_MODE = "1"
if ($SinSimConnect) { $env:SIMFLEET_DISABLE_WATCHER = "1" } else { Remove-Item Env:SIMFLEET_DISABLE_WATCHER -ErrorAction SilentlyContinue }
# rustup instala cargo aquí; la terminal puede no tenerlo aún en el PATH.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) { $env:PATH = "$cargoBin;$env:PATH" }

$estado = if ($SinSimConnect) { "SimConnect OFF" } else { "SimConnect ON" }
Write-Host "==> SimFleet ENTORNO DE PRUEBAS ($estado) — datos en $env:LOCALAPPDATA\SimFleet-Pruebas\data" -ForegroundColor Yellow
Set-Location $PSScriptRoot
npm run tauri dev
