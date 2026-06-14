//! (v5.0.0) Comandos del motor de rendimiento (FPS) por aeropuerto.
//!
//! El frontend pasa `install_path` (la carpeta del addon en Community)
//! directamente — siempre lo tiene en `CommunityPackage.installPath` —
//! así no hace falta volver a detectar la carpeta Community aquí.

use std::path::PathBuf;

use crate::logger::CmdTimer;
use crate::perf_config::{self, PerfConfig, ToggleResult};
use crate::{cmd_log, AppState};

/// Lee (o genera por escaneo local) el manifiesto de rendimiento del
/// addon. `None` si el escenario no tiene objetos opcionales.
#[tauri::command]
pub async fn perf_read_config(
    install_path: String,
    folder_name: String,
    icao: Option<String>,
) -> Result<Option<PerfConfig>, String> {
    cmd_log!("perf_read_config", "folder={folder_name}");
    let _t = CmdTimer::start("perf_read_config");
    let dir = PathBuf::from(&install_path);
    tokio::task::spawn_blocking(move || perf_config::read_or_generate(&dir, &folder_name, icao))
        .await
        .map_err(|e| format!("la tarea de lectura falló: {e}"))
}

/// Aplica un toggle: renombra los `.bgl`↔`.bgl.off` de la opción de
/// forma atómica con rollback. `enable=false` desactiva (gana FPS).
#[tauri::command]
pub async fn perf_toggle_option(
    install_path: String,
    option_id: String,
    enable: bool,
) -> Result<ToggleResult, String> {
    cmd_log!("perf_toggle_option", "opt={option_id} enable={enable}");
    let _t = CmdTimer::start("perf_toggle_option");
    let dir = PathBuf::from(&install_path);
    tokio::task::spawn_blocking(move || perf_config::apply_toggle(&dir, &option_id, enable))
        .await
        .map_err(|e| format!("la tarea de toggle falló: {e}"))?
        .map_err(|e| format!("{e:#}"))
}

/// Enriquece el manifiesto bajando la nota "Optional Configuration" de
/// la página de SceneryAddons del aeropuerto y fusionándola con el
/// escaneo local. Best-effort: si la red falla, cae al escaneo local.
#[tauri::command]
pub async fn perf_enrich_from_source(
    install_path: String,
    folder_name: String,
    icao: Option<String>,
    page_url: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<PerfConfig>, String> {
    cmd_log!("perf_enrich_from_source", "folder={folder_name} url={page_url}");
    let _t = CmdTimer::start("perf_enrich_from_source");

    let html = match state.http.get(&page_url).send().await {
        Ok(resp) => match resp.error_for_status() {
            Ok(ok) => ok.text().await.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("perf: página {page_url} devolvió error: {e}");
                String::new()
            }
        },
        Err(e) => {
            tracing::warn!("perf: no se pudo bajar {page_url}: {e}");
            String::new()
        }
    };

    let dir = PathBuf::from(&install_path);
    tokio::task::spawn_blocking(move || {
        if html.is_empty() {
            perf_config::read_or_generate(&dir, &folder_name, icao)
        } else {
            perf_config::enrich_with_html(&dir, &folder_name, icao, &html)
        }
    })
    .await
    .map_err(|e| format!("la tarea de enriquecimiento falló: {e}"))
}
