//! Sistema de puntuación de vuelo estilo Virtual Airline.
//!
//! Cada vuelo (SimConnect o VAS-ACARS importado) puede evaluarse contra
//! un **rubric** de reglas agrupadas por phase. Cada regla:
//!   · Tiene un `points_max` fijo (el peso de la regla).
//!   · Lee la materia prima: la fila de `flight_log` + las muestras de
//!     `flight_log_track` filtradas por la phase relevante + las
//!     transiciones de `flight_log_phase`.
//!   · Produce un `ScoreItem` con `points_earned` (0..=points_max),
//!     `passed` (bool), `severity` (info/warn/fail) y `evidence`
//!     (JSON con los números que dispararon el verdict).
//!
//! El total se persiste como columnas `score_total/score_max/score_grade`
//! en `flight_log` (lectura barata para badges en la sidebar), y los
//! breakdowns rule-by-rule van a `flight_log_score_item`.
//!
//! Auto-trigger: `finish_flight` llama a `score_flight(flight_id)` para
//! todos los vuelos que terminan, y el resultado se sube al cloud
//! automáticamente (decisión Q15 del kickoff Phase H).

use serde::Serialize;
use sqlx::SqlitePool;

pub mod rubric;

pub use rubric::{Phase, Rule, RULES};

/// Resultado del scoring para un vuelo — listo para devolverse al
/// frontend como respuesta del comando `score_flight`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub flight_id: i64,
    pub total: i64,
    pub max: i64,
    pub percentage: f32,
    pub grade: String,
    pub items: Vec<ScoreItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreItem {
    pub phase: String,
    pub rule_id: String,
    pub label: String,
    pub points_earned: i64,
    pub points_max: i64,
    pub passed: bool,
    pub severity: String,
    pub evidence: serde_json::Value,
}

/// Contexto del vuelo cargado de DB — todo lo que el rubric necesita
/// en una sola estructura inmutable.
#[derive(Debug, Clone)]
pub struct FlightContext {
    pub flight_id: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub origin_icao: Option<String>,
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub destination_icao: Option<String>,
    pub destination_lat: Option<f64>,
    pub destination_lon: Option<f64>,
    pub distance_nm: Option<f64>,
    pub flight_time_s: Option<i64>,
    pub max_altitude_ft: Option<i64>,
    pub landing_fpm: Option<i64>,
    pub max_ground_speed_kt: Option<i64>,
    pub max_true_airspeed_kt: Option<i64>,
    pub flight_number: Option<String>,
    pub callsign: Option<String>,
    pub airline_icao: Option<String>,
    pub status: String,
    pub arrival_gate: Option<String>,
    pub track: Vec<TrackSample>,
    pub phases: Vec<PhaseRange>,
}

#[derive(Debug, Clone)]
pub struct TrackSample {
    pub ts: String, // ISO-8601 UTC
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i64>,
    pub gs_kt: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PhaseRange {
    pub phase: String,
    pub entered_at: String,
    pub exited_at: Option<String>,
}

impl FlightContext {
    /// Devuelve las muestras del track que cayeron dentro de la
    /// ventana temporal de la phase. Si la phase no está registrada,
    /// devuelve un slice vacío — la regla decide cómo manejarlo (ej.
    /// las reglas de `general` no necesitan phase).
    pub fn samples_in_phase(&self, phase: &str) -> Vec<&TrackSample> {
        let Some(range) = self.phases.iter().find(|p| p.phase == phase) else {
            return Vec::new();
        };
        let exit = range.exited_at.as_deref().unwrap_or("9999-12-31T23:59:59Z");
        self.track
            .iter()
            .filter(|s| s.ts.as_str() >= range.entered_at.as_str() && s.ts.as_str() <= exit)
            .collect()
    }

    /// Devuelve las muestras durante CUALQUIERA de las phases pasadas
    /// (útil para reglas que abarcan multiple — ej. "no overspeed
    /// below 10k" cubre climb y descent).
    pub fn samples_in_phases(&self, phases: &[&str]) -> Vec<&TrackSample> {
        let mut out: Vec<&TrackSample> = Vec::new();
        for p in phases {
            out.extend(self.samples_in_phase(p));
        }
        out
    }
}

/// Row helper para `sqlx::query_as` — sqlx no implementa FromRow para
/// tuplas de >16 elementos, así que usamos un struct dedicado.
#[derive(sqlx::FromRow)]
struct FlightRow {
    started_at: String,
    ended_at: Option<String>,
    origin_icao: Option<String>,
    origin_lat: f64,
    origin_lon: f64,
    destination_icao: Option<String>,
    destination_lat: Option<f64>,
    destination_lon: Option<f64>,
    distance_nm: Option<f64>,
    flight_time_s: Option<i64>,
    max_altitude_ft: Option<i64>,
    landing_fpm: Option<i64>,
    max_ground_speed_kt: Option<i64>,
    max_true_airspeed_kt: Option<i64>,
    flight_number: Option<String>,
    callsign: Option<String>,
    airline_icao: Option<String>,
    status: String,
    arrival_gate: Option<String>,
}

/// Cargar el FlightContext desde DB para un vuelo dado.
pub async fn load_context(pool: &SqlitePool, flight_id: i64) -> anyhow::Result<FlightContext> {
    let row: FlightRow = sqlx::query_as(
        r#"
        SELECT started_at, ended_at, origin_icao, origin_lat, origin_lon,
               destination_icao, destination_lat, destination_lon,
               distance_nm, flight_time_s,
               max_altitude_ft, landing_fpm,
               max_ground_speed_kt, max_true_airspeed_kt,
               flight_number, callsign, airline_icao, status,
               arrival_gate
        FROM flight_log
        WHERE id = ?1
        "#,
    )
    .bind(flight_id)
    .fetch_one(pool)
    .await?;

    let track: Vec<(String, f64, f64, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT ts, lat, lon, alt_ft, gs_kt
        FROM flight_log_track
        WHERE flight_id = ?1
        ORDER BY ts
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;

    let phases: Vec<(String, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT phase, entered_at, exited_at
        FROM flight_log_phase
        WHERE flight_id = ?1
        ORDER BY entered_at
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;

    Ok(FlightContext {
        flight_id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        origin_icao: row.origin_icao,
        origin_lat: row.origin_lat,
        origin_lon: row.origin_lon,
        destination_icao: row.destination_icao,
        destination_lat: row.destination_lat,
        destination_lon: row.destination_lon,
        distance_nm: row.distance_nm,
        flight_time_s: row.flight_time_s,
        max_altitude_ft: row.max_altitude_ft,
        landing_fpm: row.landing_fpm,
        max_ground_speed_kt: row.max_ground_speed_kt,
        max_true_airspeed_kt: row.max_true_airspeed_kt,
        flight_number: row.flight_number,
        callsign: row.callsign,
        airline_icao: row.airline_icao,
        status: row.status,
        arrival_gate: row.arrival_gate,
        track: track
            .into_iter()
            .map(|(ts, lat, lon, alt, gs)| TrackSample {
                ts,
                lat,
                lon,
                alt_ft: alt,
                gs_kt: gs,
            })
            .collect(),
        phases: phases
            .into_iter()
            .map(|(phase, entered_at, exited_at)| PhaseRange {
                phase,
                entered_at,
                exited_at,
            })
            .collect(),
    })
}

/// Evalúa **todas** las reglas del rubric sobre el FlightContext y
/// produce un `ScoreReport`. Persiste los items en
/// `flight_log_score_item` (upsert por (flight_id, phase, rule_id))
/// y actualiza las columnas resumen `score_total/score_max/score_grade`
/// en `flight_log`.
pub async fn score_flight(pool: &SqlitePool, flight_id: i64) -> anyhow::Result<ScoreReport> {
    let ctx = load_context(pool, flight_id).await?;

    let mut items: Vec<ScoreItem> = Vec::new();
    let mut total: i64 = 0;
    let mut max: i64 = 0;

    for rule in RULES.iter() {
        let item = (rule.evaluator)(&ctx, rule);
        total += item.points_earned;
        max += rule.points_max;
        items.push(item);
    }

    let percentage = if max > 0 {
        (total as f32 / max as f32) * 100.0
    } else {
        0.0
    };
    let grade = grade_for_percentage(percentage);

    // Persistencia idempotente: UPSERT cada item, luego actualizar
    // las columnas resumen.
    let mut tx = pool.begin().await?;
    for item in &items {
        sqlx::query(
            r#"
            INSERT INTO flight_log_score_item
                (flight_id, phase, rule_id, label, points_earned, points_max,
                 passed, severity, evidence, ts)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(flight_id, phase, rule_id) DO UPDATE SET
                label         = excluded.label,
                points_earned = excluded.points_earned,
                points_max    = excluded.points_max,
                passed        = excluded.passed,
                severity      = excluded.severity,
                evidence      = excluded.evidence,
                ts            = excluded.ts
            "#,
        )
        .bind(flight_id)
        .bind(&item.phase)
        .bind(&item.rule_id)
        .bind(&item.label)
        .bind(item.points_earned)
        .bind(item.points_max)
        .bind(if item.passed { 1 } else { 0 })
        .bind(&item.severity)
        .bind(item.evidence.to_string())
        .bind(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        r#"
        UPDATE flight_log
        SET score_total = ?1,
            score_max   = ?2,
            score_grade = ?3
        WHERE id = ?4
        "#,
    )
    .bind(total)
    .bind(max)
    .bind(&grade)
    .bind(flight_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(ScoreReport {
        flight_id,
        total,
        max,
        percentage,
        grade,
        items,
    })
}

/// Carga el ScoreReport **sin recomputar** (si ya hay items
/// persistidos). Si la tabla está vacía para el flight, devuelve `None`
/// — el caller decide si dispara `score_flight` o muestra "no
/// puntuado todavía".
pub async fn load_persisted_report(
    pool: &SqlitePool,
    flight_id: i64,
) -> anyhow::Result<Option<ScoreReport>> {
    let summary: Option<(Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT score_total, score_max, score_grade FROM flight_log WHERE id = ?1",
    )
    .bind(flight_id)
    .fetch_optional(pool)
    .await?;
    let Some((Some(total), Some(max), grade)) = summary else {
        return Ok(None);
    };
    let items_raw: Vec<(
        String,
        String,
        String,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        r#"
        SELECT phase, rule_id, label, points_earned, points_max,
               passed, severity, evidence
        FROM flight_log_score_item
        WHERE flight_id = ?1
        ORDER BY phase, rule_id
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;
    let items: Vec<ScoreItem> = items_raw
        .into_iter()
        .map(|(phase, rule_id, label, earned, max_p, passed, sev, ev)| ScoreItem {
            phase,
            rule_id,
            label,
            points_earned: earned,
            points_max: max_p,
            passed: passed != 0,
            severity: sev.unwrap_or_else(|| "info".to_string()),
            evidence: ev
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null),
        })
        .collect();
    let percentage = if max > 0 {
        (total as f32 / max as f32) * 100.0
    } else {
        0.0
    };
    Ok(Some(ScoreReport {
        flight_id,
        total,
        max,
        percentage,
        grade: grade.unwrap_or_else(|| grade_for_percentage(percentage)),
        items,
    }))
}

/// (v3.6.0 Phase H — H13) Hook llamado por el watcher post-`finish_flight`.
/// Corre scoring + auto-upload al cloud, emitiendo eventos para que
/// la UI muestre el progreso/resultado:
///
///   · `score:done`           — Payload `ScoreReport`. Score listo.
///   · `score:upload:started` — Payload `{ flightId }`. Sube empezó.
///   · `score:upload:success` — Payload `UploadReport`. Sube OK.
///   · `score:upload:error`   — Payload `{ flightId, error }`. Sube falló.
///
/// La función NO devuelve error — todos los fallos se emiten como
/// eventos. El watcher sigue su loop normal pase lo que pase.
pub async fn finalize_after_finish(
    pool: &sqlx::SqlitePool,
    app: &tauri::AppHandle,
    flight_id: i64,
) {
    use tauri::Emitter;

    let report = match score_flight(pool, flight_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "scoring",
                "finalize_after_finish: score_flight({}) failed: {:#}",
                flight_id, e
            );
            let _ = app.emit(
                "score:error",
                &serde_json::json!({ "flightId": flight_id, "error": e.to_string() }),
            );
            return;
        }
    };

    tracing::info!(
        target: "scoring",
        "flight {} scored: {}/{} ({:.0}%) grade={}",
        flight_id, report.total, report.max, report.percentage, report.grade
    );
    let _ = app.emit("score:done", &report);

    // Auto-upload to cloud (non-blocking — spawn detached). Toma el
    // reqwest::Client del AppState compartido (Manager::state).
    let pool_c = pool.clone();
    let app_c = app.clone();
    tokio::spawn(async move {
        use tauri::Manager;
        let _ = app_c.emit(
            "score:upload:started",
            &serde_json::json!({ "flightId": flight_id }),
        );
        let state = app_c.state::<crate::AppState>();
        match crate::cloud_sync::upload_all(&pool_c, &state.http).await {
            Ok(rep) => {
                tracing::info!(
                    target: "scoring",
                    "auto-upload after scoring OK: {:?}", rep
                );
                let _ = app_c.emit("score:upload:success", &rep);
            }
            Err(e) => {
                tracing::warn!(
                    target: "scoring",
                    "auto-upload after scoring failed: {:#}", e
                );
                let _ = app_c.emit(
                    "score:upload:error",
                    &serde_json::json!({
                        "flightId": flight_id,
                        "error": e.to_string()
                    }),
                );
            }
        }
    });
}

fn grade_for_percentage(p: f32) -> String {
    match p as i32 {
        95..=200 => "A",
        85..=94 => "B",
        70..=84 => "C",
        50..=69 => "D",
        _ => "F",
    }
    .to_string()
}
