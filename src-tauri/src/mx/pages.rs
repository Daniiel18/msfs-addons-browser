//! (02/11) Etapa 5b — pantallas Vuelos, Cabina y Cockpit del Tech Log propio
//! (aviones sin Fenix/PMDG TechLog), con los mismos tipos que las de TechLog
//! para que el Hangar las dibuje igual.
//!
//! · Vuelos: el FlightBook (ruta, horas, pasajeros, toma, nota y su traza
//!   cada 10 s) más el registro del Tech Log propio (eventos, desgaste,
//!   defectos que dejó y el informe de vida de piezas).

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

use crate::airline_economy::normalize_reg;
use crate::techlog::TlSource;
use crate::techlog_pages::{
    col_f64, col_i64, localize_detail, localize_plan, TlCabinPage, TlEffect, TlEvent, TlFlightDetail, TlFlightRow,
    TlFlightsPage, TlLandingSummary, TlScoreItem, TlSeatInfo, TlTrack, TlWear, TRACK_MAX_ROWS,
};

/// (02/11) Cabina propia: siempre "en vivo" (es de SimFleet). Sin plano si el
/// avión no tiene cabina de pasajeros (cargueros, avionetas).
pub async fn own_cabin(pool: &SqlitePool, registration: &str) -> TlCabinPage {
    let plan = match crate::mx::store::cabin_view(pool, registration).await {
        Ok(Some(mut p)) => {
            localize_plan(&mut p);
            Some(p)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(target: "mx", "cabina de {registration}: {e:#}");
            None
        }
    };
    TlCabinPage {
        state: "live",
        source: Some(TlSource::Simfleet),
        active_registration: None,
        saved_at: None,
        plan,
    }
}

/// (02/11) Plazas y asientos bloqueados de la Cabina propia (pre-vuelo).
pub async fn own_seat_info(pool: &SqlitePool, registration: &str) -> Result<Option<TlSeatInfo>> {
    let Some(plan) = crate::mx::store::cabin_view(pool, registration).await? else {
        return Ok(None);
    };
    if plan.seats.is_empty() {
        return Ok(None);
    }
    Ok(Some(TlSeatInfo {
        total: plan.seats.len() as i64,
        blocked: plan.seats.iter().filter(|s| s.blocked).map(|s| s.seat.clone()).collect(),
    }))
}

fn utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)).ok()
}

fn minutes(a: Option<&str>, b: Option<&str>, paused_s: f64) -> Option<f64> {
    let (a, b) = (utc(a?)?, utc(b?)?);
    let m = ((b - a).num_seconds() as f64 - paused_s).max(0.0) / 60.0;
    Some((m * 10.0).round() / 10.0)
}

/// Fila de la lista a partir del FlightBook (más lo que dejó en el Tech Log).
fn row_of(r: &sqlx::sqlite::SqliteRow, warnings: i64, effects: i64) -> TlFlightRow {
    let s = |c: &str| r.try_get::<Option<String>, _>(c).ok().flatten().filter(|v| !v.is_empty());
    let started = s("started_at");
    let ended = s("ended_at");
    let paused = col_f64(r, "paused_seconds").unwrap_or(0.0);
    let score = match (col_f64(r, "score_total"), col_f64(r, "score_max")) {
        (Some(t), Some(m)) if m > 0.0 => Some((t / m * 100.0).round()),
        _ => None,
    };
    let fpm = col_f64(r, "landing_fpm");
    TlFlightRow {
        id: r.try_get("id").unwrap_or(0),
        status: if ended.is_some() { "closed".into() } else { "open".into() },
        opened_utc: started.clone(),
        out_utc: started.clone(),
        in_utc: ended.clone(),
        from: s("origin_icao"),
        to: s("destination_icao"),
        block_min: minutes(started.as_deref(), ended.as_deref(), paused),
        air_min: col_f64(r, "flight_time_s").map(|v| (v / 6.0).round() / 10.0),
        score,
        flags: Vec::new(),
        pax: col_i64(r, "passengers").filter(|p| *p > 0),
        distance_nm: col_f64(r, "distance_nm"),
        max_alt_ft: col_f64(r, "max_altitude_ft"),
        landing: fpm.map(|v| TlLandingSummary {
            airport: s("destination_icao"),
            vs_fpm: Some(v),
            bounces: col_i64(r, "bounced"),
            ..Default::default()
        }),
        warnings,
        effects,
    }
}

/// Eventos guardados por el Tech Log propio para un vuelo.
fn mx_events(raw: Option<String>) -> Vec<Value> {
    raw.and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok()).unwrap_or_default()
}

/// Vuelos de un avión del Tech Log propio (los del FlightBook, recientes primero).
pub async fn own_flights(pool: &SqlitePool, registration: &str, limit: i64) -> Result<Option<TlFlightsPage>> {
    let reg = normalize_reg(registration);
    let rows = sqlx::query(
        "SELECT * FROM flight_log WHERE aircraft_registration IS NOT NULL AND aircraft_registration <> ''
         ORDER BY started_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let mine: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            r.try_get::<Option<String>, _>("aircraft_registration")
                .ok()
                .flatten()
                .is_some_and(|x| normalize_reg(&x) == reg)
        })
        .take(limit.clamp(1, 500) as usize)
        .collect();
    if mine.is_empty() {
        return Ok(None);
    }
    let mut warnings: HashMap<i64, i64> = HashMap::new();
    for m in sqlx::query("SELECT flight_id, events FROM mx_flight WHERE registration = ?1")
        .bind(&reg)
        .fetch_all(pool)
        .await?
    {
        let n = mx_events(m.try_get("events").ok())
            .iter()
            .filter(|e| e.get("severity").and_then(|v| v.as_str()).is_some_and(|s| s != "Info"))
            .count() as i64;
        warnings.insert(m.try_get("flight_id").unwrap_or(0), n);
    }
    let mut effects: HashMap<i64, i64> = HashMap::new();
    for (fid, n) in sqlx::query_as::<_, (i64, i64)>(
        "SELECT raised_flight_id, COUNT(*) FROM mx_defect WHERE registration = ?1 AND raised_flight_id IS NOT NULL GROUP BY raised_flight_id",
    )
    .bind(&reg)
    .fetch_all(pool)
    .await?
    {
        effects.insert(fid, n);
    }
    Ok(Some(TlFlightsPage {
        source: TlSource::Simfleet,
        app_name: TlSource::Simfleet.app_name().into(),
        flights: mine
            .iter()
            .map(|r| {
                let id: i64 = r.try_get("id").unwrap_or(0);
                row_of(r, warnings.get(&id).copied().unwrap_or(0), effects.get(&id).copied().unwrap_or(0))
            })
            .collect(),
    }))
}

/// Punto de la traza del FlightBook.
struct Pt {
    t: DateTime<Utc>,
    alt: Option<f64>,
    ias: Option<f64>,
    gs: Option<f64>,
    vs: Option<f64>,
    g: Option<f64>,
    wind_dir: Option<f64>,
    wind_kt: Option<f64>,
}

fn closest(track: &[Pt], at: DateTime<Utc>) -> Option<&Pt> {
    track.iter().min_by_key(|p| (p.t - at).num_milliseconds().abs())
}

/// Detalle de un vuelo propio con las mismas pestañas que los de TechLog.
pub async fn own_flight_detail(pool: &SqlitePool, registration: &str, flight_id: i64) -> Result<Option<TlFlightDetail>> {
    let reg = normalize_reg(registration);
    let Some(r) = sqlx::query("SELECT * FROM flight_log WHERE id = ?1")
        .bind(flight_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let flight_reg: Option<String> = r.try_get("aircraft_registration").ok().flatten();
    if flight_reg.as_deref().map(normalize_reg).as_deref() != Some(reg.as_str()) {
        return Ok(None);
    }
    let mx = sqlx::query("SELECT summary, events, wear FROM mx_flight WHERE flight_id = ?1")
        .bind(flight_id)
        .fetch_optional(pool)
        .await?;
    let events_raw = mx_events(mx.as_ref().and_then(|m| m.try_get("events").ok()));
    let wear: Vec<TlWear> = mx
        .as_ref()
        .and_then(|m| m.try_get::<String, _>("wear").ok())
        .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|w| {
            Some(TlWear {
                item: w.get("item")?.as_str()?.to_string(),
                amount: w.get("amount")?.as_f64()?,
                unit: w.get("unit")?.as_str()?.to_string(),
                detail: None,
            })
        })
        .filter(|w| w.amount.abs() > 0.0001)
        .collect();
    let warnings = events_raw
        .iter()
        .filter(|e| e.get("severity").and_then(|v| v.as_str()).is_some_and(|s| s != "Info"))
        .count() as i64;
    let effects: Vec<TlEffect> = sqlx::query(
        "SELECT id, ata, title, source, status, mel_id, category FROM mx_defect WHERE raised_flight_id = ?1 ORDER BY id",
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|d| TlEffect {
        defect_id: d.try_get("id").ok(),
        ata: col_i64(d, "ata"),
        system: String::new(),
        title: d.try_get("title").unwrap_or_default(),
        origin: d.try_get::<Option<String>, _>("source").ok().flatten().map(|s| s.to_lowercase()),
        restriction: d
            .try_get::<Option<String>, _>("mel_id")
            .ok()
            .flatten()
            .map(|m| format!("MEL {m} · {}", d.try_get::<String, _>("category").unwrap_or_default())),
        complied: None,
        compliance_note: None,
        outcome: d.try_get::<Option<String>, _>("status").ok().flatten(),
    })
    .collect();

    // Fases (con hora) y, tras cada una, lo que el Tech Log anotó en ella.
    let phases: Vec<(String, String)> =
        sqlx::query_as("SELECT phase, entered_at FROM flight_log_phase WHERE flight_id = ?1 ORDER BY id")
            .bind(flight_id)
            .fetch_all(pool)
            .await?;
    let mut events: Vec<TlEvent> = Vec::new();
    let mut used = vec![false; events_raw.len()];
    let start = r.try_get::<Option<String>, _>("started_at").ok().flatten().as_deref().and_then(utc);
    let mx_event = |i: usize, e: &Value| TlEvent {
        id: 100_000 + i as i64,
        phase: e.get("phase").and_then(|v| v.as_str()).map(|s| s.to_lowercase()),
        kind: e.get("kind").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        severity: e.get("severity").and_then(|v| v.as_str()).unwrap_or("Info").to_string(),
        text: e.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        ..Default::default()
    };
    for (n, (phase, at)) in phases.iter().enumerate() {
        let when = utc(at);
        events.push(TlEvent {
            id: n as i64,
            utc: Some(at.clone()),
            t: match (start, when) {
                (Some(s), Some(w)) => Some((w - s).num_seconds() as f64),
                _ => None,
            },
            phase: Some(phase.clone()),
            kind: "PHASE".into(),
            severity: "Info".into(),
            text: phase.clone(),
            ..Default::default()
        });
        // El Tech Log apunta sus eventos por fase en mayúsculas ("TAKEOFF").
        for (i, e) in events_raw.iter().enumerate() {
            let p = e.get("phase").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
            if !used[i] && !p.is_empty() && phase.starts_with(&p) {
                used[i] = true;
                events.push(mx_event(i, e));
            }
        }
    }
    for (i, e) in events_raw.iter().enumerate() {
        if !used[i] {
            events.push(mx_event(i, e));
        }
    }

    // Traza (cada 10 s): perfil, despegue y toma.
    let track: Vec<Pt> = sqlx::query(
        "SELECT ts, alt_ft, ias_kt, gs_kt, vs_fpm, g_force, wind_dir_deg, wind_speed_kt
         FROM flight_log_track WHERE flight_id = ?1 ORDER BY ts",
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?
    .iter()
    .filter_map(|p| {
        Some(Pt {
            t: utc(&p.try_get::<String, _>("ts").ok()?)?,
            alt: col_f64(p, "alt_ft"),
            ias: col_f64(p, "ias_kt"),
            gs: col_f64(p, "gs_kt"),
            vs: col_f64(p, "vs_fpm"),
            g: col_f64(p, "g_force"),
            wind_dir: col_f64(p, "wind_dir_deg"),
            wind_kt: col_f64(p, "wind_speed_kt"),
        })
    })
    .collect();
    let phase_at = |names: &[&str]| -> Option<DateTime<Utc>> {
        phases
            .iter()
            .find(|(p, _)| names.contains(&p.as_str()))
            .and_then(|(_, at)| utc(at))
    };
    let off = phase_at(&["initial_climb", "climb"]);
    let on = phase_at(&["landing", "taxi_in", "arrived"]);
    let s = |c: &str| r.try_get::<Option<String>, _>(c).ok().flatten().filter(|v| !v.is_empty());

    // La traza es cada 10 s: no vale para la rotación ni el cabeceo en la toma.
    let takeoff = phase_at(&["takeoff", "takeoff_accel"]).map(|t0| {
        let lift = track
            .iter()
            .filter(|p| p.t >= t0 && off.map_or(true, |o| p.t <= o))
            .find(|p| p.vs.is_some_and(|v| v > 250.0));
        json!({
            "airport": s("origin_icao"),
            "liftoffIas": lift.and_then(|p| p.ias),
            "liftoffGs": lift.and_then(|p| p.gs),
        })
    });
    let landing = on.map(|t1| {
        let tp = closest(&track, t1);
        let peak_g = track
            .iter()
            .filter(|p| (p.t - t1).num_seconds().abs() <= 30)
            .filter_map(|p| p.g)
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        json!({
            "index": 1,
            "utc": t1.to_rfc3339(),
            "airport": s("destination_icao"),
            "vsFpm": col_f64(&r, "landing_fpm"),
            "peakG": peak_g,
            "ias": tp.and_then(|p| p.ias),
            "gs": tp.and_then(|p| p.gs),
            "windDir": tp.and_then(|p| p.wind_dir),
            "windKt": tp.and_then(|p| p.wind_kt),
            "bounces": col_i64(&r, "bounced"),
            "touchAndGo": false,
        })
    });
    let profile = (!track.is_empty()).then(|| {
        let t0 = track[0].t;
        let rows: Vec<Vec<Option<f64>>> = track
            .iter()
            .map(|p| vec![Some((p.t - t0).num_seconds() as f64), p.alt, p.ias, p.gs, p.vs])
            .collect();
        let step = rows.len().div_ceil(TRACK_MAX_ROWS).max(1);
        let mut thin: Vec<Vec<Option<f64>>> = rows.iter().step_by(step).cloned().collect();
        if (rows.len() - 1) % step != 0 {
            thin.extend(rows.last().cloned());
        }
        TlTrack { columns: ["t", "alt", "ias", "gs", "vs"].map(String::from).to_vec(), rows: thin }
    });

    let score_items: Vec<TlScoreItem> = sqlx::query(
        "SELECT rule_id, label, points_earned, points_max FROM flight_log_score_item
         WHERE flight_id = ?1 AND passed = 0 ORDER BY id",
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?
    .iter()
    .filter_map(|i| {
        let lost = col_f64(i, "points_max")? - col_f64(i, "points_earned").unwrap_or(0.0);
        (lost > 0.0).then(|| TlScoreItem {
            rule_id: i.try_get("rule_id").unwrap_or_default(),
            label: i.try_get("label").unwrap_or_default(),
            lost,
        })
    })
    .collect();

    let mut d = TlFlightDetail {
        source: Some(TlSource::Simfleet),
        flight: row_of(&r, warnings, effects.len() as i64),
        off_utc: off.map(|t| t.to_rfc3339()),
        on_utc: on.map(|t| t.to_rfc3339()),
        takeoff,
        landings: landing.into_iter().collect(),
        summary: mx.as_ref().and_then(|m| m.try_get::<Option<String>, _>("summary").ok().flatten()),
        events,
        effects,
        wear,
        report: crate::mx::store::flight_report(pool, flight_id).await.unwrap_or(None),
        track: profile,
        has_log: true,
        fuel_burn_kg: col_f64(&r, "fuel_used_kg"),
        score_items,
        ..Default::default()
    };
    localize_detail(&mut d);
    Ok(Some(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contra una COPIA de la base: `OWN_DB=<copia> OWN_REG=C-GORP cargo test
    /// --lib own_pages_real_copy -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn own_pages_real_copy() {
        let (Ok(path), Ok(reg)) = (std::env::var("OWN_DB"), std::env::var("OWN_REG")) else { return };
        let pool = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let page = own_flights(&pool, &reg, 20).await.unwrap().expect("vuelos");
        for f in page.flights.iter().take(5) {
            println!(
                "#{} {:?}→{:?} block {:?} air {:?} score {:?} pax {:?} warn {} eff {} landing {:?}",
                f.id, f.from, f.to, f.block_min, f.air_min, f.score, f.pax, f.warnings, f.effects, f.landing.as_ref().and_then(|l| l.vs_fpm)
            );
        }
        let first = page.flights.iter().find(|f| f.status == "closed").expect("uno cerrado");
        let d = own_flight_detail(&pool, &reg, first.id).await.unwrap().expect("detalle");
        let rep = d.report.as_ref().map(|r| (r.health_before, r.health_after, r.groups.len(), r.aged_most.len(), r.crossed.len()));
        println!(
            "detalle #{}: {} eventos ({} fases), {} efectos, {} desgastes, informe {:?}, traza {:?}, despegue {}, tomas {}, combustible {:?}, reglas {}",
            first.id,
            d.events.len(),
            d.events.iter().filter(|e| e.kind == "PHASE").count(),
            d.effects.len(),
            d.wear.len(),
            rep,
            d.track.as_ref().map(|t| t.rows.len()),
            d.takeoff.is_some(),
            d.landings.len(),
            d.fuel_burn_kg,
            d.score_items.len()
        );
        println!("despegue {:?}", d.takeoff);
        println!("toma {:?}", d.landings.first());
        assert!(d.track.as_ref().is_none_or(|t| t.rows.len() <= TRACK_MAX_ROWS + 1));

        let cabin = own_cabin(&pool, &reg).await;
        let plan = cabin.plan.expect("cabina");
        let worst = plan.seats.iter().filter_map(|s| s.worst_remaining_pct).fold(100.0f64, f64::min);
        println!(
            "cabina: {} filas {} · {} asientos ({} bloqueados) · peor {:.1}% · {} equipos · ej. {:?}",
            plan.rows,
            plan.letters,
            plan.seats.len(),
            plan.blocked_seats,
            worst,
            plan.fittings.len(),
            plan.fittings.first().map(|f| (&f.name, f.remaining_pct, &f.basis))
        );
        println!("asientos: {:?}", own_seat_info(&pool, &reg).await.unwrap().map(|i| (i.total, i.blocked.len())));
    }
}
