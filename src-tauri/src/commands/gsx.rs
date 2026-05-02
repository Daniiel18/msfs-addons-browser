use crate::gsx::{self, GsxProfile};
use crate::AppState;

/// Devuelve los perfiles GSX Pro disponibles para un ICAO (vacío si no
/// hay coincidencias). El comando es idempotente y respeta el caché de
/// 24h en SQLite — una segunda llamada con el mismo ICAO no toca la
/// red. ICAOs vacíos o blancos se cortocircuitan a `[]`.
#[tauri::command]
pub async fn gsx_lookup(
    icao: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GsxProfile>, String> {
    let trimmed = icao.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    gsx::lookup_with_cache(&state.gsx, &state.db, trimmed)
        .await
        .map_err(|e| e.to_string())
}
