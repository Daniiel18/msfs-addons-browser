//! (02/11) Etapa 6 — comandos de la secuencia de arranque.

use tauri::State;

use crate::dispatch::DispatchChanges;
use crate::launch_flow::{self, DetectedApp, LaunchRun, LaunchSequence, LaunchStep, SimInfo};
use crate::AppState;

/// Las secuencias (la primera vez, con las de ejemplo).
#[tauri::command]
pub async fn launch_sequences(state: State<'_, AppState>) -> Result<Vec<LaunchSequence>, String> {
    launch_flow::list(&state.db).await.map_err(|e| e.to_string())
}

/// Guarda una secuencia y devuelve la lista nueva.
#[tauri::command]
pub async fn launch_sequence_save(
    state: State<'_, AppState>,
    sequence: LaunchSequence,
) -> Result<Vec<LaunchSequence>, String> {
    launch_flow::save(&state.db, sequence).await.map_err(|e| e.to_string())?;
    launch_flow::list(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn launch_sequence_delete(state: State<'_, AppState>, id: i64) -> Result<Vec<LaunchSequence>, String> {
    launch_flow::delete(&state.db, id).await.map_err(|e| e.to_string())?;
    launch_flow::list(&state.db).await.map_err(|e| e.to_string())
}

/// Apps de simulación encontradas en el PC.
#[tauri::command]
pub async fn launch_detected_apps() -> Result<Vec<DetectedApp>, String> {
    tokio::task::spawn_blocking(launch_flow::detect_apps)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn launch_sim_info() -> Result<SimInfo, String> {
    Ok(launch_flow::sim_info().await)
}

/// Arranca una secuencia (`test_all` = «Probar todo»). `changes`: addons ya
/// confirmados, que se aplican antes de abrir nada.
#[tauri::command]
pub async fn launch_run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    sequence_id: i64,
    test_all: bool,
    changes: Option<DispatchChanges>,
) -> Result<LaunchRun, String> {
    launch_flow::run(app, &state.db, sequence_id, test_all, changes)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn launch_status() -> Option<LaunchRun> {
    launch_flow::status()
}

#[tauri::command]
pub fn launch_cancel() {
    launch_flow::cancel();
}

/// «Seguir ya»: deja de esperar en el paso actual.
#[tauri::command]
pub fn launch_skip_wait() {
    launch_flow::skip_wait();
}

/// «Probar»: abre sólo ese paso, ya.
#[tauri::command]
pub async fn launch_test_step(state: State<'_, AppState>, step: LaunchStep) -> Result<String, String> {
    launch_flow::test_step(&state.db, step).await.map(str::to_string).map_err(|e| e.to_string())
}

/// (07/11) Etapa F: los scripts de estos pasos que aún no tienen tu permiso.
#[tauri::command]
pub async fn launch_scripts_pending(
    state: State<'_, AppState>,
    steps: Vec<LaunchStep>,
) -> Result<Vec<launch_flow::ScriptPermission>, String> {
    Ok(launch_flow::scripts_pending(&state.db, &steps).await)
}

/// Das permiso a esas versiones de los scripts.
#[tauri::command]
pub async fn launch_scripts_approve(state: State<'_, AppState>, hashes: Vec<String>) -> Result<(), String> {
    launch_flow::approve_scripts(&state.db, hashes).await.map_err(|e| e.to_string())
}
