//! Integración mínima con SimBrief.
//!
//! La API pública es:
//!   `https://www.simbrief.com/api/xml.fetcher.php?userid={pilotId}`
//!
//! Devuelve XML del **último OFP** (Operational Flight Plan)
//! generado por ese piloto. NO hay endpoint público de historial,
//! así que la app va construyendo su propia tabla local: cada
//! refresh descarga el OFP actual y lo persiste si es nuevo.
//!
//! Decisión deliberada: parseamos el XML con regex en lugar de
//! añadir una dependencia (`quick-xml` o `serde-xml-rs`). Sólo
//! necesitamos ~10 campos top-level y los selectores de SimBrief
//! son estables — un parser completo es overkill y aumenta el
//! tamaño del binario sin beneficio.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Vuelo persistido en `simbrief_flights`. Lo expone el comando
/// `list_simbrief_flights` para que el mapa pinte una LineString
/// por cada uno.
/// (v4.10.0) Un punto del plan de ruta (navlog de SimBrief). Cada `<fix>`
/// del OFP: waypoint, VOR, NDB, aeropuerto o punto lat/long, con su etapa
/// (CLB/CRZ/DSC) para poder colorear/declutter por fase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteFix {
    /// Identificador (p.ej. "KOK", "SOVAR", "TOC"). Puede repetirse.
    pub ident: String,
    /// Tipo SimBrief: "apt" | "wpt" | "vor" | "ndb" | "ltlg" | etc.
    pub fix_type: Option<String>,
    pub lat: f64,
    pub lon: f64,
    /// Etapa de vuelo del fix: "CLB" | "CRZ" | "DSC" (vacío en algunos).
    pub stage: Option<String>,
    /// `true` si el fix pertenece a una SID o STAR (procedimiento de
    /// terminal) — útil para declutter (ocultar nombres en zoom bajo).
    pub is_sid_star: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefFlight {
    pub ofp_id: String,
    pub pilot_id: String,
    pub flight_number: Option<String>,
    pub callsign: Option<String>,
    pub aircraft_icao: Option<String>,
    pub origin_icao: String,
    pub origin_name: Option<String>,
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub destination_icao: String,
    pub destination_name: Option<String>,
    pub destination_lat: f64,
    pub destination_lon: f64,
    pub route: Option<String>,
    pub distance_nm: Option<i64>,
    pub est_time_enroute_s: Option<i64>,
    pub generated_at: Option<String>,
    pub fetched_at: String,
    /// (v1.1.0) Pasajeros planificados (`pax_count_actual` o
    /// `pax_count` del bloque `<weights>` del OFP). Lo usa la
    /// integración OUT-event para pre-popular `flight_log.passengers`
    /// — misma data que GSX consume.
    pub pax_count: Option<i64>,
    /// Carga útil planificada en **kg** (siempre normalizada, sin
    /// importar las units del OFP). `<weights><cargo>` o
    /// `<weights><freight_added>` cuando esté.
    pub cargo_kg: Option<i64>,
    /// Combustible planificado a quemar (`enroute_burn + taxi`) en
    /// **kg**. Equivalente al "block fuel burn" que muestra GSX.
    pub fuel_burn_kg: Option<i64>,
    /// `"lbs"` o `"kgs"` — del OFP. Sólo para debug; los campos kg
    /// arriba ya están normalizados.
    pub units: Option<String>,
    /// (v4.0.0 P7.5b) Matrícula (`<aircraft><reg>`) del OFP. SimBrief
    /// la expone como "Custom Tail" — campo per-pilot que cada usuario
    /// configura en sus preferencias. Cuando matchea con `ATC ID` del
    /// avión en el sim, es el discriminador más fuerte para casos de
    /// cuenta SimBrief compartida.
    pub aircraft_reg: Option<String>,
    /// (v4.0.0 P7.5b) Fuel total ramp planificado en kg (`<fuel><plan_ramp>`).
    /// Suma de enroute + taxi + reserve + alternate + extra. Comparable
    /// con `FUEL TOTAL QUANTITY WEIGHT` capturado al OUT — si delta <10%,
    /// es el OFP del piloto que cargó el avión.
    pub plan_ramp_kg: Option<i64>,
    /// (v4.10.0) Puntos del plan de ruta (navlog del OFP). Se persiste
    /// como JSON TEXT en la columna `route_fixes` (decodificado con
    /// `#[sqlx(json)]`). Las SELECT usan `COALESCE(route_fixes,'[]')`
    /// para que filas viejas (NULL) devuelvan lista vacía.
    #[serde(default)]
    #[sqlx(json)]
    pub route_fixes: Vec<RouteFix>,
}

/// Resultado del refresh manual: cuántos OFPs nuevos se añadieron
/// y la URL/error si algo falló. La UI muestra esto como toast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefRefreshResult {
    pub added: usize,
    pub already_known: bool,
    pub flight: Option<SimBriefFlight>,
}

/// (v3.32.0 #4/#6) Briefing meteorológico + NOTAMs del OFP más reciente.
/// **Efímero** — NO se persiste: SimBrief sólo guarda el último OFP y el
/// clima/NOTAMs son del momento de generación. Se obtiene on-demand al
/// abrir Weather/NOTAMs y la UI lo muestra sólo si el OFP matchea
/// origen+destino del vuelo seleccionado (si no, usa el fallback actual /
/// estado vacío). Es clima de la VIDA REAL (METAR/TAF), no del simulador.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefBriefing {
    pub origin_icao: Option<String>,
    pub destination_icao: Option<String>,
    pub alternate_icao: Option<String>,
    pub orig_metar: Option<String>,
    pub orig_taf: Option<String>,
    pub dest_metar: Option<String>,
    pub dest_taf: Option<String>,
    pub altn_metar: Option<String>,
    pub altn_taf: Option<String>,
    pub notams: Vec<SimBriefNotam>,
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefNotam {
    /// ICAO de la localización del NOTAM (origen/destino/alterno/FIR).
    pub location: Option<String>,
    pub text: String,
}

const API_BASE: &str = "https://www.simbrief.com/api/xml.fetcher.php";

/// Descarga el último OFP del piloto y lo persiste. Devuelve si
/// fue una entrada nueva o si ya estaba en la tabla (mismo OFP id).
pub async fn refresh_latest(
    pool: &SqlitePool,
    http: &reqwest::Client,
    pilot_id: &str,
) -> anyhow::Result<SimBriefRefreshResult> {
    let url = format!("{}?userid={}", API_BASE, pilot_id);
    tracing::info!("simbrief: fetching {}", url);

    let xml = http
        .get(&url)
        .header("User-Agent", "SimFleet/3.0")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let flight = parse_ofp_xml(&xml, pilot_id).ok_or_else(|| {
        anyhow::anyhow!(
            "no se pudo parsear el OFP de SimBrief para pilot_id={}",
            pilot_id
        )
    })?;

    // ¿Existe ya el ofp_id?
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT ofp_id FROM simbrief_flights WHERE ofp_id = ?1",
    )
    .bind(&flight.ofp_id)
    .fetch_optional(pool)
    .await?;
    let already_known = existing.is_some();

    sqlx::query(
        r#"
        INSERT INTO simbrief_flights (
            ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
            origin_icao, origin_name, origin_lat, origin_lon,
            destination_icao, destination_name, destination_lat, destination_lon,
            route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
            pax_count, cargo_kg, fuel_burn_kg, units,
            aircraft_reg, plan_ramp_kg, route_fixes
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'), ?18, ?19, ?20, ?21, ?22, ?23, ?24)
        ON CONFLICT(ofp_id) DO UPDATE SET
            fetched_at   = datetime('now'),
            pax_count    = COALESCE(excluded.pax_count, simbrief_flights.pax_count),
            cargo_kg     = COALESCE(excluded.cargo_kg, simbrief_flights.cargo_kg),
            fuel_burn_kg = COALESCE(excluded.fuel_burn_kg, simbrief_flights.fuel_burn_kg),
            units        = COALESCE(excluded.units, simbrief_flights.units),
            aircraft_reg = COALESCE(excluded.aircraft_reg, simbrief_flights.aircraft_reg),
            plan_ramp_kg = COALESCE(excluded.plan_ramp_kg, simbrief_flights.plan_ramp_kg),
            route_fixes  = COALESCE(NULLIF(excluded.route_fixes, '[]'), simbrief_flights.route_fixes)
        "#,
    )
    .bind(&flight.ofp_id)
    .bind(&flight.pilot_id)
    .bind(&flight.flight_number)
    .bind(&flight.callsign)
    .bind(&flight.aircraft_icao)
    .bind(&flight.origin_icao)
    .bind(&flight.origin_name)
    .bind(flight.origin_lat)
    .bind(flight.origin_lon)
    .bind(&flight.destination_icao)
    .bind(&flight.destination_name)
    .bind(flight.destination_lat)
    .bind(flight.destination_lon)
    .bind(&flight.route)
    .bind(flight.distance_nm)
    .bind(flight.est_time_enroute_s)
    .bind(&flight.generated_at)
    .bind(flight.pax_count)
    .bind(flight.cargo_kg)
    .bind(flight.fuel_burn_kg)
    .bind(&flight.units)
    .bind(&flight.aircraft_reg)
    .bind(flight.plan_ramp_kg)
    .bind(serde_json::to_string(&flight.route_fixes).unwrap_or_else(|_| "[]".to_string()))
    .execute(pool)
    .await?;

    Ok(SimBriefRefreshResult {
        added: if already_known { 0 } else { 1 },
        already_known,
        flight: Some(flight),
    })
}

pub async fn list_flights(pool: &SqlitePool) -> anyhow::Result<Vec<SimBriefFlight>> {
    let rows = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
               pax_count, cargo_kg, fuel_burn_kg, units,
               aircraft_reg, plan_ramp_kg,
               COALESCE(route_fixes, '[]') AS route_fixes
        FROM simbrief_flights
        ORDER BY COALESCE(generated_at, fetched_at) DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// (v1.1.0) Busca el OFP más reciente cuyo origen coincida con
/// `origin_icao`. Filtra por antigüedad (default 48h) para evitar
/// matchear OFPs muy viejos con vuelos nuevos del mismo aeropuerto.
///
/// Lo usa el watcher en el OUT event para pre-popular
/// `flight_log.passengers/cargo_kg/fuel_used_kg` con los valores
/// planificados del OFP — misma data que GSX consume.
///
/// (v4.0.0 P7.6) Esta función se reservó para callers que NO tienen
/// info de aircraft. Los call-sites del watcher migran a
/// `find_matching_for_flight`, que cruza ICAO + aircraft + consumed.
pub async fn find_recent_for_origin(
    pool: &SqlitePool,
    origin_icao: &str,
    within_hours: i64,
) -> anyhow::Result<Option<SimBriefFlight>> {
    let row = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
               pax_count, cargo_kg, fuel_burn_kg, units,
               aircraft_reg, plan_ramp_kg,
               COALESCE(route_fixes, '[]') AS route_fixes
        FROM simbrief_flights
        WHERE UPPER(origin_icao) = UPPER(?1)
          AND (
            generated_at IS NOT NULL
            AND CAST(generated_at AS INTEGER) > strftime('%s', 'now', '-' || ?2 || ' hours')
          )
        ORDER BY CAST(generated_at AS INTEGER) DESC
        LIMIT 1
        "#,
    )
    .bind(origin_icao)
    .bind(within_hours)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// (v4.0.0 P7.5b) Resultado scored de un OFP candidato. El breakdown
/// permite logging detallado y UI explicativa al usuario en caso de
/// confirmación manual.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredOfp {
    pub ofp: SimBriefFlight,
    pub score: i32,
    /// Lista de pares (factor, puntos). Factor ej: "reg", "type", "fuel".
    pub breakdown: Vec<(String, i32)>,
}

/// (v4.0.0 P7.5b) Veredicto del scorer SimBrief.
///
/// - `Auto`: hay UN claro ganador (top score ≥ threshold y top - 2do ≥ 15).
/// - `Ambiguous`: 2+ candidatos pasan threshold y empate cerrado → UI debe
///   pedir al usuario que confirme cuál es suyo.
/// - `NoMatch`: ningún candidato pasa threshold.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MatchResult {
    Auto { ofp: SimBriefFlight, score: i32, breakdown: Vec<(String, i32)> },
    Ambiguous { candidates: Vec<ScoredOfp> },
    NoMatch,
}

/// (v4.0.0 P7.5b) Threshold mínimo para considerar un OFP "viable". Un
/// match perfecto por reg da 50 puntos solo — equilibrado para que el
/// caso optimista (reg presente) cierre solo con eso.
const SCORE_THRESHOLD: i32 = 50;
/// Diferencia mínima entre top-1 y top-2 para auto-link. Si la
/// diferencia es menor, hay ambigüedad — UI pregunta.
const SCORE_DOMINANCE_GAP: i32 = 15;

/// (v4.0.0 P7.5b) Scoring multi-factor de OFPs candidatos.
///
/// Reemplaza a `find_matching_for_flight` (v4.0.0 P7.6) que solo hacía
/// match binario por aircraft type. Problema: cuando los pilotos vuelan
/// JUNTOS el mismo avión (caso típico), aircraft type no discrimina.
///
/// Discriminadores (en orden de fuerza):
///
/// 1. **Registration** (`<aircraft><reg>` ↔ `ATC ID`): per-pilot "Custom
///    Tail" de SimBrief. Cuando ambos lados lo tienen y matchean → +50.
///    Mismatch fuerte → -30. Ausencia de uno o ambos → 0 (neutral).
/// 2. **Aircraft type** (`<aircraft><icao>` ↔ `ATC TYPE`, primeros 3 chars):
///    Match → +15. Mismatch → -25 (un vuelo en B738 no usa OFP de A319).
/// 3. **Fuel loaded** (`<fuel><plan_ramp>` ↔ `FUEL TOTAL QUANTITY WEIGHT`
///    al OUT): Δ < 10% → +20. Δ 10-25% → +5. Δ > 25% → -10. Ausencia → 0.
/// 4. **Freshness**: OFP generado en últimas N horas → bonus decreciente
///    de hasta +20 (lineal de 0 a 24h).
///
/// Threshold: score ≥ 50 para considerar viable.
/// Dominancia: top - 2do ≥ 15 para auto-link, sino ambiguous.
pub async fn score_simbrief_candidates(
    pool: &SqlitePool,
    origin_icao: &str,
    aircraft_atc: Option<&str>,
    aircraft_reg: Option<&str>,
    fuel_loaded_lb: Option<f64>,
    within_hours: i64,
    current_flight_id: Option<i64>,
) -> anyhow::Result<MatchResult> {
    // (v4.0.0 P7.6b iter 4) Excluimos `current_flight_id` del filtro
    // de "consumed". Sin esto, al cerrar el vuelo (ended_at NOT NULL)
    // su propio OFP se auto-excluye, la re-validación del linkeo falla
    // y el watcher emite Ambiguous OTRA VEZ — toast post-cierre que no
    // permite click porque la fila ya está cerrada.
    //
    // Bug reportado: usuario clickea toast al OUT → MANUAL LINK OK.
    // Al ENGINE SHUTDOWN, populate re-valida, no encuentra su OFP,
    // desliga (clear flight_number/callsign), emite Ambiguous, toast
    // re-aparece, click no hace nada (link rechazado por ended_at).
    let rows = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
               pax_count, cargo_kg, fuel_burn_kg, units,
               aircraft_reg, plan_ramp_kg,
               COALESCE(route_fixes, '[]') AS route_fixes
        FROM simbrief_flights
        WHERE UPPER(origin_icao) = UPPER(?1)
          AND (
            generated_at IS NOT NULL
            AND CAST(generated_at AS INTEGER) > strftime('%s', 'now', '-' || ?2 || ' hours')
          )
          AND ofp_id NOT IN (
            SELECT simbrief_ofp_id FROM flight_log
            WHERE simbrief_ofp_id IS NOT NULL
              AND ended_at IS NOT NULL
              AND (?3 IS NULL OR id != ?3)
          )
        ORDER BY CAST(generated_at AS INTEGER) DESC
        LIMIT 10
        "#,
    )
    .bind(origin_icao)
    .bind(within_hours)
    .bind(current_flight_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(MatchResult::NoMatch);
    }

    let plane_atc_norm: Option<String> = aircraft_atc
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty());
    let plane_reg_norm: Option<String> = aircraft_reg
        .map(normalize_reg)
        .filter(|s| !s.is_empty());
    let fuel_loaded_kg: Option<f64> = fuel_loaded_lb.map(|lb| lb * 0.4535924);

    // Score cada candidato.
    let mut scored: Vec<ScoredOfp> = Vec::with_capacity(rows.len());
    for ofp in rows.into_iter() {
        // (v4.15.0 #4) Gate de exclusión ESTRICTO por matrícula. Si
        // conocemos la matrícula LOCAL (ATC ID / TAIL NUMBER capturado por
        // SimConnect) y el OFP también trae matrícula pero es DISTINTA,
        // ese plan NO pertenece a esta sesión → se descarta por completo.
        // Esto evita la contaminación cruzada cuando dos pilotos vuelan el
        // MISMO plan a la vez con SimBrief (mismo origen/tipo/fuel): la
        // matrícula es el discriminador estricto. Si el OFP no trae
        // matrícula, seguimos con el scoring normal (best-effort).
        if let (Some(plane), Some(ofp_reg)) =
            (plane_reg_norm.as_deref(), ofp.aircraft_reg.as_deref())
        {
            let ofp_norm = normalize_reg(ofp_reg);
            if !ofp_norm.is_empty() && ofp_norm != plane {
                tracing::info!(
                    target: "simbrief",
                    "ofp_id={} DESCARTADO por matrícula estricta: OFP={:?} != local={:?}",
                    ofp.ofp_id, ofp_norm, plane,
                );
                continue;
            }
        }
        let mut score: i32 = 0;
        let mut breakdown: Vec<(String, i32)> = Vec::new();

        // 1. Registration match
        let reg_pts = match (plane_reg_norm.as_deref(), ofp.aircraft_reg.as_deref()) {
            (Some(plane), Some(ofp_reg)) => {
                let ofp_norm = normalize_reg(ofp_reg);
                if !ofp_norm.is_empty() && ofp_norm == plane {
                    50
                } else if ofp_norm.is_empty() {
                    0
                } else {
                    -30
                }
            }
            _ => 0,
        };
        breakdown.push(("reg".to_string(), reg_pts));
        score += reg_pts;
        let reg_is_strong_match = reg_pts >= 50;

        // 2. Aircraft type match (3-char prefix)
        //
        // (v4.0.0 P7.5b iter 2) Si el reg ya da match fuerte, ignoramos
        // el type completamente. Motivo: SimConnect `ATC TYPE` reporta
        // el FABRICANTE para algunos addons third-party (Fenix devuelve
        // "Airbus" en vez de "A320"), lo que generaba un falso mismatch
        // de -25 puntos contra el "A320" del OFP. Si la matrícula es
        // la misma (`F-HEPI` == `F-HEPI`), por definición es la misma
        // aeronave — no necesitamos validar el type.
        //
        // El type penalty/bonus sigue activo cuando reg NO matchea, ya
        // que ahí es el único discriminador que tenemos.
        let type_pts = if reg_is_strong_match {
            0
        } else {
            match (plane_atc_norm.as_deref(), ofp.aircraft_icao.as_deref()) {
                (Some(plane), Some(ofp_ac)) => {
                    let plane_prefix: String = plane.chars().take(3).collect();
                    let ofp_prefix: String =
                        ofp_ac.trim().to_ascii_uppercase().chars().take(3).collect();
                    if !plane_prefix.is_empty() && plane_prefix == ofp_prefix {
                        15
                    } else if ofp_prefix.is_empty() || plane_prefix.is_empty() {
                        0
                    } else {
                        -25
                    }
                }
                _ => 0,
            }
        };
        breakdown.push(("type".to_string(), type_pts));
        score += type_pts;

        // 3. Fuel match
        let fuel_pts = match (fuel_loaded_kg, ofp.plan_ramp_kg) {
            (Some(loaded), Some(planned)) if planned > 0 => {
                let delta_pct = ((loaded - planned as f64).abs() / planned as f64) * 100.0;
                if delta_pct < 10.0 {
                    20
                } else if delta_pct < 25.0 {
                    5
                } else {
                    -10
                }
            }
            _ => 0,
        };
        breakdown.push(("fuel".to_string(), fuel_pts));
        score += fuel_pts;

        // 4. Freshness — lineal de 20 (0h viejo) a 0 (24h viejo)
        let fresh_pts = ofp
            .generated_at
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .map(|gen_ts| {
                let now_ts = chrono::Utc::now().timestamp();
                let age_h = ((now_ts - gen_ts).max(0)) as f64 / 3600.0;
                ((20.0 - age_h * 20.0 / 24.0).max(0.0)) as i32
            })
            .unwrap_or(0);
        breakdown.push(("freshness".to_string(), fresh_pts));
        score += fresh_pts;

        // Log estructurado por candidato.
        tracing::info!(
            target: "simbrief",
            "score ofp_id={} pilot={} reg={:?} ac={:?} score={} breakdown={:?}",
            ofp.ofp_id,
            ofp.pilot_id,
            ofp.aircraft_reg,
            ofp.aircraft_icao,
            score,
            breakdown,
        );

        scored.push(ScoredOfp { ofp, score, breakdown });
    }

    // Ordenar por score desc.
    scored.sort_by(|a, b| b.score.cmp(&a.score));

    // Filtrar viables.
    let viable: Vec<ScoredOfp> = scored
        .into_iter()
        .filter(|s| s.score >= SCORE_THRESHOLD)
        .collect();

    if viable.is_empty() {
        return Ok(MatchResult::NoMatch);
    }

    // Único viable → Auto.
    if viable.len() == 1 {
        let top = viable.into_iter().next().unwrap();
        return Ok(MatchResult::Auto {
            ofp: top.ofp,
            score: top.score,
            breakdown: top.breakdown,
        });
    }

    // Múltiples viables → checar dominancia.
    let top_score = viable[0].score;
    let runner_score = viable[1].score;
    if top_score - runner_score >= SCORE_DOMINANCE_GAP {
        let top = viable.into_iter().next().unwrap();
        return Ok(MatchResult::Auto {
            ofp: top.ofp,
            score: top.score,
            breakdown: top.breakdown,
        });
    }

    // Ambiguous — UI debe pedir confirmación.
    Ok(MatchResult::Ambiguous { candidates: viable })
}

/// Normaliza una matrícula: uppercase, sin espacios ni guiones. Cubre
/// formatos `PT-TMC` / `PT TMC` / `pttmc` → `PTTMC`.
fn normalize_reg(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

pub async fn delete_flight(pool: &SqlitePool, ofp_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM simbrief_flights WHERE ofp_id = ?1")
        .bind(ofp_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_pilot_id(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'simbrief_pilot_id'")
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(v,)| v).filter(|s| !s.trim().is_empty()))
}

pub async fn set_pilot_id(pool: &SqlitePool, pilot_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES ('simbrief_pilot_id', ?1, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(pilot_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---- Parser XML ------------------------------------------------------------

/// Selectores precompilados para los campos que necesitamos.
/// Los nombres de tag de SimBrief son estables desde 2018+;
/// si cambian, el `Regex::captures` devuelve `None` y el campo
/// se queda en `Option::None` — degradación elegante.
fn tag_re(tag: &str) -> Regex {
    // Acepta atributos en la apertura: `<icao_code>` o `<icao_code attr="…">`.
    // Captura todo entre apertura y cierre, sin importar saltos de línea.
    Regex::new(&format!(r"(?s)<{tag}(?:\s[^>]*)?>(.*?)</{tag}>")).unwrap()
}

static TAG_OFP_ID: Lazy<Regex> = Lazy::new(|| tag_re("request_id"));
static TAG_FLIGHT_NUMBER: Lazy<Regex> = Lazy::new(|| tag_re("flight_number"));
static TAG_CALLSIGN: Lazy<Regex> = Lazy::new(|| tag_re("callsign"));
// `TAG_AIRCRAFT_ICAO` se quitó: usamos `inner_tag(aircraft_block, "icao_code")`
// para no chocar con los `<icao_code>` de origin/destination.
static TAG_ROUTE: Lazy<Regex> = Lazy::new(|| tag_re("route"));
static TAG_DISTANCE: Lazy<Regex> = Lazy::new(|| tag_re("air_distance"));
static TAG_EST_TIME: Lazy<Regex> = Lazy::new(|| tag_re("est_time_enroute"));
static TAG_TIME_GENERATED: Lazy<Regex> = Lazy::new(|| tag_re("time_generated"));
static TAG_ORIGIN: Lazy<Regex> = Lazy::new(|| tag_re("origin"));
static TAG_DEST: Lazy<Regex> = Lazy::new(|| tag_re("destination"));
static TAG_AIRCRAFT: Lazy<Regex> = Lazy::new(|| tag_re("aircraft"));
static TAG_WEIGHTS: Lazy<Regex> = Lazy::new(|| tag_re("weights"));
static TAG_FUEL: Lazy<Regex> = Lazy::new(|| tag_re("fuel"));
static TAG_PARAMS: Lazy<Regex> = Lazy::new(|| tag_re("params"));
// (v3.32.0 #4/#6) Weather + alterno + NOTAMs.
static TAG_WEATHER: Lazy<Regex> = Lazy::new(|| tag_re("weather"));
static TAG_ALTERNATE: Lazy<Regex> = Lazy::new(|| tag_re("alternate"));
static TAG_NOTAMS: Lazy<Regex> = Lazy::new(|| tag_re("notams"));
static TAG_NOTAMDREC: Lazy<Regex> = Lazy::new(|| tag_re("notamdrec"));

static TAG_NAVLOG: Lazy<Regex> = Lazy::new(|| tag_re("navlog"));
static TAG_FIX: Lazy<Regex> = Lazy::new(|| tag_re("fix"));

/// Extrae los `<fix>` del bloque `<navlog>` del OFP. Cada fix con su
/// ident, tipo, lat/lon y etapa. Degradación elegante: si falta lat/lon
/// el fix se omite; si no hay navlog devuelve lista vacía.
pub fn parse_navlog(xml: &str) -> Vec<RouteFix> {
    let navlog = match TAG_NAVLOG.captures(xml).and_then(|c| c.get(1)) {
        Some(m) => m.as_str(),
        None => return Vec::new(),
    };
    let mut fixes = Vec::new();
    for cap in TAG_FIX.captures_iter(navlog) {
        let block = match cap.get(1) {
            Some(m) => m.as_str(),
            None => continue,
        };
        let lat = inner_tag(block, "pos_lat").and_then(|s| s.trim().parse::<f64>().ok());
        let lon = inner_tag(block, "pos_long").and_then(|s| s.trim().parse::<f64>().ok());
        let (lat, lon) = match (lat, lon) {
            (Some(la), Some(lo)) if la.abs() <= 90.0 && lo.abs() <= 180.0 => (la, lo),
            _ => continue,
        };
        let ident = inner_tag(block, "ident")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "•".to_string());
        let is_sid_star = inner_tag(block, "is_sid_star")
            .map(|s| {
                let t = s.trim();
                !t.is_empty() && t != "0"
            })
            .unwrap_or(false);
        fixes.push(RouteFix {
            ident,
            fix_type: inner_tag(block, "type").map(|s| s.trim().to_string()),
            lat,
            lon,
            stage: inner_tag(block, "stage").map(|s| s.trim().to_string()),
            is_sid_star,
        });
    }
    fixes
}

/// Parser principal. Devuelve `None` si faltan los campos
/// imprescindibles (origen, destino con lat/lon).
pub fn parse_ofp_xml(xml: &str, pilot_id: &str) -> Option<SimBriefFlight> {
    let origin_block = TAG_ORIGIN.captures(xml)?.get(1)?.as_str();
    let dest_block = TAG_DEST.captures(xml)?.get(1)?.as_str();

    let origin_icao = inner_tag(origin_block, "icao_code")?;
    let origin_lat: f64 = inner_tag(origin_block, "pos_lat")?.parse().ok()?;
    let origin_lon: f64 = inner_tag(origin_block, "pos_long")?.parse().ok()?;
    let origin_name = inner_tag(origin_block, "name");

    let destination_icao = inner_tag(dest_block, "icao_code")?;
    let destination_lat: f64 = inner_tag(dest_block, "pos_lat")?.parse().ok()?;
    let destination_lon: f64 = inner_tag(dest_block, "pos_long")?.parse().ok()?;
    let destination_name = inner_tag(dest_block, "name");

    // Aircraft block puede no estar — fallback a None.
    let aircraft_block = TAG_AIRCRAFT
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let aircraft_icao = aircraft_block
        .as_deref()
        .and_then(|b| inner_tag(b, "icao_code"));
    // (v4.0.0 P7.5b) Matrícula "Custom Tail" del piloto. SimBrief lo
    // expone como `<reg>` dentro del bloque `<aircraft>`. Si el piloto
    // no lo configura en sus preferencias, queda vacío o como un valor
    // genérico (ej. ICAO genérico del tipo). Normalizamos a uppercase y
    // strip espacios/guiones para comparar contra `ATC ID` del sim.
    let aircraft_reg = aircraft_block
        .as_deref()
        .and_then(|b| inner_tag(b, "reg"))
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty());

    let ofp_id = first_tag(&TAG_OFP_ID, xml).unwrap_or_else(|| {
        // Sin request_id no podemos deduplicar — usamos hash del
        // contenido como fallback.
        format!("nohash-{}", xml.len())
    });

    // (v1.1.0) Units viene en `<params><units>`. Por defecto SimBrief
    // emite "lbs" para users US y "kgs" para users con preferencia
    // métrica. Lo usamos para convertir todos los pesos a kg.
    let units = TAG_PARAMS
        .captures(xml)
        .and_then(|c| c.get(1))
        .and_then(|m| inner_tag(m.as_str(), "units"))
        .map(|s| s.to_lowercase());
    let lb_to_kg = matches!(units.as_deref(), Some("lbs"));
    let convert = |v: i64| -> i64 {
        if lb_to_kg {
            ((v as f64) * 0.4535924).round() as i64
        } else {
            v
        }
    };

    // Weights block — pax_count_actual prefiere, sino pax_count.
    // Cargo: `<weights><cargo>` es el más usado.
    let (pax_count, cargo_kg) = TAG_WEIGHTS
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| {
            let block = m.as_str();
            let pax = inner_tag(block, "pax_count_actual")
                .or_else(|| inner_tag(block, "pax_count"))
                .and_then(|s| s.trim().parse::<i64>().ok());
            let cargo = inner_tag(block, "cargo")
                .or_else(|| inner_tag(block, "freight_added"))
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(convert);
            (pax, cargo)
        })
        .unwrap_or((None, None));

    // Fuel block — `enroute_burn + taxi` es la mejor aproximación al
    // "fuel quemado durante el bloque" que GSX usa.
    // (v4.0.0 P7.5b) Adicionalmente extraemos `plan_ramp` (total fuel
    // al bloque planificado = enroute + taxi + reserve + alternate +
    // extra). Esto se compara contra el FUEL TOTAL QUANTITY WEIGHT del
    // sim al OUT — si delta < 10%, es el OFP del piloto que cargó.
    let fuel_block_str = TAG_FUEL
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let fuel_burn_kg = fuel_block_str.as_deref().and_then(|block| {
        let enroute = inner_tag(block, "enroute_burn")
            .and_then(|s| s.trim().parse::<i64>().ok())?;
        let taxi = inner_tag(block, "taxi")
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        Some(convert(enroute + taxi))
    });
    let plan_ramp_kg = fuel_block_str.as_deref().and_then(|block| {
        inner_tag(block, "plan_ramp")
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(convert)
    });

    Some(SimBriefFlight {
        ofp_id,
        pilot_id: pilot_id.to_string(),
        flight_number: first_tag(&TAG_FLIGHT_NUMBER, xml),
        callsign: first_tag(&TAG_CALLSIGN, xml),
        aircraft_icao,
        origin_icao,
        origin_name,
        origin_lat,
        origin_lon,
        destination_icao,
        destination_name,
        destination_lat,
        destination_lon,
        route: first_tag(&TAG_ROUTE, xml),
        distance_nm: first_tag(&TAG_DISTANCE, xml).and_then(|s| s.trim().parse().ok()),
        est_time_enroute_s: first_tag(&TAG_EST_TIME, xml)
            .and_then(|s| s.trim().parse().ok()),
        generated_at: first_tag(&TAG_TIME_GENERATED, xml),
        fetched_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        pax_count,
        cargo_kg,
        fuel_burn_kg,
        units,
        aircraft_reg,
        plan_ramp_kg,
        route_fixes: parse_navlog(xml),
    })
}

/// (v3.32.0 #4/#6) Baja el OFP más reciente del piloto y extrae SÓLO el
/// briefing (weather + NOTAMs). No toca la DB — es on-demand y efímero.
pub async fn fetch_briefing(
    http: &reqwest::Client,
    pilot_id: &str,
) -> anyhow::Result<SimBriefBriefing> {
    let url = format!("{}?userid={}", API_BASE, pilot_id);
    tracing::info!(target: "simbrief", "briefing: fetching {}", url);
    let xml = http
        .get(&url)
        .header("User-Agent", "SimFleet/3.0")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_briefing_xml(&xml))
}

/// Extrae `<weather>` (METAR/TAF de origen/destino/alterno) + NOTAMs del
/// OFP. Degradación elegante: cualquier campo ausente → None / lista vacía.
pub fn parse_briefing_xml(xml: &str) -> SimBriefBriefing {
    let origin_block = TAG_ORIGIN
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let dest_block = TAG_DEST
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let altn_block = TAG_ALTERNATE
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let weather_block = TAG_WEATHER
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let icao = |b: &Option<String>| b.as_deref().and_then(|x| inner_tag(x, "icao_code"));
    let wx = |tag: &str| weather_block.as_deref().and_then(|w| inner_tag(w, tag));

    SimBriefBriefing {
        origin_icao: icao(&origin_block),
        destination_icao: icao(&dest_block),
        alternate_icao: icao(&altn_block),
        orig_metar: wx("orig_metar"),
        orig_taf: wx("orig_taf"),
        dest_metar: wx("dest_metar"),
        dest_taf: wx("dest_taf"),
        altn_metar: wx("altn_metar"),
        altn_taf: wx("altn_taf"),
        notams: parse_notams(xml),
        generated_at: first_tag(&TAG_TIME_GENERATED, xml),
    }
}

/// NOTAMs del OFP (best-effort). SimBrief los expone en `<notams>` con
/// hijos `<notamdrec>` SÓLO si el usuario activa NOTAMs en su layout; si
/// no, la lista queda vacía y la UI muestra estado vacío. De cada registro
/// tomamos el ICAO de localización y el texto, probando varios nombres de
/// campo por compat entre versiones del schema.
fn parse_notams(xml: &str) -> Vec<SimBriefNotam> {
    let Some(block) = TAG_NOTAMS
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cap in TAG_NOTAMDREC.captures_iter(&block) {
        let rec = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let location = inner_tag(rec, "location_icao")
            .or_else(|| inner_tag(rec, "location_id"))
            .or_else(|| inner_tag(rec, "location"));
        let text = inner_tag(rec, "notam_text")
            .or_else(|| inner_tag(rec, "all"))
            .or_else(|| inner_tag(rec, "notam_html"));
        if let Some(text) = text {
            out.push(SimBriefNotam { location, text });
        }
    }
    out
}

fn first_tag(re: &Regex, s: &str) -> Option<String> {
    re.captures(s)
        .and_then(|c| c.get(1))
        .map(|m| decode_entities(m.as_str().trim()))
        .filter(|s| !s.is_empty())
}

fn inner_tag(block: &str, tag: &str) -> Option<String> {
    let re = tag_re(tag);
    re.captures(block)
        .and_then(|c| c.get(1))
        .map(|m| decode_entities(m.as_str().trim()))
        .filter(|s| !s.is_empty())
}

/// Decodifica las pocas entidades XML que SimBrief puede emitir
/// (`&amp;`, `&lt;`, `&gt;`). Suficiente para los campos que
/// extraemos.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<OFP>
  <fetch>
    <request_id>123456</request_id>
    <userid>50956</userid>
  </fetch>
  <params>
    <units>lbs</units>
    <time_generated>1735689600</time_generated>
  </params>
  <general>
    <flight_number>BEL341</flight_number>
    <icao_airline>BEL</icao_airline>
    <route>EBBR DCT KOK DCT MAD</route>
    <air_distance>720</air_distance>
  </general>
  <origin>
    <icao_code>EBBR</icao_code>
    <pos_lat>50.901</pos_lat>
    <pos_long>4.484</pos_long>
    <name>Brussels Airport</name>
  </origin>
  <destination>
    <icao_code>LEMD</icao_code>
    <pos_lat>40.471</pos_lat>
    <pos_long>-3.564</pos_long>
    <name>Madrid Barajas</name>
  </destination>
  <aircraft>
    <icao_code>A320</icao_code>
  </aircraft>
  <weights>
    <pax_count>148</pax_count>
    <pax_count_actual>148</pax_count_actual>
    <cargo>3320</cargo>
  </weights>
  <fuel>
    <taxi>800</taxi>
    <enroute_burn>7800</enroute_burn>
  </fuel>
  <times>
    <est_time_enroute>5400</est_time_enroute>
  </times>
  <navlog>
    <fix>
      <ident>KOK</ident>
      <type>wpt</type>
      <pos_lat>51.09</pos_lat>
      <pos_long>2.65</pos_long>
      <stage>CLB</stage>
      <is_sid_star>1</is_sid_star>
    </fix>
    <fix>
      <ident>TOC</ident>
      <type>wpt</type>
      <pos_lat>49.5</pos_lat>
      <pos_long>1.2</pos_long>
      <stage>CRZ</stage>
      <is_sid_star>0</is_sid_star>
    </fix>
    <fix>
      <ident>MAD</ident>
      <type>vor</type>
      <pos_lat>40.48</pos_lat>
      <pos_long>-3.45</pos_long>
      <stage>DSC</stage>
      <is_sid_star>0</is_sid_star>
    </fix>
  </navlog>
  <api_params>
    <time_generated>1735689600</time_generated>
  </api_params>
</OFP>"#;

    #[test]
    fn parses_real_simbrief_xml() {
        let f = parse_ofp_xml(SAMPLE_XML, "50956").expect("parse should succeed");
        assert_eq!(f.ofp_id, "123456");
        assert_eq!(f.pilot_id, "50956");
        assert_eq!(f.origin_icao, "EBBR");
        assert_eq!(f.destination_icao, "LEMD");
        assert!((f.origin_lat - 50.901).abs() < 0.01);
        assert!((f.destination_lon - -3.564).abs() < 0.01);
        assert_eq!(f.aircraft_icao.as_deref(), Some("A320"));
        assert_eq!(f.distance_nm, Some(720));
        assert_eq!(f.est_time_enroute_s, Some(5400));
        // (v1.1.0) Weights/fuel parsing — units=lbs, así que
        // cargo 3320 lb → ~1506 kg, fuel (7800+800)=8600 lb → ~3901 kg.
        assert_eq!(f.units.as_deref(), Some("lbs"));
        assert_eq!(f.pax_count, Some(148));
        let cargo = f.cargo_kg.expect("cargo_kg parsed");
        assert!(
            (cargo - 1506).abs() <= 2,
            "cargo_kg = {} (esperado ~1506)",
            cargo
        );
        let fuel = f.fuel_burn_kg.expect("fuel_burn_kg parsed");
        assert!(
            (fuel - 3901).abs() <= 2,
            "fuel_burn_kg = {} (esperado ~3901)",
            fuel
        );
    }

    #[test]
    fn parses_navlog_fixes() {
        let f = parse_ofp_xml(SAMPLE_XML, "50956").expect("parse should succeed");
        assert_eq!(f.route_fixes.len(), 3, "deben parsearse 3 fixes");
        let kok = &f.route_fixes[0];
        assert_eq!(kok.ident, "KOK");
        assert_eq!(kok.fix_type.as_deref(), Some("wpt"));
        assert_eq!(kok.stage.as_deref(), Some("CLB"));
        assert!(kok.is_sid_star, "KOK es parte de SID");
        assert!((kok.lat - 51.09).abs() < 0.01);
        assert!((kok.lon - 2.65).abs() < 0.01);
        assert!(!f.route_fixes[1].is_sid_star, "TOC no es SID/STAR");
        assert_eq!(f.route_fixes[2].ident, "MAD");
        assert!((f.route_fixes[2].lon - -3.45).abs() < 0.01);
    }

    #[test]
    fn navlog_absent_yields_empty() {
        let xml = r#"<?xml version="1.0"?>
<OFP>
  <fetch><request_id>1</request_id></fetch>
  <params><units>kgs</units></params>
  <origin><icao_code>LEMD</icao_code><pos_lat>40</pos_lat><pos_long>-3</pos_long></origin>
  <destination><icao_code>EBBR</icao_code><pos_lat>50</pos_lat><pos_long>4</pos_long></destination>
</OFP>"#;
        let f = parse_ofp_xml(xml, "1").expect("parse");
        assert!(f.route_fixes.is_empty());
    }

    #[test]
    fn parses_kg_units_without_conversion() {
        let xml = r#"<?xml version="1.0"?>
<OFP>
  <fetch><request_id>1</request_id></fetch>
  <params><units>kgs</units></params>
  <origin><icao_code>LEMD</icao_code><pos_lat>40</pos_lat><pos_long>-3</pos_long></origin>
  <destination><icao_code>EBBR</icao_code><pos_lat>50</pos_lat><pos_long>4</pos_long></destination>
  <weights><pax_count>180</pax_count><cargo>2500</cargo></weights>
  <fuel><taxi>300</taxi><enroute_burn>4200</enroute_burn></fuel>
</OFP>"#;
        let f = parse_ofp_xml(xml, "1").expect("parse");
        assert_eq!(f.units.as_deref(), Some("kgs"));
        assert_eq!(f.pax_count, Some(180));
        // sin conversión: cargo 2500 kg, fuel (4200+300)=4500 kg.
        assert_eq!(f.cargo_kg, Some(2500));
        assert_eq!(f.fuel_burn_kg, Some(4500));
    }

    #[test]
    fn returns_none_when_origin_missing() {
        let xml = "<OFP><general/></OFP>";
        assert!(parse_ofp_xml(xml, "1").is_none());
    }
}
