//! Comandos Tauri para la sincronización con Google Drive.
//!
//! Mapean 1:1 con funciones de `crate::cloud_sync`. Sólo se ocupan de:
//!   · Aceptar parámetros del frontend.
//!   · Mapear errores a `Result<_, String>` (Tauri serializa).
//!   · Loguear via `CmdTimer` para diagnóstico.

use crate::cloud_sync::{
    self, CloudConfig, CloudTestReport, DownloadReport, FolderSyncConfig,
    FolderSyncLoadReport, FolderSyncSaveReport, OauthStart, SyncReport, UploadReport,
};
use crate::logger::CmdTimer;
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn cloud_get_config(
    state: tauri::State<'_, AppState>,
) -> Result<CloudConfig, String> {
    cloud_sync::get_config(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_set_credentials(
    client_id: String,
    client_secret: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("cloud_set_credentials", "stored client_id ({} chars)", client_id.len());
    cloud_sync::set_credentials(&state.db, &client_id, &client_secret)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_start_oauth(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<OauthStart, String> {
    let _t = CmdTimer::start("cloud_start_oauth");
    cloud_sync::start_oauth(state.db.clone(), state.http.clone(), app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_disconnect(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("cloud_disconnect", "");
    cloud_sync::disconnect(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_sync_now(
    state: tauri::State<'_, AppState>,
) -> Result<SyncReport, String> {
    let _t = CmdTimer::start("cloud_sync_now");
    cloud_sync::sync_now(&state.db, &state.http)
        .await
        .map_err(|e| e.to_string())
}

/// (v3.5.0 F3) Subir TODO el contenido local a la nube. Sobreescribe
/// el snapshot remoto con el estado actual (todos los vuelos, todos
/// los tracks, todos los settings con `pref_*`).
#[tauri::command]
pub async fn cloud_upload_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<UploadReport, String> {
    let _t = CmdTimer::start("cloud_upload_all");
    cloud_sync::upload_all(&state.db, &state.http, Some(&app))
        .await
        .map_err(|e| e.to_string())
}

/// (v3.5.0 F3) Bajar SOLO los vuelos faltantes del cloud. Los locales
/// existentes NO se sobreescriben. Auto-cierra cualquier vuelo bajado
/// con `ended_at = NULL` para evitar el bug del "FLYING NOW phantom".
#[tauri::command]
pub async fn cloud_download_missing(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<DownloadReport, String> {
    let _t = CmdTimer::start("cloud_download_missing");
    cloud_sync::download_missing(&state.db, &state.http, Some(&app))
        .await
        .map_err(|e| e.to_string())
}

/// (v2.0.2) Diagnóstico paso a paso. No falla nunca — devuelve el
/// reporte con los pasos completados/fallidos para que el frontend
/// los muestre al usuario.
#[tauri::command]
pub async fn cloud_test_connection(
    state: tauri::State<'_, AppState>,
) -> Result<CloudTestReport, String> {
    let _t = CmdTimer::start("cloud_test_connection");
    Ok(cloud_sync::test_connection(&state.db, &state.http).await)
}

/// (v3.5.0) Purga el snapshot del cloud — borra el archivo de Drive
/// y resetea `last_sync_at`. Los vuelos locales NO se tocan. Devuelve
/// los conteos del snapshot remoto antes de borrarlo para feedback
/// honesto al usuario.
#[tauri::command]
pub async fn cloud_purge(
    state: tauri::State<'_, AppState>,
) -> Result<cloud_sync::PurgeReport, String> {
    let _t = CmdTimer::start("cloud_purge");
    cloud_sync::purge(&state.db, &state.http)
        .await
        .map_err(|e| e.to_string())
}

// ─── (v2.0.1) Folder sync — alternativa simple a OAuth ────────────────────────

#[tauri::command]
pub async fn folder_sync_get_config(
    state: tauri::State<'_, AppState>,
) -> Result<FolderSyncConfig, String> {
    cloud_sync::get_folder_config(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn folder_sync_save(
    folder_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FolderSyncSaveReport, String> {
    let _t = CmdTimer::start("folder_sync_save");
    cmd_log!("folder_sync_save", "folder={}", folder_path);
    cloud_sync::save_to_folder(&state.db, &folder_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn folder_sync_load(
    folder_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FolderSyncLoadReport, String> {
    let _t = CmdTimer::start("folder_sync_load");
    cmd_log!("folder_sync_load", "folder={}", folder_path);
    cloud_sync::load_from_folder(&state.db, &folder_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn folder_sync_clear(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("folder_sync_clear", "");
    cloud_sync::set_folder_path(&state.db, "")
        .await
        .map_err(|e| e.to_string())
}
