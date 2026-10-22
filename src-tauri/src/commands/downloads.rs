use crate::download::DownloadJob;
use crate::sources::DownloadMethod;
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn list_downloads(state: tauri::State<'_, AppState>) -> Result<Vec<DownloadJob>, String> {
    Ok(state.downloads.list().await)
}

#[tauri::command]
pub async fn start_download(
    addon_id: String,
    addon_title: String,
    source: String,
    method: DownloadMethod,
    #[allow(non_snake_case)] addon_simulator: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DownloadJob, String> {
    let addon_simulator = addon_simulator.unwrap_or_default();
    cmd_log!(
        "start_download",
        "addon={} title={:?} sim={:?} source={} method={:?} url={}",
        addon_id, addon_title, addon_simulator, source, method.kind, method.url
    );
    let result = state
        .downloads
        .start(
            addon_id.clone(),
            addon_title.clone(),
            source.clone(),
            method.clone(),
            addon_simulator,
        )
        .await
        .map_err(|e| {
            tracing::error!(target: "download", "start_download error: {e:#}");
            e.to_string()
        })?;
    tracing::info!(
        target: "download",
        "start_download OK — job_id={} addon={}",
        result.id,
        addon_id
    );
    Ok(result)
}

#[tauri::command]
pub async fn cancel_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("cancel_download", "id={}", id);
    state.downloads.cancel(&id).await.map_err(|e| {
        tracing::error!(target: "download", "cancel_download {} error: {e:#}", id);
        e.to_string()
    })
}

#[tauri::command]
pub async fn pause_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("pause_download", "id={}", id);
    state.downloads.pause(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("resume_download", "id={}", id);
    state.downloads.resume(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_download(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    cmd_log!("clear_download", "id={}", id);
    state.downloads.clear(&id).await;
    Ok(())
}

/// (v8.3) El frontend cuenta qué pasa con una descarga ya bajada mientras la
/// procesa (extraer → listo para el usuario → instalado / guardado / error),
/// para verlo en el panel de Descargas. `key` es la ventana de hoster
/// (`hoster-dl-<job>`) o el id del job (descargas de flightsim.to).
#[tauri::command]
pub async fn download_job_status(
    key: String,
    status: String,
    message: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use crate::download::manager::HosterStatus;
    let id = key.strip_prefix("hoster-dl-").unwrap_or(&key).to_string();
    let st = match status.as_str() {
        "processing" => HosterStatus::Processing,
        "ready" => HosterStatus::Ready,
        "installed" => HosterStatus::Installed,
        "saved" => HosterStatus::Saved,
        "failed" => HosterStatus::Failed(message.unwrap_or_default()),
        "cancelled" => HosterStatus::Cancelled,
        other => return Err(format!("estado desconocido: {other}")),
    };
    state.downloads.hoster_status(&id, st).await;
    Ok(())
}
