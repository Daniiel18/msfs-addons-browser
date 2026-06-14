//! (v5.0.0) Comandos del motor de rendimiento (FPS) por aeropuerto.
//!
//! El frontend pasa `install_path` (la carpeta del addon en Community)
//! directamente — siempre lo tiene en `CommunityPackage.installPath` —
//! así no hace falta volver a detectar la carpeta Community aquí.

use std::path::{Path, PathBuf};

use futures_util::{stream, StreamExt};

use crate::logger::CmdTimer;
use crate::perf_config::{self, PerfConfig, ToggleResult};
use crate::{cmd_log, AppState};

/// Item para el escaneo masivo de "optimizables".
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfScanItem {
    pub folder_name: String,
    pub install_path: String,
    #[serde(default)]
    pub icao: Option<String>,
}

/// Concurrencia del escaneo masivo contra SceneryAddons. Modesto para no
/// martillar el servidor con ~150 aeropuertos.
const SCAN_CONCURRENCY: usize = 4;

/// Devuelve los `folderName` que YA tienen una `config/simfleet_perf.json`
/// con opciones reales (de la página de SceneryAddons o tras un toggle)
/// — para pintar el badge de la tuerca. NO hace escaneo permisivo.
#[tauri::command]
pub async fn perf_list_optimizable(items: Vec<PerfScanItem>) -> Result<Vec<String>, String> {
    cmd_log!("perf_list_optimizable", "n={}", items.len());
    let _t = CmdTimer::start("perf_list_optimizable");
    tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .filter(|it| perf_config::has_optional_objects(Path::new(&it.install_path)))
            .map(|it| it.folder_name)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| format!("la tarea de escaneo falló: {e}"))
}

/// (Escáner masivo) Para cada aeropuerto instalado con ICAO, busca su
/// página en SceneryAddons, baja la nota "Optional Configuration", la
/// parsea y escribe `config/simfleet_perf.json` SÓLO si la página trae
/// la nota. Devuelve los `folderName` que quedaron con optimización
/// (para activar sus badges). Concurrencia limitada.
#[tauri::command]
pub async fn perf_scan_all_from_source(
    items: Vec<PerfScanItem>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    cmd_log!("perf_scan_all_from_source", "n={}", items.len());
    let _t = CmdTimer::start("perf_scan_all_from_source");
    let source = state
        .source("sceneryaddons")
        .ok_or_else(|| "fuente SceneryAddons no disponible".to_string())?;
    let http = state.http.clone();

    let found: Vec<String> = stream::iter(
        items
            .into_iter()
            .filter(|it| it.icao.as_deref().map(|c| c.len() == 4).unwrap_or(false)),
    )
    .map(|it| {
        let source = source.clone();
        let http = http.clone();
        async move {
            let icao = it.icao.clone().unwrap_or_default();
            // 1) Resolver la página del aeropuerto en SceneryAddons.
            let page_url = match source.search(&icao).await {
                Ok(addons) => addons
                    .iter()
                    .find(|a| {
                        a.icao
                            .as_deref()
                            .map(|c| c.eq_ignore_ascii_case(&icao))
                            .unwrap_or(false)
                            && !a.page_url.is_empty()
                    })
                    .or_else(|| addons.iter().find(|a| !a.page_url.is_empty()))
                    .map(|a| a.page_url.clone()),
                Err(e) => {
                    tracing::warn!("perf scan: search {icao} falló: {e}");
                    None
                }
            };
            let page_url = page_url?;
            // 2) Bajar el HTML de la página.
            let html = match http.get(&page_url).send().await {
                Ok(r) => r.text().await.unwrap_or_default(),
                Err(_) => String::new(),
            };
            if html.is_empty() {
                return None;
            }
            // 3) Parsear + escribir SÓLO si hay nota real.
            let dir = PathBuf::from(&it.install_path);
            let folder = it.folder_name.clone();
            let icao_opt = it.icao.clone();
            let out_folder = it.folder_name.clone();
            let got = tokio::task::spawn_blocking(move || {
                perf_config::enrich_page_note_only(&dir, &folder, icao_opt, &html).is_some()
            })
            .await
            .unwrap_or(false);
            got.then_some(out_folder)
        }
    })
    .buffer_unordered(SCAN_CONCURRENCY)
    .filter_map(|x| async move { x })
    .collect()
    .await;

    tracing::info!(
        "perf scan: {} aeropuerto(s) con Optional Configuration",
        found.len()
    );
    Ok(found)
}

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
