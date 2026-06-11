//! Comandos Tauri para el sistema de scoring (Epic E Phase H).
//!
//! - `score_flight` — recomputa y persiste el score del vuelo.
//! - `score_get_report` — devuelve el score persistido SIN recomputar
//!   (rápido para abrir el ChecklistWidget). Si nunca se computó,
//!   ejecuta `score_flight`.
//! - `list_airlines` — lista de aerolíneas únicas con vuelos completados,
//!   para alimentar los chips del AirlineTagFilter.
//! - `airline_kpis` — agregados (pax, cargo, fuel, distance, etc.) para
//!   una aerolínea seleccionada.

use serde::Serialize;

use crate::scoring::{self, ScoreReport};
use crate::AppState;

#[tauri::command]
pub async fn score_flight(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<ScoreReport, String> {
    let report = scoring::score_flight(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())?;
    // (v4.19.0) Re-evaluar desde la UI también (re)genera el log por
    // vuelo `ORIG-DEST-CALLSIGN.log` — así se pueden producir reportes
    // de vuelos VIEJOS sin volver a volarlos. Best-effort.
    if let Ok(data_dir) = crate::resolve_portable_data_dir() {
        if let Err(e) =
            crate::flight_report::write_flight_report(&state.db, flight_id, &report, &data_dir)
                .await
        {
            tracing::debug!(target: "scoring", "flight report({flight_id}) falló: {e:#}");
        }
    }
    Ok(report)
}

#[tauri::command]
pub async fn score_get_report(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<ScoreReport, String> {
    match scoring::load_persisted_report(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(rep) => Ok(rep),
        None => scoring::score_flight(&state.db, flight_id)
            .await
            .map_err(|e| e.to_string()),
    }
}

/// Una aerolínea identificada en el historial de vuelos del usuario.
/// La UI muestra un chip por cada entrada de este Vec.
///
/// Agrupación: PRIMERO por `airline_icao` (más robusto, 3-letter code).
/// Para vuelos donde `airline_icao` es NULL pero hay `aircraft_airline`,
/// agregamos un chip por `aircraft_airline` (con `icao=None`). El
/// usuario decidió (Q7=a con fallback b) que es una mezcla aceptable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlineTag {
    /// ICAO code (3 letras) — preferred. Si vino vía fallback de
    /// aircraft_airline, queda `None`.
    pub icao: Option<String>,
    /// Nombre legible — `aircraft_airline` para vuelos VAS, o
    /// extraído del livery cuando no hay otra cosa.
    pub name: String,
    /// Cantidad de vuelos completados de esta aerolínea.
    pub flight_count: i64,
}

#[tauri::command]
pub async fn list_airlines(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AirlineTag>, String> {
    let pool = &state.db;
    // Primero: agrupación por airline_icao (canónica).
    let by_icao: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        r#"
        SELECT airline_icao,
               MAX(aircraft_airline) AS sample_name,
               COUNT(*) AS cnt
        FROM flight_log
        WHERE status = 'completed'
          AND airline_icao IS NOT NULL
        GROUP BY airline_icao
        ORDER BY cnt DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let icao_set: std::collections::HashSet<String> =
        by_icao.iter().map(|(i, _, _)| i.clone()).collect();

    // Segundo: fallback por aircraft_airline cuando airline_icao es NULL.
    let by_name: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT aircraft_airline,
               COUNT(*) AS cnt
        FROM flight_log
        WHERE status = 'completed'
          AND airline_icao IS NULL
          AND aircraft_airline IS NOT NULL
          AND TRIM(aircraft_airline) <> ''
        GROUP BY aircraft_airline
        ORDER BY cnt DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut tags: Vec<AirlineTag> = Vec::with_capacity(by_icao.len() + by_name.len());
    for (icao, name, cnt) in by_icao {
        tags.push(AirlineTag {
            icao: Some(icao.clone()),
            name: name.unwrap_or(icao),
            flight_count: cnt,
        });
    }
    for (name, cnt) in by_name {
        // Si por casualidad otra fila ya tiene esta name como ICAO
        // (improbable pero defensivo), skip.
        if icao_set.contains(&name) {
            continue;
        }
        tags.push(AirlineTag {
            icao: None,
            name,
            flight_count: cnt,
        });
    }
    Ok(tags)
}

/// Agregados consolidados para una aerolínea (Epic D — KPIs dinámicos).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlineKpis {
    pub airline_icao: Option<String>,
    pub airline_name: Option<String>,
    pub flight_count: i64,
    pub total_passengers: i64,
    pub total_cargo_kg: i64,
    pub total_fuel_kg: i64,
    pub total_distance_nm: f64,
    pub total_block_seconds: i64,
    /// Promedio de landing FPM entre vuelos que lo capturaron.
    pub avg_landing_fpm: Option<i64>,
}

/// Filtro: si `airline_icao` está presente, agrupa por esa key.
/// Si en cambio el frontend manda `airline_name` (el caso fallback
/// donde el ICAO es null), agrupa por aircraft_airline.
#[tauri::command]
pub async fn airline_kpis(
    airline_icao: Option<String>,
    airline_name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AirlineKpis, String> {
    let pool = &state.db;
    let (where_clause, bind_value) = match (&airline_icao, &airline_name) {
        (Some(icao), _) => ("airline_icao = ?1", icao.clone()),
        (None, Some(name)) => (
            "airline_icao IS NULL AND aircraft_airline = ?1",
            name.clone(),
        ),
        _ => return Err("airline_icao or airline_name required".into()),
    };
    let sql = format!(
        r#"
        SELECT
            COUNT(*) AS cnt,
            COALESCE(SUM(passengers), 0) AS pax,
            COALESCE(SUM(cargo_kg), 0) AS cargo,
            COALESCE(SUM(fuel_used_kg), 0) AS fuel,
            COALESCE(SUM(distance_nm), 0.0) AS dist,
            COALESCE(SUM(flight_time_s), 0) AS block_s
        FROM flight_log
        WHERE status = 'completed' AND {}
        "#,
        where_clause
    );
    let row: (i64, i64, i64, i64, f64, i64) = sqlx::query_as(&sql)
        .bind(&bind_value)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Avg landing FPM — sólo entre vuelos que lo capturaron (negativo).
    let avg_fpm_sql = format!(
        r#"
        SELECT CAST(AVG(landing_fpm) AS INTEGER)
        FROM flight_log
        WHERE status = 'completed'
          AND landing_fpm IS NOT NULL
          AND landing_fpm < 0
          AND {}
        "#,
        where_clause
    );
    let avg_fpm: Option<i64> = sqlx::query_scalar(&avg_fpm_sql)
        .bind(&bind_value)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();

    Ok(AirlineKpis {
        airline_icao,
        airline_name,
        flight_count: row.0,
        total_passengers: row.1,
        total_cargo_kg: row.2,
        total_fuel_kg: row.3,
        total_distance_nm: row.4,
        total_block_seconds: row.5,
        avg_landing_fpm: avg_fpm,
    })
}
