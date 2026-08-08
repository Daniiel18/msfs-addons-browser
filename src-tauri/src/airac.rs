//! (v6.2.8) Avisos de actualización del **AIRAC** (datos de navegación).
//!
//! A diferencia de GSX, el AIRAC NO necesita scrapear: los ciclos siguen un
//! calendario FIJO de 28 días anclado a una fecha de referencia conocida
//! (AIRAC 2301 efectivo el 2023-01-26, validado contra 2501 = 2025-01-23). Así
//! calculamos el ciclo vigente para HOY de forma determinística y lo comparamos
//! con el que el usuario tiene instalado (persistido en `settings` bajo
//! `airac_installed_cycle`). El frontend lo muestra en Notificaciones + Dashboard
//! y, al actualizar, abre Navigraph Hub y guarda el ciclo nuevo.

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::AppState;

const AIRAC_INSTALLED_KEY: &str = "airac_installed_cycle";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiracUpdateInfo {
    pub installed_cycle: String,
    pub latest_cycle: String,
    pub effective_date: String, // ISO YYYY-MM-DD del ciclo vigente
    pub has_update: bool,
}

/// Ciclo AIRAC ("YYNN") y su fecha efectiva para una fecha dada.
fn airac_for(date: NaiveDate) -> (String, NaiveDate) {
    // Ancla: AIRAC 2301 efectivo 2023-01-26.
    let anchor = NaiveDate::from_ymd_opt(2023, 1, 26).unwrap();
    let days = (date - anchor).num_days();
    let periods = days.div_euclid(28);
    let eff = anchor + Duration::days(periods * 28);
    let year = eff.year();
    // Ordinal dentro del año = nº de fechas efectivas de ese año hasta `eff`.
    let mut first = eff;
    while (first - Duration::days(28)).year() == year {
        first -= Duration::days(28);
    }
    let ordinal = ((eff - first).num_days() / 28) + 1;
    (format!("{:02}{:02}", year % 100, ordinal), eff)
}

async fn read_installed(pool: &SqlitePool, latest: &str, today: NaiveDate) -> String {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(AIRAC_INSTALLED_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row.map(|r| r.0).filter(|s| !s.trim().is_empty()) {
        Some(v) => v,
        // Semilla por defecto: el ciclo ANTERIOR al vigente, para que la primera
        // corrida muestre la notificación (también sirve de prueba). Al
        // actualizar se reemplaza por el real.
        None => {
            let prev = airac_for(today - Duration::days(28)).0;
            if prev == latest {
                latest.to_string()
            } else {
                prev
            }
        }
    }
}

#[tauri::command]
pub async fn airac_check_update(
    state: tauri::State<'_, AppState>,
) -> Result<AiracUpdateInfo, String> {
    // (v7.2.4) Fecha LOCAL, no UTC: los ciclos AIRAC son efectivos a las 0000Z,
    // pero el usuario compara contra SU calendario. Con UTC, un usuario en
    // UTC-4 (Rep. Dominicana) veía el ciclo nuevo la noche ANTERIOR a la fecha
    // efectiva impresa (a las 20:00 locales ya era el día siguiente en UTC).
    // Usando la fecha local, el aviso aparece el mismo día que la fecha mostrada.
    let today = Local::now().date_naive();
    let (latest_cycle, eff) = airac_for(today);
    let installed = read_installed(&state.db, &latest_cycle, today).await;
    // "Más nuevo" = distinto y con ciclo numéricamente mayor (los YYNN crecen
    // monótonamente con el tiempo, así que comparar como número va bien).
    let has_update = installed != latest_cycle
        && installed.parse::<u32>().unwrap_or(0) < latest_cycle.parse::<u32>().unwrap_or(0);
    Ok(AiracUpdateInfo {
        installed_cycle: installed,
        latest_cycle,
        effective_date: eff.format("%Y-%m-%d").to_string(),
        has_update,
    })
}

/// Marca un ciclo como instalado (tras actualizar en Navigraph Hub).
#[tauri::command]
pub async fn airac_set_installed_cycle(
    cycle: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(AIRAC_INSTALLED_KEY)
    .bind(cycle.trim())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Resuelve la ruta del actualizador (Navigraph Hub) POR USUARIO. Devuelve el
/// exe o el acceso directo si existe; `None` si no se encuentra (el frontend
/// abre entonces la web). No se hardcodea la ruta porque vive en el perfil del
/// usuario (otro PC = otro usuario).
#[tauri::command]
pub fn airac_updater_path() -> Option<String> {
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push(
                std::path::Path::new(&local)
                    .join("Programs")
                    .join("navigraph-hub")
                    .join("Navigraph Hub.exe"),
            );
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(
                std::path::Path::new(&appdata)
                    .join(r"Microsoft\Windows\Start Menu\Programs\Navigraph Hub\Navigraph Hub.lnk"),
            );
        }
        v
    };
    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
}
