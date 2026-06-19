# Lanza SimFleet en MODO SEGURO (instancia de pruebas) junto a la principal.
#
# Activa SIMFLEET_SAFE_MODE → NO conecta a SimConnect, NO auto-sincroniza a la
# nube y SALTA el single-instance (por eso SÍ abre aunque la app principal ya
# esté corriendo mientras vuelas). La ventana sale titulada "MODO SEGURO" con
# banner y badge rojos.
#
# Uso:  .\dev-safe.ps1
$env:SIMFLEET_SAFE_MODE = "1"
Write-Host "==> SimFleet MODO SEGURO (sin SimConnect / sin cloud / 2a instancia OK)" -ForegroundColor Red
npm run tauri dev
