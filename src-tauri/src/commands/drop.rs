//! Comandos Tauri para drag-and-drop universal (v2.1.0).
//!
//! Mapean directamente a funciones de `crate::drop_install`. La
//! sesión + tempdir vive en `AppState::drop_sessions` (Mutex).

use std::path::PathBuf;

use crate::community;
use crate::drop_install::{self, DropCommitReport, DropInspection};
use crate::logger::CmdTimer;
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn drop_inspect(
    archive_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DropInspection, String> {
    let _t = CmdTimer::start("drop_inspect");
    cmd_log!("drop_inspect", "archive={}", archive_path);
    let path = PathBuf::from(archive_path);
    // `inspect` puede tardar varios segundos para un .rar grande —
    // lo movemos a spawn_blocking para no bloquear el runtime async.
    let sessions: &drop_install::DropSessions = &state.drop_sessions;
    // Workaround para mover el &DropSessions a spawn_blocking sin
    // mover `state`: hacemos la operación inline (es sync).
    drop_install::inspect(&path, sessions).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn drop_commit(
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
    let community = match community_path {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(
            community::detect_community_folder()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Community folder no detectada".to_string())?
                .path,
        ),
    };
    drop_install::commit(&session_id, &selected_paths, &community, &state.drop_sessions)
        .map_err(|e| e.to_string())
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
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(150 * (attempt + 1)))
                    .await;
            }
        }
    }
    let msg = format!(
        "No se pudo borrar {}: {}",
        archive_path,
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "desconocido".to_string())
    );
    tracing::warn!(target: "drop", "{}", msg);
    Err(msg)
}
