use crate::db::repo::{self, InstalledAddonRow};
use crate::AppState;

#[tauri::command]
pub async fn list_installed(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledAddonRow>, String> {
    repo::list_installed(&state.db).await.map_err(|e| e.to_string())
}
