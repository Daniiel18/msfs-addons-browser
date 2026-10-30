use crate::airports::{self, AddonOnMap, AirportBrief};
use crate::AppState;
use tauri::Manager;

/// (24/10) Tarjeta de aeropuerto del FlightBook (clic en el mapa).
/// (26/10) `flight_ids`: los vuelos de la aerolínea filtrada en el FlightBook
/// (el filtro se resuelve en el frontend, con la misma clave que la lista).
#[tauri::command]
pub async fn airport_card(
    icao: String,
    flight_ids: Option<Vec<i64>>,
    state: tauri::State<'_, AppState>,
) -> Result<crate::airport_card::AirportCard, String> {
    let only: Option<std::collections::HashSet<i64>> = flight_ids.map(|v| v.into_iter().collect());
    crate::airport_card::card(&state.db, &icao, only.as_ref())
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
    // (26/10) v2: fotos filtradas (sin logos, mapas ni diagramas). La caché
    // vieja pudo guardar alguno de esos; se borra una vez.
    let old = base.join("airport-photos");
    if old.is_dir() {
        let _ = std::fs::remove_dir_all(&old);
    }
    let dir = base.join("airport-photos-v2");
    // Sólo el ICAO suele caer en una página de desambiguación ("KJFK"): sin
    // nombre (la precarga del mapa) se toma el de la base de aeropuertos.
    let name = match name.filter(|n| !n.trim().is_empty()) {
        Some(n) => Some(n),
        None => airports::lookup_airports(&state.db, std::slice::from_ref(&icao))
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
            .map(|a| a.name),
    };
    crate::airport_card::photo(&state.http, &dir, &icao, name.as_deref())
        .await
        .map(|p| p.map(|p| p.to_string_lossy().into_owned()))
        .map_err(|e| e.to_string())
}

/// (26/10) Pistas y helipuertos del aeropuerto (OurAirports). Si el listado
/// aún no se bajó (primer arranque), se baja ahora.
#[tauri::command]
pub async fn airport_runways(
    icao: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::runways::Runway>, String> {
    if let Err(e) = crate::runways::ensure_dataset(&state.db, &state.http).await {
        tracing::warn!("runways: {e:#}");
    }
    crate::runways::of_airport(&state.db, &icao)
        .await
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
