//! Comandos de configuración persistente.
//!
//! Todo se guarda en la tabla `settings` (k/v). La tabla ya existe
//! desde la migración 001 y desde la 009 (`simbrief_pilot_id`);
//! aquí añadimos las claves nuevas:
//!
//!   · `pref_show_simbrief_lines`     — bool ("1"/"0")
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
    pub show_simbrief_lines: bool,
    pub show_simconnect_lines: bool,
    pub check_updates_on_start: bool,
    pub minimize_to_tray: bool,
    pub onboarding_completed: bool,
    pub default_view: String,
    /// Tema visual: "dark" (default) o "light". Controla la clase
    /// `dark` en `<html>` (Tailwind class-based) y el basemap del
    /// mapa de FlightBook.
    pub theme: String,
    pub autostart_enabled: bool,
    pub simbrief_pilot_id: Option<String>,
    /// Carpeta Community detectada — la mostramos en read-only para
    /// que el usuario sepa qué disco/path está leyendo la app.
    pub community_path: Option<String>,
    /// Path absoluto a la carpeta de logs (`<app_data>/logs/`).
    pub logs_path: Option<String>,
    /// Path al directorio de datos de la app (`<app_data>/`).
    pub app_data_path: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            show_simbrief_lines: true,
            show_simconnect_lines: true,
            check_updates_on_start: true,
            minimize_to_tray: false,
            onboarding_completed: false,
            default_view: "dashboard".to_string(),
            theme: "dark".to_string(),
            autostart_enabled: false,
            simbrief_pilot_id: None,
            community_path: None,
            logs_path: None,
            app_data_path: None,
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
    let app_data_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let logs_path = app_data_path.as_ref().map(|p| {
        std::path::Path::new(p)
            .join("logs")
            .to_string_lossy()
            .into_owned()
    });
    Ok(AppSettings {
        show_simbrief_lines: as_bool(&kv, "pref_show_simbrief_lines", true),
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
        autostart_enabled,
        simbrief_pilot_id: kv
            .get("simbrief_pilot_id")
            .filter(|s| !s.trim().is_empty())
            .cloned(),
        community_path,
        logs_path,
        app_data_path,
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
        "pref_show_simbrief_lines"
            | "pref_show_simconnect_lines"
            | "pref_check_updates_on_start"
            | "pref_minimize_to_tray"
            | "pref_onboarding_completed"
            | "pref_default_view"
            // `pref_theme`: "dark" | "light" — controla CSS class
            // en <html> (Tailwind class-based dark mode).
            | "pref_theme"
    )
}

fn as_bool(kv: &HashMap<String, String>, key: &str, default: bool) -> bool {
    kv.get(key)
        .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
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
