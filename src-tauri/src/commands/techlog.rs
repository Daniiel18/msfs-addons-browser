//! (19/10) Etapa 2 — comandos de la sincronía con Fenix/PMDG TechLog.

use tauri::State;

use crate::techlog::{
    self, TechLogAircraft, TechLogFlight, TlActionRequest, TlActionResult, TlAppStatus, TlCabin, TlPendingOrder,
    TlSource,
};
use crate::AppState;

/// Apps TechLog instaladas / abiertas y cuántos aviones conoce cada una.
#[tauri::command]
pub async fn techlog_status() -> Result<Vec<TlAppStatus>, String> {
    Ok(techlog::status().await)
}

/// Estado TechLog de una matrícula (`null` si ningún TechLog la conoce).
#[tauri::command]
pub async fn techlog_aircraft(registration: String) -> Result<Option<TechLogAircraft>, String> {
    techlog::aircraft(&registration).await.map_err(|e| e.to_string())
}

/// Matrículas (normalizadas) que gestiona TechLog → SimFleet no les aplica su
/// propio desgaste.
#[tauri::command]
pub async fn techlog_managed_registrations() -> Result<Vec<String>, String> {
    let mut v: Vec<String> = techlog::managed_registrations().await.into_keys().collect();
    v.sort();
    Ok(v)
}

/// Acción de mantenimiento en TechLog (reparar, diferir, servicio, check…).
/// (20/10) Si TechLog está cerrado lo abre; si no deja elegir el avión (el
/// sim tiene otro cargado) la orden queda en cola (`queued: true`).
#[tauri::command]
pub async fn techlog_action(
    state: State<'_, AppState>,
    request: TlActionRequest,
) -> Result<TlActionResult, String> {
    techlog::action(&state.db, request)
        .await
        .map_err(|e| e.to_string())
}

/// (20/10) Órdenes en cola de una matrícula.
#[tauri::command]
pub async fn techlog_pending(
    state: State<'_, AppState>,
    registration: String,
) -> Result<Vec<TlPendingOrder>, String> {
    techlog::pending(&state.db, &registration)
        .await
        .map_err(|e| e.to_string())
}

/// (20/10) Quita una orden de la cola.
#[tauri::command]
pub async fn techlog_cancel_pending(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    techlog::cancel_pending(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

/// (20/10) Abre la app TechLog (minimizada). `false` si ya estaba abierta.
#[tauri::command]
pub async fn techlog_launch(source: String) -> Result<bool, String> {
    let source = TlSource::parse(&source)
        .filter(|s| *s != TlSource::Simfleet)
        .ok_or_else(|| "not_installed".to_string())?;
    techlog::launch(source).await.map_err(|e| e.to_string())
}

/// (23/10) Cabina del Fenix cargado (TechLog 2.1+). `null` si esa matrícula
/// no es el avión activo de un TechLog abierto.
#[tauri::command]
pub async fn techlog_cabin(registration: String) -> Result<Option<TlCabin>, String> {
    techlog::cabin(&registration).await.map_err(|e| e.to_string())
}

/// (23/10) Luces de cabina (`control: "lights"`, value "Bright"|"Dim1"|"Dim2"|"Off")
/// o pre-select de agua (`"preselect"`, value 0–100 en pasos de 25).
#[tauri::command]
pub async fn techlog_cabin_set(
    registration: String,
    control: String,
    value: serde_json::Value,
) -> Result<TlActionResult, String> {
    techlog::cabin_set(&registration, &control, &value)
        .await
        .map_err(|e| e.to_string())
}

/// Vuelo de TechLog que corresponde a un vuelo del FlightBook.
#[tauri::command]
pub async fn techlog_flight(
    registration: String,
    started_at: String,
    ended_at: Option<String>,
) -> Result<Option<TechLogFlight>, String> {
    techlog::flight_for(&registration, &started_at, ended_at.as_deref())
        .await
        .map_err(|e| e.to_string())
}
