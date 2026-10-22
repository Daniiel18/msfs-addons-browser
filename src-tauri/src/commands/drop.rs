//! Comandos Tauri para drag-and-drop universal (v2.1.0).
//!
//! Mapean directamente a funciones de `crate::drop_install`. La
//! sesión + tempdir vive en `AppState::drop_sessions` (Mutex).

use std::path::PathBuf;

use crate::community;
use crate::drop_install::{self, DropCommitReport, DropInspection, SavedExternal};
use crate::logger::CmdTimer;
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn drop_inspect(
    archive_path: String,
    password: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DropInspection, String> {
    let _t = CmdTimer::start("drop_inspect");
    cmd_log!("drop_inspect", "archive={} pw={}", archive_path, password.is_some());
    let path = PathBuf::from(archive_path);
    let sessions: &drop_install::DropSessions = &state.drop_sessions;
    // `inspect` es sync (extrae a tempdir); lo corremos inline.
    drop_install::inspect(&path, password.as_deref(), sessions).map_err(|e| e.to_string())
}

/// (v8.3) Guarda un item `external_content` / `external_rest` (apps,
/// instaladores, navdata, lo que no va a MSFS) en la carpeta que eligió el
/// usuario. Extraer/copiar puede tardar con archivos grandes → hilo aparte.
/// La sesión sigue viva (el front la cancela o commitea lo demás después).
#[tauri::command]
pub async fn drop_save_external(
    session_id: String,
    source_path: String,
    dest_folder: String,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<SavedExternal, String> {
    let _t = CmdTimer::start("drop_save_external");
    cmd_log!("drop_save_external", "session={} dest={}", session_id, dest_folder);
    let job = drop_install::external_job(&session_id, &source_path, &state.drop_sessions)
        .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        drop_install::save_external(&job, &PathBuf::from(dest_folder), name.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| {
        tracing::error!(target: "drop", "drop_save_external falló: {e:#}");
        e.to_string()
    })
}

#[tauri::command]
pub async fn drop_commit(
    app: tauri::AppHandle,
    session_id: String,
    selected_paths: Vec<String>,
    community_path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DropCommitReport, String> {
    let _t = CmdTimer::start("drop_commit");
    cmd_log!(
        "drop_commit",
        "session={} selected={}",
        session_id,
        selected_paths.len()
    );
    // (v6.2.60) Capturamos el path del archivo original ANTES del commit (que
    // consume la sesión). Si vino de una descarga del embebido, lo usamos abajo
    // para registrar el tracking de updates de flightsim.to.
    let archive_path: Option<String> = state
        .drop_sessions
        .0
        .lock()
        .ok()
        .and_then(|m| m.get(&session_id).map(|s| s.archive_path.clone()));
    let community = match community_path {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(
            community::detect_community_folder()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| crate::tr!("dropCmd.communityNotDetected"))?
                .path,
        ),
    };
    // (v6.1) Versión de MSFS preferida → carpeta de estado (work) donde van los
    // configs de avión iFly/PMDG. Es DISTINTA por versión (2020 `Packages`, 2024
    // `WASM\MSFS2020`) y NO es la carpeta Community.
    let prefer_2024 = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = 'pref_sim_version'",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(|v| v.contains("2024"))
    .unwrap_or(false);
    let report = drop_install::commit(
        &session_id,
        &selected_paths,
        &community,
        prefer_2024,
        &state.drop_sessions,
    )
    .map_err(|e| e.to_string())?;

    // (v6.1 #37) Cross-link 2020/2024 también para drag&drop: si se instaló al
    // menos un paquete Community y el usuario tiene ambos sims, ofrecemos
    // enlazarlo a la otra Community (misma UX que la descarga).
    if !report.installed_packages.is_empty() {
        if let Some(offer) = crate::cross_link::build_offer_for_paths(&report.installed_packages) {
            use tauri::Emitter;
            let _ = app.emit("cross-link://offer", &offer);
        }
    }

    // (v6.2.60) Tracking de updates de flightsim.to: si esta instalación vino de
    // una descarga del embebido asociada a un archivo (file_id), registramos
    // cada carpeta instalada con su versión/updatedAt actuales (baseline).
    if let Some(ap) = archive_path {
        let file_id = state
            .livery_downloads
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&ap));
        if let Some(file_id) = file_id {
            // `installed_packages` + `installed_livery_packages` traen rutas
            // completas; el tracking se indexa por NOMBRE de carpeta
            // (= `CommunityPackage.folderName`) para casar el badge en Addons.
            // (v7.2.0) Las liveries instaladas por SimFleet son su PROPIO paquete
            // Community (`pmdg-livery-*` / `ifly-livery-*`, doguer27 addon linker),
            // así que se rastrean igual que cualquier otro paquete — con su
            // file_id de flightsim.to (fiable, sólo descargas del embebido).
            let folders: Vec<String> = report
                .installed_packages
                .iter()
                .chain(report.installed_livery_packages.iter())
                .filter_map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                })
                .collect();
            if !folders.is_empty() {
                // En background: hace un GET a flightsim.to para leer versión/fecha;
                // no debe retrasar la respuesta del install.
                let db = state.db.clone();
                let http = state.http.clone();
                tokio::spawn(async move {
                    crate::flightsim_track::record_tracked(&db, &http, &file_id, &folders).await;
                });
            }
        }
    }

    Ok(report)
}

#[tauri::command]
pub async fn drop_cancel(
    session_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    drop_install::cancel(&session_id, &state.drop_sessions);
    Ok(())
}

/// (v4.7.0) Borra el archivo comprimido ORIGINAL que el usuario arrastró
/// (.zip/.rar/.7z), tras una instalación exitosa y SOLO si el usuario lo
/// confirma en el modal. Borra en cualquier ruta del sistema (Descargas,
/// etc.). Para la inspección/extracción solo LEÍMOS el archivo y el handle
/// ya está cerrado a esta altura; aun así reintentamos un par de veces por
/// si un antivirus/indexador lo tiene con un lock de E/S transitorio.
#[tauri::command]
pub async fn delete_dropped_archive(archive_path: String) -> Result<(), String> {
    let _t = CmdTimer::start("delete_dropped_archive");
    cmd_log!("delete_dropped_archive", "archive={}", archive_path);
    let path = PathBuf::from(&archive_path);
    if !path.exists() {
        return Ok(()); // ya no está — nada que borrar
    }
    let mut last_err: Option<std::io::Error> = None;
    for attempt in 0..4u64 {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(target: "drop", "archivo original borrado: {}", archive_path);
                remove_hoster_job_dir(&path);
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(150 * (attempt + 1)))
                    .await;
            }
        }
    }
    let msg = crate::tr!(
        "dropCmd.deleteFailed",
        path = archive_path,
        e = last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| crate::tr!("dropCmd.unknown"))
    );
    tracing::warn!(target: "drop", "{}", msg);
    Err(msg)
}

/// (v8.3) Si el archivo era de una descarga de hoster (`…\simfleet-mirror-dl\
/// hoster-dl-<job>\archivo`), se borra también su carpeta: puede tener las
/// demás partes de un RAR multivolumen. Sólo esa forma exacta de ruta — nunca
/// una carpeta del usuario.
fn remove_hoster_job_dir(file: &std::path::Path) {
    let Some(dir) = file.parent() else { return };
    let is_job_dir = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("hoster-dl-"));
    let in_mirror = dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "simfleet-mirror-dl");
    if is_job_dir && in_mirror {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => tracing::info!(target: "drop", "carpeta de la descarga borrada: {}", dir.display()),
            Err(e) => tracing::warn!(target: "drop", "no se pudo borrar {}: {e}", dir.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::remove_hoster_job_dir;

    #[test]
    fn only_the_hoster_job_folder_is_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let job = tmp.path().join("simfleet-mirror-dl").join("hoster-dl-abc");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("simfleet-addon.part2.rar"), b"x").unwrap();
        remove_hoster_job_dir(&job.join("simfleet-addon.part1.rar"));
        assert!(!job.exists(), "carpeta del job (con las otras partes) borrada");

        // Una carpeta del usuario con el mismo nombre de archivo: intacta.
        let user = tmp.path().join("Downloads").join("hoster-dl-abc");
        std::fs::create_dir_all(&user).unwrap();
        remove_hoster_job_dir(&user.join("addon.zip"));
        assert!(user.exists());
    }
}
