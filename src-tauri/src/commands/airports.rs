use crate::airports::{self, AddonOnMap};
use crate::AppState;

/// Devuelve los addons del catálogo local que tienen ICAO con coords
/// resolvibles. Es lo que pinta el mapa.
///
/// Si la tabla `airports` está vacía (primer arranque, descarga aún
/// en curso) este comando devuelve `[]` — la UI muestra "cargando
/// dataset" hasta que el background fetch termine.
#[tauri::command]
pub async fn list_addons_on_map(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AddonOnMap>, String> {
    airports::list_addons_on_map(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Fuerza una recarga del dataset de aeropuertos. Pensado para botón
/// «refrescar mapa» en la UI o para tests; en el flujo normal el
/// dataset se sincroniza solo al arrancar (con TTL de 30 días).
#[tauri::command]
pub async fn refresh_airports_dataset(
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    airports::ensure_dataset(&state.db, &state.http)
        .await
        .map_err(|e| e.to_string())?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM airports")
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(count as usize)
}
