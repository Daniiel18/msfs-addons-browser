//! (v6.2.4) Chequeo de actualizaciones de **GSX / couatl** (FSDreamTeam).
//!
//! FSDT no tiene API: publican las notas de cada release en
//! `couatl_liveupdate_notes.html`, con la versión MÁS NUEVA arriba en el formato
//! `Version X.Y.Z – fecha`. Parseamos la primera coincidencia = última versión y
//! la comparamos con la versión que el usuario tiene instalada (persistida en
//! `settings` bajo `gsx_installed_version`). El frontend consulta esto cada hora
//! y lo muestra en Notificaciones + Dashboard. Al actualizar (clic en la
//! notificación → abre el "FSDT Installer"), el frontend llama a
//! `gsx_set_installed_version` con la última, de modo que no vuelve a avisar
//! hasta la siguiente.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::AppState;

const GSX_NOTES_URL: &str = "https://www.fsdreamteam.com/couatl_liveupdate_notes.html";
const GSX_INSTALLED_KEY: &str = "gsx_installed_version";
/// Semilla por defecto DELIBERADAMENTE por debajo de la última publicada para
/// que la primera corrida muestre la notificación (sirve también de prueba). En
/// cuanto el usuario actualice, se reemplaza por la versión real.
const GSX_DEFAULT_INSTALLED: &str = "4.0.4";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GsxUpdateInfo {
    pub installed_version: String,
    pub latest_version: Option<String>,
    pub latest_date: Option<String>,
    pub has_update: bool,
    pub notes_url: String,
}

/// Primera (= más nueva) "Version X.Y.Z – fecha" de la página.
fn parse_latest(html: &str) -> Option<(String, Option<String>)> {
    let re = regex::Regex::new(
        r"(?i)Version\s+(\d+\.\d+\.\d+)\s*[\x{2013}\x{2014}\-]?\s*([^<\r\n]{0,40})",
    )
    .ok()?;
    let c = re.captures(html)?;
    let ver = c.get(1)?.as_str().to_string();
    let date = c
        .get(2)
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty());
    Some((ver, date))
}

/// `a > b` por semver; si alguno no parsea, comparación lexicográfica.
fn version_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(va), Ok(vb)) => va > vb,
        _ => a > b,
    }
}

async fn read_installed(pool: &SqlitePool) -> String {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(GSX_INSTALLED_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    row.map(|r| r.0)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| GSX_DEFAULT_INSTALLED.to_string())
}

#[tauri::command]
pub async fn gsx_check_update(
    state: tauri::State<'_, AppState>,
) -> Result<GsxUpdateInfo, String> {
    let installed = read_installed(&state.db).await;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (SimFleet GSX update check)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let html = client
        .get(GSX_NOTES_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let (latest, date) = match parse_latest(&html) {
        Some((v, d)) => (Some(v), d),
        None => (None, None),
    };
    let has_update = latest
        .as_deref()
        .map(|l| version_gt(l, &installed))
        .unwrap_or(false);
    Ok(GsxUpdateInfo {
        installed_version: installed,
        latest_version: latest,
        latest_date: date,
        has_update,
        notes_url: GSX_NOTES_URL.to_string(),
    })
}

/// Marca una versión como instalada (la llama el frontend cuando el usuario
/// actualiza GSX desde la notificación). Así no vuelve a avisar por esa versión.
#[tauri::command]
pub async fn gsx_set_installed_version(
    version: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(GSX_INSTALLED_KEY)
    .bind(version.trim())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
