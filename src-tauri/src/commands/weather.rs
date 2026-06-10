//! (v4.16.0 #5a) Comandos del módulo de clima (Weather).

/// Devuelve la API key de OpenWeather embebida en build-time desde
/// `secrets.local.toml` (vía `build.rs` → `option_env!`). El frontend la
/// usa para los tiles meteo del mapa (clouds / precipitation / wind …).
/// Devuelve `None` si no se configuró la key al compilar — el mapa cae a
/// solo el basemap sin capa meteo.
#[tauri::command]
pub fn get_openweather_key() -> Option<String> {
    option_env!("SIMFLEET_OPENWEATHER_API_KEY")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
