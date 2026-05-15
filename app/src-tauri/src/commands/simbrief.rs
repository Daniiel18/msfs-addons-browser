use crate::simbrief::{self, SimBriefFlight, SimBriefRefreshResult};
use crate::AppState;

#[tauri::command]
pub async fn get_simbrief_pilot_id(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    simbrief::get_pilot_id(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_simbrief_pilot_id(
    pilot_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let trimmed = pilot_id.trim();
    if trimmed.is_empty() {
        return Err("El pilot_id no puede estar vacío".into());
    }
    simbrief::set_pilot_id(&state.db, trimmed)
        .await
        .map_err(|e| e.to_string())
}

/// Descarga el último OFP de SimBrief y lo persiste si es nuevo.
/// Si no hay `simbrief_pilot_id` configurado en settings, devuelve
/// error para que la UI pida al usuario que lo configure.
#[tauri::command]
pub async fn refresh_simbrief(
    state: tauri::State<'_, AppState>,
) -> Result<SimBriefRefreshResult, String> {
    let pilot_id = simbrief::get_pilot_id(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "Configura tu SimBrief Pilot ID antes de refrescar.".to_string()
        })?;
    simbrief::refresh_latest(&state.db, &state.http, &pilot_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_simbrief_flights(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SimBriefFlight>, String> {
    simbrief::list_flights(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_simbrief_flight(
    ofp_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    simbrief::delete_flight(&state.db, &ofp_id)
        .await
        .map_err(|e| e.to_string())
}
