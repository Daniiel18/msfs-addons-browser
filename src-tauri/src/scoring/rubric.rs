//! Rubric — define las reglas que componen el score. Cada regla
//! tiene id estable, label visible, phase de pertenencia, peso
//! (`points_max`) y un evaluador (closure).
//!
//! La regla se evalúa contra `FlightContext`. El resultado es un
//! `ScoreItem` con puntos earned, passed/failed, severity y evidencia
//! (JSON con los valores observados que dispararon el verdict).
//!
//! ## Diseño de pesos
//!
//! (v3.6.1 fix I7 — feedback usuario) **Rubric saneado**. Reglas que
//! NO son responsabilidad del piloto fueron removidas:
//!
//! · `pushback_speed_safe` — quitada. GSX controla la velocidad del
//!   pushback; el piloto no puede limitarla. Penalizarlo era injusto.
//! · `cruise_altitude_held` — quitada. Pilotos pueden subir/bajar
//!   altitud de crucero por motivos legítimos (turbulencia, clima,
//!   instrucciones ATC). Mantener una altitud fija no es virtud.
//!
//! Total max ≈ **940 puntos**. Distribución actualizada:
//!
//! | Phase           | Pts | Reglas |
//! |-----------------|-----|--------|
//! | General         | 180 | metadata, distance, time |
//! | Pre-departure   | 100 | origen detectado, gate registrado |
//! | Taxi-out        |  80 | speed limit (≤ 30 kt) |
//! | Takeoff         |  80 | clean rotation (VS 500-3000 fpm) |
//! | Climb           | 120 | no overspeed > 280 kt bajo 10k ft |
//! | Descent         | 100 | rate ≤ 3000 fpm |
//! | Approach        | 100 | speed estable ≤ 200 kt a 5 nm dest |
//! | Landing         | 120 | smooth touchdown FPM |
//! | Taxi-in         |  60 | speed limit (≤ 30 kt) |
//! | Arrived         |  40 | vuelo completado |
//!
//! La grade derivada del % total:
//! · A ≥ 95%, B ≥ 85%, C ≥ 70%, D ≥ 50%, F < 50%

use serde_json::json;

use super::{FlightContext, ScoreItem, TrackSample};

#[allow(dead_code)] // Variants are kept for future use and for the rubric DSL even if not all are referenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    General,
    PreDeparture,
    Pushback,
    TaxiOut,
    Takeoff,
    Climb,
    Cruise,
    Descent,
    Approach,
    Landing,
    TaxiIn,
    Arrived,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::General => "general",
            Phase::PreDeparture => "pre_departure",
            Phase::Pushback => "pushback",
            Phase::TaxiOut => "taxi_out",
            Phase::Takeoff => "takeoff",
            Phase::Climb => "climb",
            Phase::Cruise => "cruise",
            Phase::Descent => "descent",
            Phase::Approach => "approach",
            Phase::Landing => "landing",
            Phase::TaxiIn => "taxi_in",
            Phase::Arrived => "arrived",
        }
    }
}

pub type Evaluator = fn(&FlightContext, &Rule) -> ScoreItem;

pub struct Rule {
    pub id: &'static str,
    pub label: &'static str,
    pub phase: Phase,
    pub points_max: i64,
    pub evaluator: Evaluator,
}

/// Lista plana de todas las reglas. El orden define el orden de
/// evaluación y el orden en que se persisten — la UI los agrupa por
/// `phase` para mostrar el breakdown.
pub static RULES: &[Rule] = &[
    // ===== GENERAL (180 pts) =====
    Rule {
        id: "va_metadata_present",
        label: "Datos de aerolínea presentes",
        phase: Phase::General,
        points_max: 60,
        evaluator: eval_va_metadata,
    },
    Rule {
        id: "distance_reasonable",
        label: "Distancia voladada > 50 nm",
        phase: Phase::General,
        points_max: 60,
        evaluator: eval_distance_reasonable,
    },
    Rule {
        id: "flight_time_reasonable",
        label: "Tiempo de vuelo > 15 min",
        phase: Phase::General,
        points_max: 60,
        evaluator: eval_flight_time_reasonable,
    },
    // ===== PRE-DEPARTURE (100 pts) =====
    Rule {
        id: "started_in_airport_area",
        label: "Origen detectado (ICAO)",
        phase: Phase::PreDeparture,
        points_max: 50,
        evaluator: eval_origin_known,
    },
    Rule {
        id: "departure_gate_recorded",
        label: "Gate de salida registrado",
        phase: Phase::PreDeparture,
        points_max: 50,
        evaluator: eval_departure_gate,
    },
    // ===== PUSHBACK =====
    // (v3.6.1 fix I7) Regla `pushback_speed_safe` REMOVIDA.
    // GSX controla la velocidad del pushback con su propio script —
    // el piloto no puede limitarla. Penalizarlo era un falso negativo.
    // Si en el futuro queremos rescatar algo de la phase pushback,
    // podríamos puntuar "freno de mano suelto durante pushback" o
    // "no aceleración intencional con motores running" pero requiere
    // capturar más simvars.
    // ===== TAXI-OUT (80 pts) =====
    Rule {
        id: "taxi_speed_below_30kt",
        label: "Taxi ≤ 30 kt",
        phase: Phase::TaxiOut,
        points_max: 80,
        evaluator: eval_taxi_out_speed,
    },
    // ===== TAKEOFF (80 pts) =====
    Rule {
        id: "clean_rotation",
        label: "Rotación limpia (VS 500-3000 fpm)",
        phase: Phase::Takeoff,
        points_max: 80,
        evaluator: eval_clean_rotation,
    },
    // ===== CLIMB (120 pts) =====
    Rule {
        id: "no_overspeed_below_10k",
        label: "Sin overspeed (≤ 250 kt) bajo 10,000 ft",
        phase: Phase::Climb,
        points_max: 120,
        evaluator: eval_no_overspeed_below_10k,
    },
    // ===== CRUISE =====
    // (v3.6.1 fix I7) Regla `cruise_altitude_held` REMOVIDA.
    // El piloto cambia altitud de crucero por razones LEGÍTIMAS:
    // turbulencia, clima, vectores ATC, optimización de fuel. Forzar
    // ±200 ft penalizaba step-climbs perfectamente normales.
    // Si en el futuro queremos puntuar crucero, sería mejor:
    //  · "VS ≈ 0 al menos N minutos seguidos" (sí mantuvo crucero
    //    estable en algún momento)
    //  · "GS ≥ M kt sostenido" (sí llegó a velocidad de crucero)
    // ===== DESCENT (100 pts) =====
    Rule {
        id: "descent_rate_reasonable",
        label: "Descenso ≤ 3000 fpm",
        phase: Phase::Descent,
        points_max: 100,
        evaluator: eval_descent_rate,
    },
    // ===== APPROACH (100 pts) =====
    Rule {
        id: "stable_approach",
        label: "Aproximación estable (≤ 200 kt a 5 nm)",
        phase: Phase::Approach,
        points_max: 100,
        evaluator: eval_stable_approach,
    },
    // ===== LANDING (120 pts) =====
    Rule {
        id: "smooth_landing",
        label: "Aterrizaje suave",
        phase: Phase::Landing,
        points_max: 120,
        evaluator: eval_smooth_landing,
    },
    // ===== TAXI-IN (60 pts) =====
    Rule {
        id: "taxi_in_speed_safe",
        label: "Taxi de llegada ≤ 30 kt",
        phase: Phase::TaxiIn,
        points_max: 60,
        evaluator: eval_taxi_in_speed,
    },
    // ===== ARRIVED (40 pts) =====
    Rule {
        id: "block_in_reached",
        label: "Vuelo completado en destino",
        phase: Phase::Arrived,
        points_max: 40,
        evaluator: eval_block_in_reached,
    },
];

// =============================================================================
// Helpers
// =============================================================================

/// Construye un `ScoreItem` con campos derivados.
fn pass(rule: &Rule, evidence: serde_json::Value) -> ScoreItem {
    ScoreItem {
        phase: rule.phase.as_str().to_string(),
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        points_earned: rule.points_max,
        points_max: rule.points_max,
        passed: true,
        severity: "info".to_string(),
        evidence,
    }
}

fn fail(rule: &Rule, severity: &str, earned: i64, evidence: serde_json::Value) -> ScoreItem {
    ScoreItem {
        phase: rule.phase.as_str().to_string(),
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        points_earned: earned.clamp(0, rule.points_max),
        points_max: rule.points_max,
        passed: false,
        severity: severity.to_string(),
        evidence,
    }
}

fn partial(rule: &Rule, earned: i64, evidence: serde_json::Value) -> ScoreItem {
    let passed = earned == rule.points_max;
    let severity = if passed {
        "info"
    } else if earned >= rule.points_max / 2 {
        "warn"
    } else {
        "fail"
    };
    ScoreItem {
        phase: rule.phase.as_str().to_string(),
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        points_earned: earned.clamp(0, rule.points_max),
        points_max: rule.points_max,
        passed,
        severity: severity.to_string(),
        evidence,
    }
}

/// Devuelve `(max_gs, max_alt)` entre las muestras dadas.
fn track_extrema(samples: &[&TrackSample]) -> (Option<i64>, Option<i64>) {
    let mut max_gs: Option<i64> = None;
    let mut max_alt: Option<i64> = None;
    for s in samples {
        if let Some(gs) = s.gs_kt {
            max_gs = Some(max_gs.map_or(gs, |m| m.max(gs)));
        }
        if let Some(alt) = s.alt_ft {
            max_alt = Some(max_alt.map_or(alt, |m| m.max(alt)));
        }
    }
    (max_gs, max_alt)
}

// =============================================================================
// Evaluadores
// =============================================================================

fn eval_va_metadata(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let has_fn = ctx.flight_number.is_some();
    let has_cs = ctx.callsign.is_some();
    let has_icao = ctx.airline_icao.is_some();
    let count = [has_fn, has_cs, has_icao].iter().filter(|v| **v).count() as i64;
    let earned = (rule.points_max * count) / 3;
    let evidence = json!({
        "flight_number": ctx.flight_number,
        "callsign": ctx.callsign,
        "airline_icao": ctx.airline_icao,
    });
    partial(rule, earned, evidence)
}

fn eval_distance_reasonable(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let d = ctx.distance_nm.unwrap_or(0.0);
    let evidence = json!({ "distance_nm": d });
    if d >= 50.0 {
        pass(rule, evidence)
    } else if d >= 20.0 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_flight_time_reasonable(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let s = ctx.flight_time_s.unwrap_or(0);
    let evidence = json!({ "flight_time_s": s, "minutes": s / 60 });
    if s >= 900 {
        // 15 min
        pass(rule, evidence)
    } else if s >= 300 {
        // 5 min
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_origin_known(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let evidence = json!({ "origin_icao": ctx.origin_icao });
    if ctx.origin_icao.is_some() {
        pass(rule, evidence)
    } else {
        fail(rule, "warn", 0, evidence)
    }
}

fn eval_departure_gate(_ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Para VAS-ACARS imports tendremos el gate del summary.
    // Para SimConnect lo tendremos si la Facility Data API resolvió.
    // No mantenemos el departure_gate en FlightContext porque no lo
    // cargamos — sería trivial agregar. Por ahora, evaluamos siempre
    // como passed si el ctx existe. Marca TODO de mejora.
    let evidence = json!({ "departure_gate": "not_loaded_into_ctx_yet" });
    partial(rule, rule.points_max / 2, evidence)
}

/// (v3.6.1) Regla removida del rubric pero conservada como referencia
/// por si la rescatamos con otra señal de input.
#[allow(dead_code)]
fn eval_pushback_speed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("pushback");
    if samples.is_empty() {
        // Sin phase pushback registrada — no penalizar fuerte, dar mid.
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "no_pushback_phase" }),
        );
    }
    let (max_gs, _) = track_extrema(&samples);
    let max_gs_v = max_gs.unwrap_or(0);
    let evidence = json!({ "max_gs_kt": max_gs_v });
    match max_gs_v {
        n if n <= 5 => pass(rule, evidence),
        n if n <= 10 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_taxi_out_speed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_out");
    if samples.is_empty() {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "no_taxi_phase" }),
        );
    }
    let (max_gs, _) = track_extrema(&samples);
    let max_gs_v = max_gs.unwrap_or(0);
    let evidence = json!({ "max_gs_kt": max_gs_v });
    match max_gs_v {
        n if n <= 30 => pass(rule, evidence),
        n if n <= 40 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 50 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_clean_rotation(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Buscamos la primera transición ground→air estimada via alt_ft.
    // El primer sample con alt > 50 ft sobre el origen marca el lift-off.
    // Tomamos VS = Δalt / Δt en una ventana de 30s después de lift-off.
    let mut lift_idx: Option<usize> = None;
    let baseline_alt: Option<i64> = ctx.track.first().and_then(|s| s.alt_ft);
    let baseline = baseline_alt.unwrap_or(0);
    for (i, s) in ctx.track.iter().enumerate() {
        if let Some(alt) = s.alt_ft {
            if alt > baseline + 50 {
                lift_idx = Some(i);
                break;
            }
        }
    }
    let Some(li) = lift_idx else {
        return fail(
            rule,
            "warn",
            0,
            json!({ "reason": "no_liftoff_detected" }),
        );
    };
    let to_idx = (li + 6).min(ctx.track.len() - 1); // 30s assuming 5s samples
    let s0 = &ctx.track[li];
    let s1 = &ctx.track[to_idx];
    let (Some(a0), Some(a1)) = (s0.alt_ft, s1.alt_ft) else {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "alt_missing" }),
        );
    };
    let dt = ts_diff_seconds(&s0.ts, &s1.ts).unwrap_or(0.0);
    if dt < 1.0 {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "dt_too_small" }),
        );
    }
    let vs_fpm = ((a1 - a0) as f64 / (dt / 60.0)).round() as i64;
    let evidence = json!({ "vs_fpm": vs_fpm });
    match vs_fpm {
        v if (500..=3000).contains(&v) => pass(rule, evidence),
        v if (300..500).contains(&v) || (3000..=4500).contains(&v) => {
            partial(rule, (rule.points_max * 2) / 3, evidence)
        }
        _ => fail(rule, "fail", rule.points_max / 4, evidence),
    }
}

fn eval_no_overspeed_below_10k(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Reglas FAA: ≤ 250 KIAS bajo 10,000 ft AGL. Como proxy usamos GS
    // (no tenemos IAS exacto en el track). El threshold lo subimos a
    // 280 kt para acomodar tailwind razonable (la nota es 'no más de
    // 30 kt de margen sobre IAS').
    let mut over: Vec<(i64, i64)> = Vec::new();
    for s in &ctx.track {
        if let (Some(alt), Some(gs)) = (s.alt_ft, s.gs_kt) {
            if alt < 10000 && gs > 280 {
                over.push((alt, gs));
            }
        }
    }
    let count = over.len() as i64;
    let evidence = json!({ "over_samples": count, "examples": over.iter().take(3).collect::<Vec<_>>() });
    match count {
        0 => pass(rule, evidence),
        1..=3 => partial(rule, (rule.points_max * 3) / 4, evidence),
        4..=10 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

/// (v3.6.1) Regla removida — pilotos pueden cambiar altitud
/// legítimamente. Conservada por si se rescata con otra lógica.
#[allow(dead_code)]
fn eval_cruise_alt_held(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("cruise");
    if samples.is_empty() {
        // Fallback: sin phase explícita, asume que cruise = ventana
        // entre 30% y 70% del vuelo en altitud > 10000 ft.
        let high_alt: Vec<&TrackSample> = ctx
            .track
            .iter()
            .filter(|s| s.alt_ft.unwrap_or(0) > 10000)
            .collect();
        if high_alt.len() < 6 {
            return partial(
                rule,
                rule.points_max / 2,
                json!({ "reason": "no_cruise_data" }),
            );
        }
        return cruise_alt_stability(&high_alt, rule);
    }
    cruise_alt_stability(&samples, rule)
}

#[allow(dead_code)]
fn cruise_alt_stability(samples: &[&TrackSample], rule: &Rule) -> ScoreItem {
    let alts: Vec<i64> = samples.iter().filter_map(|s| s.alt_ft).collect();
    if alts.is_empty() {
        return partial(rule, rule.points_max / 2, json!({ "reason": "no_alt" }));
    }
    let target = *alts.iter().max().unwrap();
    let within = alts.iter().filter(|a| (target - **a).abs() <= 200).count() as i64;
    let total = alts.len() as i64;
    let pct = (within as f32 / total as f32) * 100.0;
    let evidence = json!({ "target_alt": target, "within_200ft_pct": pct as i32 });
    let earned = (rule.points_max * within) / total.max(1);
    partial(rule, earned, evidence)
}

fn eval_descent_rate(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("descent");
    let pool: Vec<&TrackSample> = if samples.is_empty() {
        // Fallback: ventana post-cruise — desde la altitud máxima
        // hasta el último sample.
        let max_alt = ctx
            .track
            .iter()
            .filter_map(|s| s.alt_ft)
            .max()
            .unwrap_or(0);
        ctx.track
            .iter()
            .skip_while(|s| s.alt_ft.unwrap_or(0) < max_alt)
            .collect()
    } else {
        samples
    };
    if pool.len() < 3 {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "no_descent_data" }),
        );
    }
    let mut worst_vs: i64 = 0;
    for win in pool.windows(2) {
        let (a, b) = (win[0], win[1]);
        let (Some(a_alt), Some(b_alt)) = (a.alt_ft, b.alt_ft) else {
            continue;
        };
        let dt = ts_diff_seconds(&a.ts, &b.ts).unwrap_or(0.0);
        if dt < 1.0 {
            continue;
        }
        let vs = ((b_alt - a_alt) as f64 / (dt / 60.0)) as i64;
        if vs < worst_vs {
            worst_vs = vs;
        }
    }
    let evidence = json!({ "worst_vs_fpm": worst_vs });
    match worst_vs {
        v if v >= -2500 => pass(rule, evidence),
        v if v >= -3500 => partial(rule, (rule.points_max * 2) / 3, evidence),
        v if v >= -4500 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_stable_approach(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Definir "approach" como las muestras dentro de 5 nm del
    // destination_icao (o el último sample como proxy de destino).
    let (dlat, dlon) = match (ctx.destination_lat, ctx.destination_lon) {
        (Some(la), Some(lo)) => (la, lo),
        _ => match ctx.track.last() {
            Some(s) => (s.lat, s.lon),
            None => return fail(rule, "warn", 0, json!({ "reason": "no_dest" })),
        },
    };
    let close: Vec<&TrackSample> = ctx
        .track
        .iter()
        .filter(|s| crate::flight_log::haversine_nm(s.lat, s.lon, dlat, dlon) <= 5.0)
        .collect();
    if close.len() < 3 {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "insufficient_approach_samples" }),
        );
    }
    let max_gs = close.iter().filter_map(|s| s.gs_kt).max().unwrap_or(0);
    let evidence = json!({ "max_gs_within_5nm": max_gs });
    match max_gs {
        n if n <= 180 => pass(rule, evidence),
        n if n <= 220 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 260 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_smooth_landing(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let Some(fpm) = ctx.landing_fpm else {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "no_landing_fpm" }),
        );
    };
    let evidence = json!({ "landing_fpm": fpm });
    // FPM negativo = descenso. Más negativo = más duro.
    match fpm {
        f if f > -200 => pass(rule, evidence), // mariposa / muy suave
        f if f > -400 => partial(rule, (rule.points_max * 4) / 5, evidence),
        f if f > -600 => partial(rule, (rule.points_max * 3) / 5, evidence),
        f if f > -800 => partial(rule, (rule.points_max * 2) / 5, evidence),
        f if f > -1000 => partial(rule, rule.points_max / 5, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_taxi_in_speed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_in");
    if samples.is_empty() {
        return partial(
            rule,
            rule.points_max / 2,
            json!({ "reason": "no_taxi_in_phase" }),
        );
    }
    let (max_gs, _) = track_extrema(&samples);
    let v = max_gs.unwrap_or(0);
    let evidence = json!({ "max_gs_kt": v });
    match v {
        n if n <= 30 => pass(rule, evidence),
        n if n <= 40 => partial(rule, (rule.points_max * 2) / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_block_in_reached(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let has_end = ctx.ended_at.is_some();
    let has_dest = ctx.destination_icao.is_some();
    let has_gate = ctx.arrival_gate.is_some();
    let evidence = json!({
        "ended_at": ctx.ended_at,
        "destination_icao": ctx.destination_icao,
        "arrival_gate": ctx.arrival_gate,
    });
    let score = (if has_end { 1 } else { 0 })
        + (if has_dest { 1 } else { 0 })
        + (if has_gate { 1 } else { 0 });
    let earned = (rule.points_max * score) / 3;
    partial(rule, earned, evidence)
}

/// Diferencia en segundos entre dos timestamps ISO 8601 UTC.
fn ts_diff_seconds(a: &str, b: &str) -> Option<f64> {
    let ta = chrono::DateTime::parse_from_rfc3339(a).ok()?;
    let tb = chrono::DateTime::parse_from_rfc3339(b).ok()?;
    Some((tb - ta).num_seconds() as f64)
}
