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
