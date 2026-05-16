use std::path::PathBuf;

use crate::community::{self, CommunityInfo};
use crate::db::repo;
use crate::install::{self, InstallResult};
use crate::logger::CmdTimer;
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn community_folder() -> Result<Option<CommunityInfo>, String> {
    // Pure sync I/O — safe to call from async without spawn_blocking
    // because it's a handful of file-existence checks + a regex.
    community::detect_community_folder().map_err(|e| e.to_string())
}

/// Extract an archive and install the contained MSFS packages into the
/// Community folder. `community_path` is optional — if omitted, we use
/// [`community::detect_community_folder`].
///
/// Runs on a blocking thread since [`zip::ZipArchive`] and
/// [`std::fs`] are synchronous.
///
/// Tras una instalación correcta, registra cada paquete en
/// `installed_addons` para que aparezca en la pestaña «Instalados»
/// aunque no provenga de un torrent (instalación manual desde un
/// archivo local elegido por el usuario).
#[tauri::command]
pub async fn install_archive(
    archive_path: String,
    community_path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<InstallResult, String> {
    cmd_log!(
        "install_archive",
        "archive={} community_path={:?}",
        archive_path,
        community_path
    );
    let _t = CmdTimer::start("install_archive");
    let community = match community_path {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(
            community::detect_community_folder()
                .map_err(|e| {
                    tracing::error!(target: "install", "detect_community_folder error: {e:#}");
                    e.to_string()
                })?
                .ok_or_else(|| {
                    tracing::error!(target: "install", "Community folder no detectada");
                    "No se detectó la carpeta Community automáticamente — configúrala desde Ajustes."
                        .to_string()
                })?
                .path,
        ),
    };
    tracing::info!(target: "install", "destino Community: {}", community.display());

    // Lee setting de override de PMDG OC para que install_livery sepa
    // dónde buscarlo cuando el usuario lo tiene en un path no estándar
    // (p.ej. portable en D:\Downloads\PMDG Operations Center\).
    let pmdg_oc_path: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = 'pmdg_oc_path' AND value IS NOT NULL AND value <> ''",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    if let Some(p) = &pmdg_oc_path {
        tracing::info!(target: "install", "pmdg_oc_path override del usuario: {}", p);
    }

    let archive = PathBuf::from(&archive_path);
    let archive_for_task = archive.clone();
    let community_for_task = community.clone();
    let oc_for_task = pmdg_oc_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        install::install_archive(&archive_for_task, &community_for_task, oc_for_task.as_deref())
    })
    .await
    .map_err(|e| {
        tracing::error!(target: "install", "tarea de instalación panic: {e}");
        format!("la tarea de instalación falló: {}", e)
    })?
    .map_err(|e| {
        tracing::error!(target: "install", "install_archive error: {e:#}");
        e.to_string()
    })?;
    tracing::info!(
        target: "install",
        "install_archive: {} paquetes, {} bytes, ptp={}, installer={}",
        result.packages.len(),
        result.total_bytes,
        result.ptp_payload.is_some(),
        result.installer_payload.is_some()
    );

    // Título "humano" derivado del nombre del archivo cuando es una
    // instalación manual: el usuario va a ver esto en la lista de
    // instalados, así que `bravoairspace-mdsd.rar` es preferible al
    // path absoluto.
    let archive_title = archive
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| archive_path.clone());

    for pkg in &result.packages {
        let rec = repo::InstallRecord {
            id: uuid::Uuid::new_v4().to_string(),
            // Sin `addon_id`: este paquete no proviene del catálogo
            // — la migración 002 hizo opcional este campo justo para
            // este caso.
            addon_id: None,
            source: None,
            title: archive_title.clone(),
            name: Some(pkg.name.clone()),
            developer: None,
            version: None,
            install_path: pkg.install_path.clone(),
            size_bytes: Some(pkg.size_bytes as i64),
        };
        if let Err(e) = repo::record_install(&state.db, &rec).await {
            tracing::warn!(
                "no se pudo registrar instalación manual en DB ({}): {e:#}",
                pkg.name
            );
        }
    }

    Ok(result)
}

/// Quita una fila del historial de instalaciones. No borra nada del
/// disco — el usuario puede limpiar la carpeta del paquete por
/// separado si quiere desinstalarlo de verdad.
#[tauri::command]
pub async fn forget_install(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::forget_install(&state.db, &id)
        .await
        .map_err(|e| e.to_string())
}

/// Diagnóstico: dónde está PMDG Operations Center.exe en este
/// sistema, qué paths se intentaron y cuál coincidió. El frontend
/// lo invoca desde Settings cuando el usuario quiere ver por qué
/// la detección automática falló.
///
/// Devuelve los paths SIEMPRE — el usuario los necesita aunque
/// la detección haya tenido éxito (para saber dónde apuntar el
/// setting si quiere cambiar a otra instancia de OC).
#[tauri::command]
pub async fn diagnose_pmdg_oc(
    state: tauri::State<'_, AppState>,
) -> Result<crate::install::pmdg::OcDetectionReport, String> {
    let setting: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = 'pmdg_oc_path' AND value IS NOT NULL AND value <> ''",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    Ok(crate::install::pmdg::diagnose_oc_detection(setting.as_deref()))
}
