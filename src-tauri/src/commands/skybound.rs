//! (v6.2.38) Comandos de Skybound: guardar credenciales + estado.
//! Las credenciales viven en `settings` (skybound_user/skybound_pass) y
//! se reflejan en el `skybound_auth` compartido con la fuente.

use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::sources::skybound::SkyboundAuth;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkyboundStatus {
    pub has_credentials: bool,
    pub logged_in: bool,
}

async fn upsert(pool: &SqlitePool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        r#"INSERT INTO settings (key, value, updated_at)
           VALUES (?1, ?2, datetime('now'))
           ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn skybound_set_credentials(
    username: String,
    password: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    upsert(&state.db, "skybound_user", &username).await?;
    upsert(&state.db, "skybound_pass", &password).await?;
    let mut a = state.skybound_auth.write().await;
    a.username = (!username.is_empty()).then_some(username);
    a.password = (!password.is_empty()).then_some(password);
    a.session_cookie = None; // fuerza re-login con las nuevas credenciales
    Ok(())
}

#[tauri::command]
pub async fn skybound_status(
    state: tauri::State<'_, AppState>,
) -> Result<SkyboundStatus, String> {
    let a = state.skybound_auth.read().await;
    let has = a.username.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        && a.password.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let logged_in = a
        .session_cookie
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    Ok(SkyboundStatus {
        has_credentials: has,
        logged_in,
    })
}

/// Carga las credenciales guardadas al arrancar en el `auth` compartido.
pub async fn bootstrap(pool: &SqlitePool, auth: &Arc<RwLock<SkyboundAuth>>) {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key IN ('skybound_user','skybound_pass')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut user = None;
    let mut pass = None;
    for (k, v) in rows {
        match k.as_str() {
            "skybound_user" => user = Some(v),
            "skybound_pass" => pass = Some(v),
            _ => {}
        }
    }
    let mut a = auth.write().await;
    a.username = user.filter(|s| !s.is_empty());
    a.password = pass.filter(|s| !s.is_empty());
}
