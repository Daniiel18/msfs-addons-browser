//! (v4.19.0) **Log por vuelo** — un archivo de texto único por cada
//! vuelo terminado, con TODAS las métricas crudas (track sample a
//! sample: luces, flaps, gear, velocidades, fases…) más el resultado
//! de la Flight Evaluation, para auditar si el evaluador está haciendo
//! bien su trabajo (pedido del usuario).
//!
//! Nombre del archivo: `{ORIGEN}-{DESTINO}-{CALLSIGN}.log` (ejemplo del
//! usuario: `MDSD-KFLL-JBU7877.log`). Si el callsign no existe se usa
//! flight_number y como último recurso `FLIGHT<id>`. Si ya existe un
//! archivo con ese nombre de OTRO vuelo (misma ruta + callsign volada
//! dos veces), se sufija `-<id>` para mantener un log único por vuelo;
//! re-evaluar el MISMO vuelo sobreescribe su propio archivo.
//!
//! Ubicación: `<data>/logs/flights/` — junto a los `simfleet.*.log`.
//!
//! Solo LEE de la DB (load_context) y escribe el archivo: no toca el
//! flujo de finish_flight ni la captura de FPM/touchdown.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::scoring::{load_context, FlightContext, ScoreReport, TrackSample};

/// Marca en la primera línea del archivo — sirve para reconocer si un
/// archivo existente pertenece a este mismo vuelo (→ sobreescribir) o a
/// otro con el mismo nombre (→ sufijar).
fn header_marker(flight_id: i64) -> String {
    format!("# SimFleet Flight Report — flight #{flight_id}")
}

/// Sanitiza un componente del nombre: solo ASCII alfanumérico, en
/// mayúsculas. Evita `/ \ : * ? " < > |` y espacios.
fn clean(part: &str) -> String {
    part.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn base_name(ctx: &FlightContext) -> String {
    let orig = ctx
        .origin_icao
        .as_deref()
        .map(clean)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UNK".into());
    let dest = ctx
        .destination_icao
        .as_deref()
        .map(clean)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UNK".into());
    let ident = ctx
        .callsign
        .as_deref()
        .map(clean)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            ctx.flight_number
                .as_deref()
                .map(clean)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("FLIGHT{}", ctx.flight_id));
    format!("{orig}-{dest}-{ident}")
}

/// Resuelve la ruta final del archivo manejando colisiones de nombre
/// entre vuelos distintos.
fn resolve_path(dir: &Path, ctx: &FlightContext) -> PathBuf {
    let base = base_name(ctx);
    let candidate = dir.join(format!("{base}.log"));
    if !candidate.exists() {
        return candidate;
    }
    // ¿El archivo existente es de ESTE vuelo? (re-score → sobreescribir).
    let marker = header_marker(ctx.flight_id);
    if let Ok(text) = std::fs::read_to_string(&candidate) {
        if text.lines().next() == Some(marker.as_str()) {
            return candidate;
        }
    }
    dir.join(format!("{base}-{}.log", ctx.flight_id))
}

fn opt_i64(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".into())
}

fn opt_f1(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into())
}

fn opt_bool(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    }
}

fn sample_line(s: &TrackSample) -> String {
    format!(
        "{ts}  {phase:<16} {lat:>10.5} {lon:>11.5}  alt={alt:>6} gs={gs:>4} ias={ias:>4} vs={vs:>6}  pitch={pitch:>5} bank={bank:>6} g={g:>4}  flaps={flaps:>3}% gear={gear} spoil={spoil:>3}%  nav={nav} bcn={bcn} taxi={taxi} land={land} strb={strb}  pbrk={pbrk} xpdr={xpdr} n1={n1} stall={stall}",
        ts = s.ts,
        phase = s.scoring_phase.as_deref().unwrap_or("-"),
        lat = s.lat,
        lon = s.lon,
        alt = opt_i64(s.alt_ft),
        gs = opt_i64(s.gs_kt),
        ias = opt_i64(s.ias_kt),
        vs = opt_i64(s.vs_fpm),
        pitch = opt_f1(s.pitch_deg),
        bank = opt_f1(s.bank_deg),
        g = opt_f1(s.g_force),
        flaps = opt_i64(s.flaps_pct),
        gear = match s.gear_down {
            Some(true) => "DN",
            Some(false) => "UP",
            None => "-",
        },
        spoil = opt_i64(s.spoilers_pct),
        nav = opt_bool(s.light_nav),
        bcn = opt_bool(s.light_beacon),
        taxi = opt_bool(s.light_taxi),
        land = opt_bool(s.light_landing),
        strb = opt_bool(s.light_strobe),
        pbrk = opt_bool(s.parking_brake),
        xpdr = opt_i64(s.transponder_code),
        n1 = opt_f1(s.eng_n1_max),
        stall = opt_bool(s.stall_warning),
    )
}

/// Escribe el reporte completo del vuelo. Devuelve la ruta escrita.
pub async fn write_flight_report(
    pool: &sqlx::SqlitePool,
    flight_id: i64,
    report: &ScoreReport,
    data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let ctx = load_context(pool, flight_id).await?;

    let dir = data_dir.join("logs").join("flights");
    std::fs::create_dir_all(&dir)?;
    let path = resolve_path(&dir, &ctx);

    let mut out = String::with_capacity(64 * 1024 + ctx.track.len() * 220);
    let _ = writeln!(out, "{}", header_marker(flight_id));
    let _ = writeln!(
        out,
        "# Generado: {}",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );
    let _ = writeln!(out, "{}", "=".repeat(78));
    let _ = writeln!(
        out,
        "Ruta:        {} -> {}",
        ctx.origin_icao.as_deref().unwrap_or("?"),
        ctx.destination_icao.as_deref().unwrap_or("?"),
    );
    let _ = writeln!(
        out,
        "Vuelo:       callsign={} flight_number={} airline={}",
        ctx.callsign.as_deref().unwrap_or("-"),
        ctx.flight_number.as_deref().unwrap_or("-"),
        ctx.airline_icao.as_deref().unwrap_or("-"),
    );
    let _ = writeln!(
        out,
        "Aeronave:    {} ({})",
        ctx.aircraft_atc_type.as_deref().unwrap_or("-"),
        ctx.aircraft_title.as_deref().unwrap_or("-"),
    );
    let _ = writeln!(
        out,
        "Tiempos:     started={} ended={} flight_time_s={}",
        ctx.started_at,
        ctx.ended_at.as_deref().unwrap_or("-"),
        opt_i64(ctx.flight_time_s),
    );
    let _ = writeln!(
        out,
        "Métricas:    distance_nm={} max_alt_ft={} max_gs_kt={} landing_fpm={}",
        ctx.distance_nm
            .map(|d| format!("{d:.0}"))
            .unwrap_or_else(|| "-".into()),
        opt_i64(ctx.max_altitude_ft),
        opt_i64(ctx.max_ground_speed_kt),
        opt_i64(ctx.landing_fpm),
    );
    let _ = writeln!(
        out,
        "Estado:      {} · arrival_gate={}",
        ctx.status,
        ctx.arrival_gate.as_deref().unwrap_or("-"),
    );

    // ---- Fases ------------------------------------------------------
    let _ = writeln!(out, "\n## FASES ({} registradas)", ctx.phases.len());
    for p in &ctx.phases {
        let _ = writeln!(
            out,
            "  {:<16} {} -> {}",
            p.phase,
            p.entered_at,
            p.exited_at.as_deref().unwrap_or("(abierta)"),
        );
    }

    // ---- Flight Evaluation ------------------------------------------
    let _ = writeln!(
        out,
        "\n## FLIGHT EVALUATION — {}/{} ({:.0}%) grade {}",
        report.total, report.max, report.percentage, report.grade
    );
    let mut last_phase = String::new();
    for item in &report.items {
        if item.phase != last_phase {
            let _ = writeln!(out, "  [{}]", item.phase);
            last_phase = item.phase.clone();
        }
        let mark = match item.status.as_str() {
            "done" => "PASS",
            "missed" => "FAIL",
            _ => "N/A ",
        };
        let _ = writeln!(
            out,
            "    {mark} {:<38} evidence={}",
            item.rule_id, item.evidence
        );
    }

    // ---- Track sample a sample --------------------------------------
    let _ = writeln!(
        out,
        "\n## TRACK ({} samples — luces: 1=on 0=off '-'=sin dato)",
        ctx.track.len()
    );
    for s in &ctx.track {
        let _ = writeln!(out, "{}", sample_line(s));
    }

    std::fs::write(&path, out)?;
    Ok(path)
}
