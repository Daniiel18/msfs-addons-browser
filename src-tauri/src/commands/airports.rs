use crate::airports::{self, AddonOnMap, AirportBrief};
use crate::AppState;
use tauri::Manager;

/// (24/10) Tarjeta de aeropuerto del FlightBook (clic en el mapa).
#[tauri::command]
pub async fn airport_card(
    icao: String,
    state: tauri::State<'_, AppState>,
) -> Result<crate::airport_card::AirportCard, String> {
    crate::airport_card::card(&state.db, &icao)
        .await
        .map_err(|e| e.to_string())
}

/// (24/10) Foto del aeropuerto (Wikipedia, en caché local). Ruta del archivo
/// o `None` si no hay foto.
#[tauri::command]
pub async fn airport_photo(
    icao: String,
    name: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    // El entorno de pruebas guarda la caché en su propio dir de datos.
    let base = if crate::test_env::is_active() {
        crate::test_env::data_dir().map_err(|e| e.to_string())?
    } else {
        app.path().app_local_data_dir().map_err(|e| e.to_string())?
    };
    let dir = base.join("airport-photos");
    crate::airport_card::photo(&state.http, &dir, &icao, name.as_deref())
        .await
        .map(|p| p.map(|p| p.to_string_lossy().into_owned()))
        .map_err(|e| e.to_string())
}

/// (v4.11.0) Resuelve nombre/ciudad/país de uno o varios ICAO. Lo usa el
/// FlightBook para mostrar el aeropuerto bajo el código de origen/destino.
#[tauri::command]
pub async fn lookup_airports(
    icaos: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AirportBrief>, String> {
    airports::lookup_airports(&state.db, &icaos)
        .await
        .map_err(|e| e.to_string())
}

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
