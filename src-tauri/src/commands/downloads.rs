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
    replaces: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DownloadJob, String> {
    let addon_simulator = addon_simulator.unwrap_or_default();
    cmd_log!(
        "start_download",
        "addon={} title={:?} sim={:?} source={} method={:?} url={} replaces={:?}",
        addon_id, addon_title, addon_simulator, source, method.kind, method.url, replaces
    );
    let result = state
        .downloads
        .start(
            addon_id.clone(),
            addon_title.clone(),
            source.clone(),
            method.clone(),
            addon_simulator,
            replaces,
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
/// (02/11) Con «installed», `packages` son las carpetas que quedaron en
/// Community: se apunta qué versión del catálogo se instaló en cada una.
/// (04/11) Y si la descarga sustituía a otra carpeta, ahora se quita. Con
/// «installed», `message` es lo que no se pudo instalar: entonces no se quita.
#[tauri::command]
pub async fn download_job_status(
    key: String,
    status: String,
    message: Option<String>,
    packages: Option<Vec<String>>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use crate::download::manager::HosterStatus;
    let id = key.strip_prefix("hoster-dl-").unwrap_or(&key).to_string();
    let st = match status.as_str() {
        "processing" => HosterStatus::Processing,
        "ready" => HosterStatus::Ready,
        "installed" => HosterStatus::Installed,
        "saved" => HosterStatus::Saved,
        "failed" => HosterStatus::Failed(message.clone().unwrap_or_default()),
        "cancelled" => HosterStatus::Cancelled,
        other => return Err(format!("estado desconocido: {other}")),
    };
    state.downloads.hoster_status(&id, st).await;
    if status == "installed" {
        if let Some(job) = state.downloads.job(&id).await {
            let packages = packages.unwrap_or_default();
            crate::updates::record_installed_packages(
                &state.db,
                &job.addon_id,
                Some(&job.source),
                &packages,
            )
            .await;
            state.downloads.finish_replace(&id, &packages, message).await;
        }
    }
    Ok(())
}
