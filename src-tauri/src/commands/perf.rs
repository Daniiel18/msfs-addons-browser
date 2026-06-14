//! (v5.0.0) Comandos del motor de rendimiento (FPS) por aeropuerto.
//!
//! El frontend pasa `install_path` (la carpeta del addon en Community)
//! directamente — siempre lo tiene en `CommunityPackage.installPath`.
//!
//! Clave: para bajar la nota "Optional Configuration" correcta hay que
//! resolver la página del DEVELOPER INSTALADO (p. ej. el ENGM de
//! Aerosoft, no el de JustSim ni Orbx). Lo hacemos cruzando el
//! `developer` del resultado de SceneryAddons con el `creator` del
//! paquete y los tokens del `folderName`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{stream, StreamExt};

use crate::logger::CmdTimer;
use crate::perf_config::{self, PerfConfig, ToggleResult};
use crate::sources::{Addon, Source};
use crate::{cmd_log, AppState};

/// Item para los escaneos masivos.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfScanItem {
    pub folder_name: String,
    pub install_path: String,
    #[serde(default)]
    pub icao: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
}

/// Concurrencia del escaneo masivo contra SceneryAddons. Modesto para no
/// martillar el servidor con ~150 aeropuertos.
const SCAN_CONCURRENCY: usize = 4;

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// ¿El resultado de SceneryAddons corresponde al dev instalado? Cruza el
/// `developer` del resultado con el `creator` del paquete y el `folder`.
fn dev_matches(addon: &Addon, creator: &str, folder: &str) -> bool {
    let dev = addon.developer.as_deref().unwrap_or("");
    let rd = norm(dev);
    if rd.len() < 3 {
        return false;
    }
    let cr = norm(creator);
    let fd = norm(folder);
    if !cr.is_empty() && (rd.contains(&cr) || cr.contains(&rd)) {
        return true;
    }
    // Primer token del developer ("Aerosoft") presente en folder/creator.
    let first = norm(dev.split_whitespace().next().unwrap_or(""));
    if first.len() >= 3 && (fd.contains(&first) || cr.contains(&first)) {
        return true;
    }
    rd.len() >= 3 && fd.contains(&rd)
}

/// Resuelve la URL de la página del aeropuerto del DEV instalado. Si hay
/// varios devs para el ICAO y ninguno coincide, devuelve `None` (no
/// adivinamos — bajar la nota equivocada da opciones incorrectas).
async fn resolve_page_url(
    source: &Arc<dyn Source>,
    icao: &str,
    creator: &str,
    folder: &str,
) -> Option<String> {
    let addons = match source.search(icao).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("perf: search {icao} falló: {e}");
            return None;
        }
    };
    let hits: Vec<&Addon> = addons
        .iter()
        .filter(|a| {
            !a.page_url.is_empty()
                && a.icao
                    .as_deref()
                    .map(|c| c.eq_ignore_ascii_case(icao))
                    .unwrap_or(false)
        })
        .collect();
    // 1) Coincidencia por developer.
    if let Some(a) = hits.iter().find(|a| dev_matches(a, creator, folder)) {
        return Some(a.page_url.clone());
    }
    // 2) Un único resultado para el ICAO → sin ambigüedad, úsalo.
    if hits.len() == 1 {
        return Some(hits[0].page_url.clone());
    }
    // 3) Varios devs y ninguno coincide → no adivinar.
    None
}

async fn fetch_html(http: &reqwest::Client, url: &str) -> String {
    match http.get(url).send().await {
        Ok(resp) => match resp.error_for_status() {
            Ok(ok) => ok.text().await.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("perf: {url} devolvió error: {e}");
                String::new()
            }
        },
        Err(e) => {
            tracing::warn!("perf: no se pudo bajar {url}: {e}");
            String::new()
        }
    }
}

/// Devuelve los `folderName` que YA tienen una `config/simfleet_perf.json`
/// con opciones reales (de la página o tras un toggle) — para el badge.
/// NO hace escaneo permisivo.
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

/// (Escáner masivo) Para cada aeropuerto instalado con ICAO, resuelve la
/// página del DEV instalado en SceneryAddons, baja la nota, la parsea y
/// escribe la config SÓLO si la página la trae. Devuelve los `folderName`
/// que quedaron optimizables (para sus badges). Concurrencia limitada.
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
            let creator = it.creator.clone().unwrap_or_default();
            let page_url = resolve_page_url(&source, &icao, &creator, &it.folder_name).await?;
            let html = fetch_html(&http, &page_url).await;
            if html.is_empty() {
                return None;
            }
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

/// Aplica un toggle: renombra los archivos de la opción de forma atómica
/// con rollback. `enable=false` desactiva (gana FPS).
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

/// (Botón del modal) Enriquece la config bajando la nota del DEV
/// instalado en SceneryAddons. Resuelve la página por `creator`+`folder`.
/// Si no hay coincidencia de dev/red, cae al escaneo local.
#[tauri::command]
pub async fn perf_enrich_from_source(
    install_path: String,
    folder_name: String,
    icao: Option<String>,
    creator: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<PerfConfig>, String> {
    cmd_log!("perf_enrich_from_source", "folder={folder_name}");
    let _t = CmdTimer::start("perf_enrich_from_source");

    let html = match state.source("sceneryaddons") {
        Some(source) => {
            let icao_str = icao.clone().unwrap_or_default();
            let creator_str = creator.clone().unwrap_or_default();
            if icao_str.len() == 4 {
                match resolve_page_url(&source, &icao_str, &creator_str, &folder_name).await {
                    Some(url) => fetch_html(&state.http, &url).await,
                    None => String::new(),
                }
            } else {
                String::new()
            }
        }
        None => String::new(),
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
