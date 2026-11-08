use tauri::State;

use crate::cmd_log;
use crate::content_xml::{self, ContentChange, ContentReport};
use crate::AppState;

/// (07/11) Etapa F — lo que dice `Content.xml` del usuario de MSFS elegido.
#[tauri::command]
pub async fn content_report(state: State<'_, AppState>, user: Option<String>) -> Result<ContentReport, String> {
    content_xml::report(&state.db, &state.data_dir, user).await.map_err(|e| e.to_string())
}

/// Aplica los cambios marcados. `stamp` es la huella del archivo que viste.
#[tauri::command]
pub async fn content_apply(
    state: State<'_, AppState>,
    user: String,
    stamp: String,
    changes: Vec<ContentChange>,
) -> Result<usize, String> {
    cmd_log!("content_apply", "{user}: {} cambios", changes.len());
    content_xml::apply(&state.db, &state.data_dir, user, stamp, changes)
        .await
        .map_err(|e| e.to_string())
}

/// Vuelve a una copia de seguridad.
#[tauri::command]
pub async fn content_restore(state: State<'_, AppState>, user: String, file: String) -> Result<(), String> {
    cmd_log!("content_restore", "{user}: {file}");
    content_xml::restore(&state.db, &state.data_dir, user, file).await.map_err(|e| e.to_string())
}
