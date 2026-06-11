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

pub use rubric::{phase_order, rule_applies_to, rule_index, Category, Phase, Rule, RULES};

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
    /// (v4.1.0) Estado del ítem en el modelo de checklist puro:
    ///   · "done"   — cumplido (✓ verde)
    ///   · "missed" — no cumplido / parcial (✗ rojo)
    ///   · "na"     — no aplica / sin datos (— gris). NO penaliza el %.
    /// Derivado de `passed` + `severity` (severity == "skipped" → na).
    pub status: String,
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
    /// (v4.1.0) Título/tipo de la aeronave — usado por
    /// `aircraft_category()` para decidir qué ítems del checklist aplican.
    pub aircraft_title: Option<String>,
    pub aircraft_atc_type: Option<String>,
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
    // (v3.7.0 — Phase O) Simvars VAS-aligned. Vienen NULL para
    // vuelos importados de VAS-ACARS; populados sólo para vuelos
    // capturados por el watcher de SimConnect.
    pub ias_kt: Option<i64>,
    pub vs_fpm: Option<i64>,
    pub pitch_deg: Option<f64>,
    pub bank_deg: Option<f64>,
    pub g_force: Option<f64>,
    pub flaps_pct: Option<i64>,
    pub gear_down: Option<bool>,
    pub spoilers_pct: Option<i64>,
    pub light_nav: Option<bool>,
    pub light_beacon: Option<bool>,
    pub light_taxi: Option<bool>,
    pub light_landing: Option<bool>,
    pub light_strobe: Option<bool>,
    pub parking_brake: Option<bool>,
    pub transponder_code: Option<i64>,
    /// (v4.0.0 P7.8c) Fase de scoring asignada a este sample por el
    /// watcher (o el VAS importer). El evaluador filtra por ESTE campo
    /// en vez de la ventana temporal de flight_log_phase. Es lo que
    /// hace que las fases cortas (initial_climb, final_approach) no se
    /// pierdan.
    pub scoring_phase: Option<String>,
    /// (v3.21.0) Máximo N1 entre los 4 motores en este sample (%). Lo
    /// usan las reglas "engines off" (pre-departure / pushback): N1 < 5%
    /// ≈ motores apagados. None para vuelos sin datos de motor.
    pub eng_n1_max: Option<f64>,
    /// (v3.21.0) STALL WARNING del sim (bool). None si no se capturó.
    pub stall_warning: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PhaseRange {
    pub phase: String,
    pub entered_at: String,
    pub exited_at: Option<String>,
}

impl FlightContext {
    /// (v3.21.0) True si AL MENOS un sample tiene `scoring_phase`
    /// poblado — i.e. es un vuelo capturado por el watcher (o un VAS
    /// import que ya derivó fases por-sample). Los helpers del rubric lo
    /// usan para decidir entre la vía canónica (filtrar por scoring_phase
    /// con nombres `initial_climb` / `final_approach` / …) y la vía
    /// legacy (labels viejos del badge + banda AGL) para vuelos antiguos.
    pub fn has_scoring_phase(&self) -> bool {
        self.track.iter().any(|s| s.scoring_phase.is_some())
    }

    /// Devuelve las muestras del track que cayeron dentro de la
    /// ventana temporal de la phase. Si la phase no está registrada,
    /// devuelve un slice vacío — la regla decide cómo manejarlo (ej.
    /// las reglas de `general` no necesitan phase).
    pub fn samples_in_phase(&self, phase: &str) -> Vec<&TrackSample> {
        // (v4.0.0 P7.8c) Estrategia primaria: filtrar por la columna
        // `scoring_phase` de CADA sample. Esto es lo que hace el ACARS
        // y elimina el "No data to evaluate" de fases cortas. Si AL
        // MENOS un sample tiene scoring_phase poblado, usamos esta vía.
        let any_tagged = self.track.iter().any(|s| s.scoring_phase.is_some());
        if any_tagged {
            return self
                .track
                .iter()
                .filter(|s| s.scoring_phase.as_deref() == Some(phase))
                .collect();
        }

        // Fallback legacy (vuelos pre-P7.8c sin la columna poblada):
        // ventana temporal de flight_log_phase. Frágil para fases
        // cortas pero es lo único que tenemos para vuelos viejos.
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

/// (v3.7.0 Phase O) Row helper para el track con todos los simvars
/// VAS-aligned. Mismas razones que `FlightRow` para usar struct en
/// vez de tupla (sqlx FromRow tope = 16 elementos).
#[derive(sqlx::FromRow)]
struct TrackRow {
    ts: String,
    lat: f64,
    lon: f64,
    alt_ft: Option<i64>,
    gs_kt: Option<i64>,
    ias_kt: Option<i64>,
    vs_fpm: Option<i64>,
    pitch_deg: Option<f64>,
    bank_deg: Option<f64>,
    g_force: Option<f64>,
    flaps_pct: Option<i64>,
    gear_down: Option<i64>,
    spoilers_pct: Option<i64>,
    light_nav: Option<i64>,
    light_beacon: Option<i64>,
    light_taxi: Option<i64>,
    light_landing: Option<i64>,
    light_strobe: Option<i64>,
    parking_brake: Option<i64>,
    transponder_code: Option<i64>,
    scoring_phase: Option<String>,
    eng_n1_1: Option<f64>,
    eng_n1_2: Option<f64>,
    eng_n1_3: Option<f64>,
    eng_n1_4: Option<f64>,
    stall_warning: Option<i64>,
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
    aircraft_title: Option<String>,
    aircraft_atc_type: Option<String>,
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
               arrival_gate, aircraft_title, aircraft_atc_type
        FROM flight_log
        WHERE id = ?1
        "#,
    )
    .bind(flight_id)
    .fetch_one(pool)
    .await?;

    // (v3.7.0 Phase O) Carga todos los campos extendidos via struct
    // dedicada — sqlx no implementa FromRow para tuplas >16 elementos.
    // Los booleans vienen como Option<i64> (SQLite tiene INTEGER 0/1)
    // y los normalizamos a bool al construir el TrackSample.
    let track: Vec<TrackRow> = sqlx::query_as(
        r#"
        SELECT ts, lat, lon, alt_ft, gs_kt,
               ias_kt, vs_fpm, pitch_deg, bank_deg, g_force,
               flaps_pct, gear_down, spoilers_pct,
               light_nav, light_beacon, light_taxi, light_landing, light_strobe,
               parking_brake, transponder_code, scoring_phase,
               eng_n1_1, eng_n1_2, eng_n1_3, eng_n1_4, stall_warning
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
        aircraft_title: row.aircraft_title,
        aircraft_atc_type: row.aircraft_atc_type,
        track: track
            .into_iter()
            .map(|r| {
                let b = |o: Option<i64>| o.map(|v| v != 0);
                TrackSample {
                    ts: r.ts,
                    lat: r.lat,
                    lon: r.lon,
                    alt_ft: r.alt_ft,
                    gs_kt: r.gs_kt,
                    ias_kt: r.ias_kt,
                    vs_fpm: r.vs_fpm,
                    pitch_deg: r.pitch_deg,
                    bank_deg: r.bank_deg,
                    g_force: r.g_force,
                    flaps_pct: r.flaps_pct,
                    gear_down: b(r.gear_down),
                    spoilers_pct: r.spoilers_pct,
                    light_nav: b(r.light_nav),
                    light_beacon: b(r.light_beacon),
                    light_taxi: b(r.light_taxi),
                    light_landing: b(r.light_landing),
                    light_strobe: b(r.light_strobe),
                    parking_brake: b(r.parking_brake),
                    transponder_code: r.transponder_code,
                    scoring_phase: r.scoring_phase,
                    eng_n1_max: [r.eng_n1_1, r.eng_n1_2, r.eng_n1_3, r.eng_n1_4]
                        .into_iter()
                        .flatten()
                        .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v)))),
                    stall_warning: b(r.stall_warning),
                }
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

/// (v4.1.0) Clasifica la aeronave en una categoría amplia para decidir
/// qué ítems del checklist aplican. Heurística por palabras clave del
/// título + tipo ATC. Fallback = `Airliner` (la app es airliner-first).
/// No pretende ser exhaustiva — solo separar el caso GA/turbohélice del
/// airliner para no penalizar ítems que no corresponden.
pub fn aircraft_category(title: Option<&str>, atc_type: Option<&str>) -> Category {
    let hay = format!(
        "{} {}",
        title.unwrap_or("").to_lowercase(),
        atc_type.unwrap_or("").to_lowercase()
    );
    const TURBOPROP: &[&str] = &[
        "tbm", "king air", "kingair", "c208", "caravan", "pc-12", "pc12",
        "pilatus", "atr ", "atr-", "dash 8", "dhc-6", "dhc-8", "twin otter",
        "saab 340", "1900", "do228", "dornier 228", "mu-2", "kodiak",
    ];
    const GA: &[&str] = &[
        "cessna 152", "c152", "cessna 172", "c172", "skyhawk", "cessna 182",
        "c182", "skylane", "cirrus", "sr22", "sr20", "diamond", "da40",
        "da42", "da62", "piper", "pa-28", "pa28", "cherokee", "archer",
        "arrow", "bonanza", "baron", "mooney", "robin", "extra ", "icon a5",
        "savage", "xcub", "super cub", "cub ", "vans", "rv-", "duchess",
        "warrior", "seminole", "g36", "g58", "172 ", "152 ",
    ];
    if TURBOPROP.iter().any(|k| hay.contains(k)) {
        return Category::Turboprop;
    }
    if GA.iter().any(|k| hay.contains(k)) {
        return Category::Ga;
    }
    Category::Airliner
}

/// Evalúa **todas** las reglas del rubric sobre el FlightContext y
/// produce un `ScoreReport`. Persiste los items en
/// `flight_log_score_item` (upsert por (flight_id, phase, rule_id))
/// y actualiza las columnas resumen `score_total/score_max/score_grade`
/// en `flight_log`.
pub async fn score_flight(pool: &SqlitePool, flight_id: i64) -> anyhow::Result<ScoreReport> {
    let ctx = load_context(pool, flight_id).await?;

    // (v4.1.0) Categoría del avión — decide qué ítems del checklist aplican.
    let category =
        aircraft_category(ctx.aircraft_title.as_deref(), ctx.aircraft_atc_type.as_deref());

    let mut items: Vec<ScoreItem> = Vec::new();
    for rule in RULES.iter() {
        let mut item = (rule.evaluator)(&ctx, rule);
        // (v4.1.0) Filtro por categoría: si la regla no aplica a este
        // avión (ej. transponder IFR en un Cessna), la marcamos "na" → no
        // penaliza el % del checklist (se muestra como "—").
        if !rule_applies_to(rule.id, category) {
            item.passed = true;
            item.severity = "skipped".to_string();
            item.status = "na".to_string();
            item.points_earned = 0;
            item.points_max = 0;
            item.evidence = serde_json::json!({ "reason": "not_applicable_category" });
        }
        items.push(item);
    }

    // (v4.1.0) Orden CRONOLÓGICO del checklist (fase real, luego orden de
    // la regla dentro de la fase). Antes salía alfabético al persistir.
    items.sort_by_key(|i| {
        (
            phase_order(&i.phase),
            rule_index(&i.rule_id).unwrap_or(usize::MAX),
        )
    });

    // (v4.1.0) Modelo de checklist puro: la nota = % de ítems CUMPLIDOS
    // sobre los APLICABLES (los "na" no cuentan). Reusamos las columnas
    // resumen: total = nº cumplidos, max = nº aplicables.
    let total = items.iter().filter(|i| i.status == "done").count() as i64;
    let max = items.iter().filter(|i| i.status != "na").count() as i64;
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
    // (v4.1.0) Solo nos sirve como guard de "¿ya está puntuado?" — los
    // totales/grade se RECOMPUTAN abajo desde los estados de los ítems
    // (modelo de checklist), así que no los vinculamos.
    let Some((Some(_), Some(_), _)) = summary else {
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
    let mut items: Vec<ScoreItem> = items_raw
        .into_iter()
        .map(|(phase, rule_id, label, earned, max_p, passed, sev, ev)| {
            let passed = passed != 0;
            let severity = sev.unwrap_or_else(|| "info".to_string());
            // (v4.1.0) Estado de checklist derivado de passed + severity.
            let status = if severity == "skipped" {
                "na"
            } else if passed {
                "done"
            } else {
                "missed"
            }
            .to_string();
            ScoreItem {
                phase,
                rule_id,
                label,
                points_earned: earned,
                points_max: max_p,
                passed,
                severity,
                status,
                evidence: ev
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null),
            }
        })
        .collect();

    // (v4.1.0) Orden cronológico + % de cumplimiento recomputado de los
    // estados. Esto hace que incluso los vuelos viejos persistidos (con
    // el modelo de puntos ponderados) muestren el orden correcto y el %
    // de checklist correcto, sin necesidad de re-evaluar.
    items.sort_by_key(|i| {
        (
            phase_order(&i.phase),
            rule_index(&i.rule_id).unwrap_or(usize::MAX),
        )
    });
    let total = items.iter().filter(|i| i.status == "done").count() as i64;
    let max = items.iter().filter(|i| i.status != "na").count() as i64;
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
        grade: grade_for_percentage(percentage),
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
/// (v4.16.1 #5b) Captura y persiste el METAR real de salida/llegada del OFP
/// de SimBrief para un vuelo finalizado, para consulta histórica en el
/// FlightBook. Best-effort: si no hay OFP linkeado, ni Pilot ID, o el
/// briefing no coincide con origen/destino, no hace nada. No re-captura si
/// ya hay METAR guardado. NO toca el FPM/touchdown — solo lee/escribe texto.
async fn capture_metar_for_flight(
    pool: &sqlx::SqlitePool,
    http: &reqwest::Client,
    flight_id: i64,
) -> anyhow::Result<()> {
    let row: Option<(Option<String>, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT origin_icao, destination_icao, simbrief_ofp_id, metar_origin \
             FROM flight_log WHERE id = ?1",
        )
        .bind(flight_id)
        .fetch_optional(pool)
        .await?;
    let Some((Some(origin), dest, Some(ofp_id), existing_metar)) = row else {
        return Ok(());
    };
    if ofp_id.is_empty() || existing_metar.is_some() {
        return Ok(()); // sin OFP, o ya capturado
    }
    let Some(pilot_id) = crate::simbrief::get_pilot_id(pool).await? else {
        return Ok(());
    };
    let briefing = crate::simbrief::fetch_briefing(http, &pilot_id).await?;
    let norm = |s: &str| s.trim().to_ascii_uppercase();
    // El briefing es del OFP MÁS RECIENTE — solo lo usamos si coincide con
    // el origen (y destino, si lo hay) de este vuelo.
    let b_orig = briefing.origin_icao.as_deref().map(norm).unwrap_or_default();
    if b_orig != norm(&origin) {
        return Ok(());
    }
    if let Some(d) = dest.as_deref().filter(|d| !d.is_empty()) {
        let b_dest = briefing
            .destination_icao
            .as_deref()
            .map(norm)
            .unwrap_or_default();
        if b_dest != norm(d) {
            return Ok(());
        }
    }
    if briefing.orig_metar.is_none() && briefing.dest_metar.is_none() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE flight_log \
         SET metar_origin = COALESCE(metar_origin, ?2), \
             metar_dest   = COALESCE(metar_dest, ?3) \
         WHERE id = ?1",
    )
    .bind(flight_id)
    .bind(briefing.orig_metar.as_deref())
    .bind(briefing.dest_metar.as_deref())
    .execute(pool)
    .await?;
    tracing::info!(
        target: "scoring",
        "METAR real capturado y persistido para flight {}",
        flight_id
    );
    Ok(())
}

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
    // (v3.31.0 #9) El grade A/B/C acaba de persistirse en flight_log.
    // Emitimos `flightlog://changed` para que el store del FlightBook
    // recargue la lista y el grade aparezca AL INSTANTE en el sidebar,
    // sin tener que cambiar de pantalla y volver (bug reportado).
    let _ = app.emit("flightlog://changed", ());

    // (v4.19.0) Log POR VUELO: archivo `ORIG-DEST-CALLSIGN.log` en
    // <data>/logs/flights con el track completo (luces, flaps, gear,
    // velocidades, fases) + el resultado de la evaluación — para
    // auditar si la Flight Evaluation hace bien su trabajo. Best-effort:
    // jamás bloquea el cierre del vuelo.
    match crate::resolve_portable_data_dir() {
        Ok(data_dir) => {
            match crate::flight_report::write_flight_report(pool, flight_id, &report, &data_dir)
                .await
            {
                Ok(p) => tracing::info!(
                    target: "scoring",
                    "flight report escrito: {}",
                    p.display()
                ),
                Err(e) => tracing::warn!(
                    target: "scoring",
                    "flight report({}) falló: {:#}",
                    flight_id, e
                ),
            }
        }
        Err(e) => tracing::warn!(
            target: "scoring",
            "flight report: data dir no resuelto: {:#}", e
        ),
    }

    // Auto-upload to cloud (non-blocking — spawn detached). Toma el
    // reqwest::Client del AppState compartido (Manager::state).
    let pool_c = pool.clone();
    let app_c = app.clone();
    tokio::spawn(async move {
        use tauri::Manager;
        let state = app_c.state::<crate::AppState>();
        // (v4.16.1 #5b) Captura best-effort del METAR real (OFP de SimBrief)
        // ANTES del upload, para que se persista y sincronice a la nube.
        if let Err(e) = capture_metar_for_flight(&pool_c, &state.http, flight_id).await {
            tracing::debug!(
                target: "scoring",
                "capture_metar({}) omitido: {:#}", flight_id, e
            );
        }
        let _ = app_c.emit(
            "score:upload:started",
            &serde_json::json!({ "flightId": flight_id }),
        );
        match crate::cloud_sync::upload_all(&pool_c, &state.http, None).await {
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
