//! (v8.3 · Etapa 6.1) Comandos Tauri de ubicación de aviones y traslados.

use crate::ferry::{self, AircraftLocation, Ferry, FerryQuote};
use crate::AppState;

/// Dónde está un avión (último aterrizaje o traslado) y si va en traslado.
#[tauri::command]
pub async fn aircraft_location(
    state: tauri::State<'_, AppState>,
    registration: String,
) -> Result<AircraftLocation, String> {
    ferry::location(&state.db, &registration)
        .await
        .map_err(|e| e.to_string())
}

/// Costo y duración de un traslado entre dos aeropuertos.
#[tauri::command]
pub async fn ferry_quote(
    state: tauri::State<'_, AppState>,
    from_icao: String,
    to_icao: String,
    model: Option<String>,
) -> Result<FerryQuote, String> {
    ferry::quote(&state.db, &from_icao, &to_icao, model.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Pide un traslado del avión a `to_icao` (desde donde esté).
#[tauri::command]
pub async fn ferry_request(
    state: tauri::State<'_, AppState>,
    registration: String,
    to_icao: String,
    model: Option<String>,
) -> Result<Ferry, String> {
    ferry::request(&state.db, &registration, &to_icao, model.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Cancela un traslado pedido que aún no ha llegado (no se cobra).
#[tauri::command]
pub async fn ferry_cancel(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    ferry::cancel(&state.db, id).await.map_err(|e| e.to_string())
}
