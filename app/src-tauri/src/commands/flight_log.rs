//! Comandos Tauri para `flight_log`.
//!
//! Expuestos al frontend para listar el historial, borrar una
//! entrada manual, y dejar un hook de "test entry" que ayuda a
//! validar la UI mientras la integración real con SimConnect no
//! está cableada.

use crate::flight_log::{self, FlightLogEntry};
use crate::simconnect_watcher::FlightStatus;
use crate::AppState;
use tauri::Manager;

#[tauri::command]
pub async fn list_flight_log(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FlightLogEntry>, String> {
    flight_log::list_entries(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_flight_log_entry(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    flight_log::delete_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

/// Devuelve el estado actual del watcher: ¿MSFS corriendo? ¿qué
/// vuelo cuadra como "activo"? El watcher emite eventos cuando
/// cambia, pero el frontend también necesita un getter para el
/// montaje inicial.
#[tauri::command]
pub async fn get_flight_status(app: tauri::AppHandle) -> Result<FlightStatus, String> {
    let state = app
        .try_state::<crate::simconnect_watcher::SharedState>()
        .ok_or_else(|| "watcher state no inicializado".to_string())?;
    let guard = state.lock().await;
    Ok(guard.status.clone())
}

/// Helper de testing: inserta un vuelo simulado EBBR → LEMD para
/// que la UI pueda pintar el línea verde sin requerir que MSFS
/// esté corriendo. Se llama desde la consola de devtools mientras
/// la integración real con SimConnect aún no está activa.
#[tauri::command]
pub async fn debug_seed_flight_log(
    state: tauri::State<'_, AppState>,
) -> Result<i64, String> {
    let id = flight_log::start_flight(
        &state.db,
        50.901,
        4.484,
        Some("Airbus A320 Demo"),
        Some("A320"),
    )
    .await
    .map_err(|e| e.to_string())?;
    flight_log::finish_flight(&state.db, id, 40.471, -3.564, Some(36000))
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}
