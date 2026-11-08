//! (03/11) Etapa 7 — comandos de los modos de lanzamiento y los conjuntos.

use tauri::State;

use crate::dispatch::{self, ApplyReport, DispatchChanges, DispatchState, DispatchTrip, LiveryEntry, SceneryChoice};
use crate::AppState;

/// (05/11) Qué escenario usar en un aeropuerto con varios encendidos.
#[tauri::command]
pub async fn dispatch_set_scenery(
    icao: String,
    folder: Option<String>,
    ignored: bool,
    state: State<'_, AppState>,
) -> Result<Vec<SceneryChoice>, String> {
    dispatch::set_scenery_choice(&state.db, &icao, folder.as_deref(), ignored)
        .await
        .map_err(|e| e.to_string())
}

/// Modo, «Nunca apagar», conjuntos y lo pendiente de restaurar.
#[tauri::command]
pub async fn dispatch_state(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<DispatchState, String> {
    let st = dispatch::state(&state.db).await.map_err(|e| e.to_string())?;
    if st.pending_restore.is_some() {
        dispatch::ensure_restore_watch(app, state.db.clone());
    }
    Ok(st)
}

#[tauri::command]
pub async fn dispatch_set_mode(
    state: State<'_, AppState>,
    mode: String,
    set_id: Option<i64>,
) -> Result<(), String> {
    dispatch::set_mode(&state.db, &mode, set_id).await.map_err(|e| e.to_string())
}

/// Liveries instaladas con su matrícula.
#[tauri::command]
pub async fn dispatch_liveries(state: State<'_, AppState>) -> Result<Vec<LiveryEntry>, String> {
    dispatch::liveries(&state.db).await.map_err(|e| e.to_string())
}

/// Incluir o no el escenario del alternativo.
#[tauri::command]
pub async fn dispatch_set_alternate(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    dispatch::set_use_alternate(&state.db, on).await.map_err(|e| e.to_string())
}

/// (04/11) Guarda el viaje de varias paradas (`None` lo quita).
#[tauri::command]
pub async fn dispatch_set_trip(
    state: State<'_, AppState>,
    trip: Option<DispatchTrip>,
) -> Result<Option<DispatchTrip>, String> {
    dispatch::set_trip(&state.db, trip).await.map_err(|e| e.to_string())
}

/// Marca o quita un addon de «Nunca apagar». Devuelve la lista nueva.
#[tauri::command]
pub async fn dispatch_set_keep(
    state: State<'_, AppState>,
    folder: String,
    keep: bool,
) -> Result<Vec<String>, String> {
    dispatch::set_keep(&state.db, &folder, keep).await.map_err(|e| e.to_string())
}

/// Aplica cambios confirmados sin lanzar nada.
#[tauri::command]
pub async fn dispatch_apply(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    changes: DispatchChanges,
) -> Result<ApplyReport, String> {
    dispatch::apply_and_notify(&app, &state.db, changes).await.map_err(|e| e.to_string())
}

/// Guarda lo encendido ahora como conjunto nuevo.
#[tauri::command]
pub async fn content_set_save(state: State<'_, AppState>, name: String) -> Result<DispatchState, String> {
    dispatch::save_set(&state.db, &name).await.map_err(|e| e.to_string())?;
    dispatch::state(&state.db).await.map_err(|e| e.to_string())
}

/// Sustituye un conjunto por lo que hay encendido ahora.
#[tauri::command]
pub async fn content_set_update(state: State<'_, AppState>, id: i64) -> Result<DispatchState, String> {
    dispatch::update_set(&state.db, id).await.map_err(|e| e.to_string())?;
    dispatch::state(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn content_set_rename(
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<DispatchState, String> {
    dispatch::rename_set(&state.db, id, &name).await.map_err(|e| e.to_string())?;
    dispatch::state(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn content_set_delete(state: State<'_, AppState>, id: i64) -> Result<DispatchState, String> {
    dispatch::delete_set(&state.db, id).await.map_err(|e| e.to_string())?;
    dispatch::state(&state.db).await.map_err(|e| e.to_string())
}
