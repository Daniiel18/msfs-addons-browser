//! (v8.3 · Etapa 6.2a) Comandos Tauri de los vuelos reales de OpenSky.

use crate::aircraft_db::{self, AirlineFleet};
use crate::opensky::{self, OpenSkyStatus, SyncReport};
use crate::AppState;

/// (v8.3 · Etapa 6.2b) Flota real de una aerolínea (vistos volando en 30
/// días + aviones del usuario).
#[tauri::command]
pub async fn airline_fleet(
    state: tauri::State<'_, AppState>,
    airline_icao: String,
) -> Result<AirlineFleet, String> {
    aircraft_db::fleet(&state.db, &airline_icao)
        .await
        .map_err(|e| e.to_string())
}

/// Estado de la conexión y de la descarga de vuelos reales.
#[tauri::command]
pub async fn opensky_status(state: tauri::State<'_, AppState>) -> Result<OpenSkyStatus, String> {
    opensky::status(&state.db).await.map_err(|e| e.to_string())
}

/// Baja ya las ventanas pendientes (sin esperar a la sincronía de 30 min).
#[tauri::command]
pub async fn opensky_sync_now(state: tauri::State<'_, AppState>) -> Result<SyncReport, String> {
    opensky::sync(&state.db, &state.http)
        .await
        .map_err(|e| e.to_string())
}
