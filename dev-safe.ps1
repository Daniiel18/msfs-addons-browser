# Lanza SimFleet en MODO SEGURO (instancia de pruebas) junto a la principal.
#
# SIMFLEET_SAFE_MODE  → NO auto-sincroniza a la nube y SALTA el single-instance
#                       (por eso SÍ abre aunque la app principal esté corriendo).
# SIMFLEET_ENABLE_WATCHER → FUERZA el watcher de SimConnect en la instancia
#                       safe para poder PROBAR Best Landings (grabación al
#                       aterrizar). La nube SIGUE apagada. La DB de esta
#                       instancia (dev, target\debug\data) es independiente de
#                       la app instalada, así que no pisa datos reales.
#
# Uso:  .\dev-safe.ps1
$env:SIMFLEET_SAFE_MODE = "1"
$env:SIMFLEET_ENABLE_WATCHER = "1"
Write-Host "==> SimFleet MODO SEGURO (watcher ON para pruebas / nube OFF / 2a instancia OK)" -ForegroundColor Yellow
npm run tauri dev
