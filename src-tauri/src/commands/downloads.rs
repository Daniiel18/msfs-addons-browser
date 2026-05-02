use crate::download::DownloadJob;
use crate::sources::DownloadMethod;
use crate::AppState;

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
    state: tauri::State<'_, AppState>,
) -> Result<DownloadJob, String> {
    tracing::info!(
        "cmd:start_download addon={} source={} method={:?}",
        addon_id, source, method.kind
    );
    state
        .downloads
        .start(addon_id, addon_title, source, method)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.downloads.cancel(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.downloads.pause(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_download(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.downloads.resume(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_download(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.downloads.clear(&id).await;
    Ok(())
}
