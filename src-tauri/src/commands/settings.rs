//! Comandos de configuración persistente.
//!
//! Todo se guarda en la tabla `settings` (k/v). La tabla ya existe
//! desde la migración 001 y desde la 009 (`simbrief_pilot_id`);
//! aquí añadimos las claves nuevas:
//!
//!   · `pref_show_simconnect_lines`   — bool
//!   · `pref_check_updates_on_start`  — bool
//!   · `pref_minimize_to_tray`        — bool (Cierre = ocultar)
//!   · `pref_default_view`            — "dashboard" | "search" | "map" | "addons"
//!
//! El **autostart con Windows** lo gestiona `tauri-plugin-autostart`
//! escribiendo en `HKEY_CURRENT_USER\…\Run`. No persistimos el bool
//! en la DB porque el plugin ya lo lee del registro — la única
//! fuente de verdad.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;

/// Snapshot de todas las preferencias relevantes para la UI.
/// Devolvemos siempre **todas** las claves con sus defaults para
/// que el frontend no tenga que cablear "si está vacío usa X".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub show_simconnect_lines: bool,
    pub check_updates_on_start: bool,
    pub minimize_to_tray: bool,
    pub onboarding_completed: bool,
    pub default_view: String,
    /// Tema visual: "dark" (default) o "light". Controla la clase
    /// `dark` en `<html>` (Tailwind class-based) y el basemap del
    /// mapa de FlightBook.
    pub theme: String,
    /// (v3.1.0) Idioma de la UI: "auto" | "es" | "en". Default "auto".
    pub language: String,
    /// (v3.39.0 #3) Unidades POR CATEGORÍA. Cada magnitud se elige por
    /// separado; el preset Imperial/Métrico es sólo un atajo de UI. Los
    /// usuarios de v3.28–v3.38 (que sólo tenían `pref_unit_system`)
    /// migran cada categoría de ese valor — ver `unit_pref`.
    pub unit_weight: String,   // "kg" | "lb"
    pub unit_altitude: String, // "ft" | "m"
    pub unit_speed: String,    // "kt" | "kmh" | "mph"
    pub unit_vs: String,       // "fpm" | "ms"
    pub unit_distance: String, // "nm" | "km" | "mi"
    pub unit_pressure: String, // "inHg" | "hPa"
    /// (v3.28.0 P7.11) Unidad de temperatura: "C" | "F". Separada del
    /// sistema porque algunos usuarios imperiales quieren °C de todas
    /// formas (estándar METAR mundial). Default "C".
    pub temp_unit: String,
    pub autostart_enabled: bool,
    pub simbrief_pilot_id: Option<String>,
    /// Carpeta Community detectada — la mostramos en read-only para
    /// que el usuario sepa qué disco/path está leyendo la app.
    pub community_path: Option<String>,
    /// Path absoluto a la carpeta de logs (`<app_data>/logs/`).
    pub logs_path: Option<String>,
    /// Path al directorio de datos de la app (`<app_data>/`).
    pub app_data_path: Option<String>,
    /// (v5.1.0) Versión de MSFS activa: "msfs2020" | "msfs2024". Vacío =
    /// no elegida todavía → el frontend muestra el modal de elección al
    /// arrancar. Controla el filtro del catálogo de SceneryAddons y la
    /// preferencia de carpeta Community.
    pub sim_version: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            show_simconnect_lines: true,
            check_updates_on_start: true,
            minimize_to_tray: false,
            onboarding_completed: false,
            default_view: "dashboard".to_string(),
            theme: "dark".to_string(),
            language: "auto".to_string(),
            unit_weight: "lb".to_string(),
            unit_altitude: "ft".to_string(),
            unit_speed: "kt".to_string(),
            unit_vs: "fpm".to_string(),
            unit_distance: "nm".to_string(),
            unit_pressure: "inHg".to_string(),
            temp_unit: "C".to_string(),
            autostart_enabled: false,
            simbrief_pilot_id: None,
            community_path: None,
            logs_path: None,
            app_data_path: None,
            sim_version: String::new(),
        }
    }
}

#[tauri::command]
pub async fn get_app_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppSettings, String> {
    let kv = read_settings_map(&state.db).await.map_err(|e| e.to_string())?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let community_path = match crate::community::detect_community_folder() {
        Ok(Some(info)) if info.exists => Some(info.path),
        _ => None,
    };
    // (v3.2.0) Data path PORTABLE — resolvemos relativo al exe igual
    // que `init_state`. Si falla cae al `app_data_dir` del SO como
    // último recurso. Esto refleja en UI dónde están realmente los
    // datos (la carpeta de instalación, no `%APPDATA%`).
    let app_data_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("data")))
        .filter(|p| p.is_dir())
        .or_else(|| app.path().app_data_dir().ok())
        .map(|p| p.to_string_lossy().into_owned());
    let logs_path = app_data_path.as_ref().map(|p| {
        std::path::Path::new(p)
            .join("logs")
            .to_string_lossy()
            .into_owned()
    });
    Ok(AppSettings {
        show_simconnect_lines: as_bool(&kv, "pref_show_simconnect_lines", true),
        check_updates_on_start: as_bool(&kv, "pref_check_updates_on_start", true),
        minimize_to_tray: as_bool(&kv, "pref_minimize_to_tray", false),
        onboarding_completed: as_bool(&kv, "pref_onboarding_completed", false),
        default_view: kv
            .get("pref_default_view")
            .cloned()
            .unwrap_or_else(|| "dashboard".to_string()),
        theme: kv
            .get("pref_theme")
            .cloned()
            .unwrap_or_else(|| "dark".to_string()),
        language: kv
            .get("pref_language")
            .cloned()
            .unwrap_or_else(|| "auto".to_string()),
        // (v3.39.0 #3) Per-categoría, con migración del legado
        // `pref_unit_system` cuando una categoría no está seteada.
        unit_weight: unit_pref(&kv, "pref_unit_weight", &["kg", "lb"], "kg", "lb"),
        unit_altitude: unit_pref(&kv, "pref_unit_altitude", &["ft", "m"], "m", "ft"),
        unit_speed: unit_pref(&kv, "pref_unit_speed", &["kt", "kmh", "mph"], "kmh", "kt"),
        unit_vs: unit_pref(&kv, "pref_unit_vs", &["fpm", "ms"], "ms", "fpm"),
        unit_distance: unit_pref(&kv, "pref_unit_distance", &["nm", "km", "mi"], "km", "nm"),
        unit_pressure: unit_pref(&kv, "pref_unit_pressure", &["inHg", "hPa"], "hPa", "inHg"),
        temp_unit: kv
            .get("pref_temp_unit")
            .filter(|s| matches!(s.as_str(), "C" | "F"))
            .cloned()
            .unwrap_or_else(|| "C".to_string()),
        autostart_enabled,
        simbrief_pilot_id: kv
            .get("simbrief_pilot_id")
            .filter(|s| !s.trim().is_empty())
            .cloned(),
        community_path,
        logs_path,
        app_data_path,
        sim_version: kv.get("pref_sim_version").cloned().unwrap_or_default(),
    })
}

/// Setter genérico — el frontend pasa `key, value` y guardamos en
/// `settings`. Usado para los toggles booleanos y para `default_view`.
#[tauri::command]
pub async fn set_app_setting(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if !is_valid_key(&key) {
        return Err(format!("clave de setting no permitida: {key}"));
    }
    write_setting(&state.db, &key, &value)
        .await
        .map_err(|e| e.to_string())?;
    // Cuando el usuario toca «minimizar a bandeja», sincronizamos el
    // flag atómico que lee el handler de cierre de ventana. Sin esto,
    // el cambio sólo surtiría efecto al reiniciar la app.
    if key == "pref_minimize_to_tray" {
        let on = matches!(value.as_str(), "1" | "true" | "yes");
        state.minimize_to_tray.store(on, Ordering::Relaxed);
    }
    // (v5.1.0) Versión de MSFS: actualiza el flag atómico al instante
    // (aunque la UI pide reinicio para reaplicar Community/scan).
    if key == "pref_sim_version" {
        crate::sim::set_from_str(&value);
    }
    Ok(())
}

/// Limpia todas las claves `pref_*` — devuelve los toggles a sus
/// defaults. No toca `simbrief_pilot_id` ni el dataset de aeropuertos
/// (eso son datos, no preferencias).
#[tauri::command]
pub async fn reset_settings(state: tauri::State<'_, AppState>) -> Result<u64, String> {
    let r = sqlx::query("DELETE FROM settings WHERE key LIKE 'pref_%'")
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    // Reset también del flag atómico — defaults a false.
    state.minimize_to_tray.store(false, Ordering::Relaxed);
    Ok(r.rows_affected())
}

/// Toggle de autostart — delegamos al plugin para que escriba en
/// el registro. No tocamos DB; el plugin es la fuente de verdad.
#[tauri::command]
pub async fn set_autostart(
    enabled: bool,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let manager = app.autolaunch();
    let already = manager.is_enabled().unwrap_or(false);
    if enabled && !already {
        manager
            .enable()
            .map_err(|e| format!("no se pudo activar autostart: {e}"))?;
    } else if !enabled && already {
        manager
            .disable()
            .map_err(|e| format!("no se pudo desactivar autostart: {e}"))?;
    }
    Ok(manager.is_enabled().unwrap_or(false))
}

/// Limpia caches de update_check_cache + gsx_lookups. Útil cuando
/// el usuario sospecha que un valor cacheado está obsoleto y
/// quiere forzar refresh sin esperar al TTL.
#[tauri::command]
pub async fn clear_caches(state: tauri::State<'_, AppState>) -> Result<u64, String> {
    let pool = &state.db;
    let r1 = sqlx::query("DELETE FROM update_check_cache")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    let r2 = sqlx::query("DELETE FROM gsx_lookups")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(r1.rows_affected() + r2.rows_affected())
}

// -----------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------

fn is_valid_key(key: &str) -> bool {
    matches!(
        key,
        "pref_show_simconnect_lines"
            | "pref_check_updates_on_start"
            | "pref_minimize_to_tray"
            | "pref_onboarding_completed"
            | "pref_default_view"
            // `pref_theme`: "dark" | "light" — controla CSS class
            // en <html> (Tailwind class-based dark mode).
            | "pref_theme"
            // (v3.1.0) Idioma de la UI: "auto" | "es" | "en".
            | "pref_language"
            // (v3.39.0 #3) Unidades por categoría (+ temperatura).
            | "pref_unit_weight"
            | "pref_unit_altitude"
            | "pref_unit_speed"
            | "pref_unit_vs"
            | "pref_unit_distance"
            | "pref_unit_pressure"
            | "pref_temp_unit"
            // (v5.1.0) Versión de MSFS activa: "msfs2020" | "msfs2024".
            | "pref_sim_version"
            // (v6 #2b) Best Landings — config de grabación (heredada de
            // LandingToast, sin AutoLaunch que lo controlamos nosotros).
            | "rec_enabled"
            | "rec_osd_position"
            | "rec_output_path"
            | "rec_clip_seconds"
            | "rec_monitor_index"
            | "rec_source_type"
            | "rec_max_clips"
            | "rec_ffmpeg_path"
            | "rec_unlimited"
            | "rec_audio_device"
            | "rec_microphone"
    )
}

fn as_bool(kv: &HashMap<String, String>, key: &str, default: bool) -> bool {
    kv.get(key)
        .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
}

/// (v3.39.0 #3) Lee una preferencia de unidad por categoría. Si la clave
/// no existe o trae un valor inválido, MIGRA del legado
/// `pref_unit_system` ("metric" → `metric_default`, si no
/// `imperial_default`), de modo que los usuarios de v3.28–v3.38
/// conservan su elección al pasar al modelo por categoría.
fn unit_pref(
    kv: &HashMap<String, String>,
    key: &str,
    allowed: &[&str],
    metric_default: &str,
    imperial_default: &str,
) -> String {
    if let Some(v) = kv.get(key) {
        if allowed.contains(&v.as_str()) {
            return v.clone();
        }
    }
    let legacy_metric = kv
        .get("pref_unit_system")
        .map(|s| s == "metric")
        .unwrap_or(false);
    if legacy_metric {
        metric_default.to_string()
    } else {
        imperial_default.to_string()
    }
}

async fn read_settings_map(pool: &SqlitePool) -> anyhow::Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn write_setting(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
