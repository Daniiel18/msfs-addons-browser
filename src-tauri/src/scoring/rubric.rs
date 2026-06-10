//! Rubric — define las reglas que componen el score. Cada regla
//! tiene id estable, label visible, phase de pertenencia, peso
//! (`points_max`) y un evaluador (closure).
//!
//! La regla se evalúa contra `FlightContext`. El resultado es un
//! `ScoreItem` con puntos earned, passed/failed, severity y evidencia
//! (JSON con los valores observados que dispararon el verdict).
//!
//! ## Diseño (v3.7.0 — Phase O)
//!
//! El rubric se reescribió completo para alinearse con la rúbrica de
//! puntuación de las plataformas VAS (Virtual Airline System) — el
//! formato que el usuario recibe en su VA tras subir un vuelo via
//! ACARS. La idea es que **lo que SimFleet computa internamente
//! coincida con lo que la aerolínea verá** en su propio web.
//!
//! 13 phases, ~35 reglas, total max ≈ 2115 puntos:
//!
//! | Phase            | Pts  | Reglas |
//! |------------------|------|--------|
//! | General          | 590  | pitch±20, bank30, g_force2, overspeed, stall, max_alt |
//! | Pre-Departure    | 300  | parking_brake, engines_off, flaps_up, spoilers_up, ample_time |
//! | Pushback         | 160  | nav_light, beacon_light, doors, engines_off_at_start |
//! | Taxi-Out         |  80  | speed≤30, flaps_set, transponder, taxi_light |
//! | Take-Off         |  55  | landing_light, strobe, pitch≤15 |
//! | Take-Off Accel   | 105  | gear_up, ias≤250 |
//! | Initial Climb    |  40  | flaps_retracted, landing_light_off_10k |
//! | Cruise           | 120  | gear_up, landing_light_off, flaps_up, doors_closed |
//! | Initial Approach |  85  | pitch≥-15, flaps_deployed, ias≤200, landing_light |
//! | Final Approach   | 260  | pitch≥-15, flaps, gear_down, spoilers_armed, ias≤250 |
//! | Landing          |  50  | vs≤1000fpm |
//! | Taxi-In          |  80  | landing_light_off, speed≤30, flaps_retracted, taxi_light |
//! | Arrived          | 190  | parking_brake, flaps_up, spoilers_up, engines_off |
//!
//! ## Skipped rules (v3.7.0)
//!
//! Para vuelos importados de VAS-ACARS los campos como `light_*`,
//! `pitch_deg`, `bank_deg`, etc. quedan NULL — el .bin del ACARS no
//! los exporta. Los evaluadores detectan la ausencia y devuelven
//! `severity = "skipped"` con `points_max = 0` (no `rule.points_max`).
//!
//! El loop en `score_flight` ahora suma `item.points_max` (no el del
//! rule), así las skipped no inflan el denominador y el % de score
//! sigue siendo significativo aún con datos parciales.
//!
//! Para vuelos SimConnect, **todos** los campos están populados — el
//! watcher captura los simvars VAS-aligned (LIGHT NAV / TAXI / etc.)
//! por sample y los persiste en `flight_log_track`.
//!
//! ## Grade derivada del %:
//! · A ≥ 95%, B ≥ 85%, C ≥ 70%, D ≥ 50%, F < 50%

use serde_json::json;

use super::{FlightContext, ScoreItem, TrackSample};

#[allow(dead_code)] // Variants kept for completeness even if not all referenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    General,
    PreDeparture,
    Pushback,
    TaxiOut,
    Takeoff,
    TakeoffAccel,
    InitialClimb,
    Climb,
    Cruise,
    Descent,
    InitialApproach,
    FinalApproach,
    Approach, // legacy alias — kept for backwards-compat with old score_items
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
            Phase::TakeoffAccel => "takeoff_accel",
            Phase::InitialClimb => "initial_climb",
            Phase::Climb => "climb",
            Phase::Cruise => "cruise",
            Phase::Descent => "descent",
            Phase::InitialApproach => "initial_approach",
            Phase::FinalApproach => "final_approach",
            Phase::Approach => "approach",
            Phase::Landing => "landing",
            Phase::TaxiIn => "taxi_in",
            Phase::Arrived => "arrived",
        }
    }
}

/// (v4.1.0) Orden cronológico canónico de una fase (string) para la
/// presentación tipo checklist. Las fases persistidas se leían
/// `ORDER BY phase` (alfabético) → "approach" salía arriba y "taxi"
/// abajo. Esto mapea cada nombre de fase a su índice real de vuelo;
/// `general` (límites de todo el vuelo) va al final. Desconocidas → 99.
pub fn phase_order(phase: &str) -> u8 {
    match phase {
        "pre_departure" | "preflight" => 0,
        "pushback" => 1,
        "taxi_out" => 2,
        "takeoff" => 3,
        "takeoff_accel" => 4,
        "initial_climb" => 5,
        "climb" | "climbing" => 6,
        "cruise" => 7,
        "descent" => 8,
        "initial_approach" => 9,
        "final_approach" | "approach" => 10,
        "landing" => 11,
        "taxi_in" | "landed_rollout" => 12,
        "arrived" | "parking" | "deboarding" => 13,
        "general" => 14,
        _ => 99,
    }
}

/// (v4.1.0) Índice de una regla dentro de `RULES` — para ordenar los
/// ítems DENTRO de una fase igual que el flujo del checklist. None si
/// el rule_id no existe (regla retirada en una versión posterior).
pub fn rule_index(rule_id: &str) -> Option<usize> {
    RULES.iter().position(|r| r.id == rule_id)
}

/// (v4.1.0) Categoría de aeronave — decide qué ítems del checklist
/// aplican. Detectada del título/tipo en `scoring::aircraft_category`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Airliner,
    Turboprop,
    Ga,
}

/// (v4.1.0) ¿La regla aplica a esta categoría de avión? Si no, el
/// scoring la marca "na" (— no aplica) y NO penaliza el % de checklist.
///
/// Conservador: la mayoría de reglas son data-driven y se auto-resuelven
/// (un avión sin spoilers reporta spoilers=0 → pasa "spoilers no
/// desplegados"; sin fase pushback → esas reglas quedan vacías → skip).
/// Solo excluimos para GA (avión ligero) los ítems que claramente no le
/// corresponden y que SÍ producirían un "✗" injusto: transponder IFR
/// (suele volar VFR), flaps de despegue (muchos despegan flaps 0) y el
/// grupo de pushback. Airliner/Turboprop = todas aplican.
pub fn rule_applies_to(rule_id: &str, cat: Category) -> bool {
    match cat {
        Category::Airliner | Category::Turboprop => true,
        Category::Ga => !matches!(
            rule_id,
            "taxi_out_transponder"
                | "taxi_out_flaps_set"
                | "pushback_nav_light"
                | "pushback_beacon_light"
                | "pushback_doors_closed"
        ),
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
    // ============================================================
    // GENERAL (590 pts) — Aircraft state durante TODO el vuelo
    // ============================================================
    Rule {
        id: "general_pitch_up_limit",
        label: "Pitch up never more than +20°",
        phase: Phase::General,
        points_max: 70,
        evaluator: eval_pitch_up_limit,
    },
    Rule {
        id: "general_pitch_down_limit",
        label: "Pitch down never less than -20°",
        phase: Phase::General,
        points_max: 70,
        evaluator: eval_pitch_down_limit,
    },
    Rule {
        id: "general_bank_limit",
        label: "Bank angle never more than 30°",
        phase: Phase::General,
        points_max: 100,
        evaluator: eval_bank_limit,
    },
    Rule {
        id: "general_g_force_limit",
        label: "G-Force never more than 2G",
        phase: Phase::General,
        points_max: 100,
        evaluator: eval_g_force_limit,
    },
    Rule {
        id: "general_no_overspeed",
        label: "Overspeed never indicated",
        phase: Phase::General,
        points_max: 50,
        evaluator: eval_no_overspeed,
    },
    Rule {
        id: "general_no_stall",
        label: "Stall never indicated",
        phase: Phase::General,
        points_max: 100,
        evaluator: eval_no_stall,
    },
    Rule {
        id: "general_max_service_ceiling",
        label: "Max service ceiling not exceeded",
        phase: Phase::General,
        points_max: 100,
        evaluator: eval_max_service_ceiling,
    },
    // ============================================================
    // PRE-DEPARTURE (300 pts) — Antes del pushback
    // ============================================================
    Rule {
        id: "predep_parking_brake",
        label: "Parking brake applied",
        phase: Phase::PreDeparture,
        points_max: 70,
        evaluator: eval_predep_parking_brake,
    },
    Rule {
        id: "predep_engines_off",
        label: "Motores apagados al inicio (arranque en frío)",
        phase: Phase::PreDeparture,
        points_max: 100,
        evaluator: eval_predep_engines_off,
    },
    Rule {
        id: "predep_flaps_up",
        label: "Flaps in up position",
        phase: Phase::PreDeparture,
        points_max: 20,
        evaluator: eval_predep_flaps_up,
    },
    Rule {
        id: "predep_spoilers_stowed",
        label: "Spoilers not deployed",
        phase: Phase::PreDeparture,
        points_max: 40,
        evaluator: eval_predep_spoilers_stowed,
    },
    Rule {
        id: "predep_ample_time",
        label: "Ample pre-departure time",
        phase: Phase::PreDeparture,
        points_max: 70,
        evaluator: eval_predep_ample_time,
    },
    // ============================================================
    // PUSHBACK (160 pts) — Durante el pushback
    // ============================================================
    Rule {
        id: "pushback_nav_light",
        label: "Navigation lights used",
        phase: Phase::Pushback,
        points_max: 20,
        evaluator: eval_pushback_nav_light,
    },
    Rule {
        id: "pushback_beacon_light",
        label: "Beacon light used",
        phase: Phase::Pushback,
        points_max: 50,
        evaluator: eval_pushback_beacon_light,
    },
    // (v4.15.0 #3) Regla "Pushback started with engines off" ELIMINADA:
    // en la aviación real el remolque comienza con los motores apagados y
    // se van encendiendo DURANTE el pushback — es un procedimiento normal,
    // no una falta. Penalizarlo era incorrecto.
    Rule {
        id: "pushback_doors_closed",
        label: "Doors closed",
        phase: Phase::Pushback,
        points_max: 10,
        evaluator: eval_pushback_doors_closed,
    },
    // ============================================================
    // TAXI-OUT (80 pts) — Rodaje a la pista
    // ============================================================
    Rule {
        id: "taxi_out_speed",
        label: "Taxi speed never more than 30 kt",
        phase: Phase::TaxiOut,
        points_max: 20,
        evaluator: eval_taxi_out_speed,
    },
    Rule {
        id: "taxi_out_flaps_set",
        label: "Flaps set for takeoff",
        phase: Phase::TaxiOut,
        points_max: 20,
        evaluator: eval_taxi_out_flaps_set,
    },
    Rule {
        id: "taxi_out_transponder",
        label: "Transponder set to IFR code",
        phase: Phase::TaxiOut,
        points_max: 20,
        evaluator: eval_taxi_out_transponder,
    },
    Rule {
        id: "taxi_out_taxi_light",
        label: "Taxi light used",
        phase: Phase::TaxiOut,
        points_max: 10,
        evaluator: eval_taxi_out_taxi_light,
    },
    Rule {
        id: "taxi_out_nav_light",
        label: "Navigation lights used",
        phase: Phase::TaxiOut,
        points_max: 10,
        evaluator: eval_taxi_out_nav_light,
    },
    // ============================================================
    // TAKE-OFF (55 pts) — Roll de despegue + rotación
    // ============================================================
    Rule {
        id: "takeoff_landing_light",
        label: "Landing lights used",
        phase: Phase::Takeoff,
        points_max: 10,
        evaluator: eval_takeoff_landing_light,
    },
    Rule {
        id: "takeoff_strobe_light",
        label: "Strobe lights used",
        phase: Phase::Takeoff,
        points_max: 20,
        evaluator: eval_takeoff_strobe_light,
    },
    Rule {
        id: "takeoff_pitch_limit",
        label: "Pitch never more than 15°",
        phase: Phase::Takeoff,
        points_max: 25,
        evaluator: eval_takeoff_pitch_limit,
    },
    // ============================================================
    // TAKE-OFF ACCELERATION (105 pts) — 0..1000 ft AGL
    // ============================================================
    Rule {
        id: "takeoff_accel_gear_up",
        label: "Landing gear retracted",
        phase: Phase::TakeoffAccel,
        points_max: 75,
        evaluator: eval_takeoff_accel_gear_up,
    },
    Rule {
        id: "takeoff_accel_ias_limit",
        label: "Indicated airspeed ≤ 250 kt",
        phase: Phase::TakeoffAccel,
        points_max: 30,
        evaluator: eval_takeoff_accel_ias_limit,
    },
    // ============================================================
    // INITIAL CLIMB (40 pts) — 1000..10000 ft AGL
    // ============================================================
    Rule {
        id: "initial_climb_flaps_retracted",
        label: "Flaps fully retracted",
        phase: Phase::InitialClimb,
        points_max: 30,
        evaluator: eval_initial_climb_flaps_retracted,
    },
    Rule {
        id: "initial_climb_landing_light_off",
        label: "Landing lights off before 10,000 ft",
        phase: Phase::InitialClimb,
        points_max: 10,
        evaluator: eval_initial_climb_landing_light_off,
    },
    // ============================================================
    // CRUISE (120 pts) — Above 10,000 ft
    // ============================================================
    Rule {
        id: "cruise_gear_up",
        label: "Landing gear not deployed",
        phase: Phase::Cruise,
        points_max: 40,
        evaluator: eval_cruise_gear_up,
    },
    Rule {
        id: "cruise_landing_light_off",
        label: "Landing lights not used",
        phase: Phase::Cruise,
        points_max: 20,
        evaluator: eval_cruise_landing_light_off,
    },
    Rule {
        id: "cruise_flaps_up",
        label: "Flaps not deployed",
        phase: Phase::Cruise,
        points_max: 50,
        evaluator: eval_cruise_flaps_up,
    },
    Rule {
        id: "cruise_doors_closed",
        label: "Doors closed",
        phase: Phase::Cruise,
        points_max: 10,
        evaluator: eval_cruise_doors_closed,
    },
    // ============================================================
    // INITIAL APPROACH (85 pts) — 1800..10000 ft, descending
    // ============================================================
    Rule {
        id: "initial_approach_pitch_limit",
        label: "Pitch never less than -15°",
        phase: Phase::InitialApproach,
        points_max: 25,
        evaluator: eval_initial_approach_pitch_limit,
    },
    Rule {
        id: "initial_approach_flaps_deployed",
        label: "Flaps deployed",
        phase: Phase::InitialApproach,
        points_max: 20,
        evaluator: eval_initial_approach_flaps_deployed,
    },
    Rule {
        id: "initial_approach_ias_limit",
        label: "Indicated airspeed ≤ 250 kt (below 10,000 ft)",
        phase: Phase::InitialApproach,
        points_max: 30,
        evaluator: eval_initial_approach_ias_limit,
    },
    Rule {
        id: "initial_approach_landing_light",
        label: "Landing lights used",
        phase: Phase::InitialApproach,
        points_max: 10,
        evaluator: eval_initial_approach_landing_light,
    },
    // ============================================================
    // FINAL APPROACH (260 pts) — Below 1800 ft AGL
    // ============================================================
    Rule {
        id: "final_approach_pitch_limit",
        label: "Pitch never less than -15°",
        phase: Phase::FinalApproach,
        points_max: 25,
        evaluator: eval_final_approach_pitch_limit,
    },
    Rule {
        id: "final_approach_flaps_deployed",
        label: "Flaps deployed",
        phase: Phase::FinalApproach,
        points_max: 20,
        evaluator: eval_final_approach_flaps_deployed,
    },
    Rule {
        id: "final_approach_gear_down",
        label: "Landing gear deployed",
        phase: Phase::FinalApproach,
        points_max: 75,
        evaluator: eval_final_approach_gear_down,
    },
    Rule {
        id: "final_approach_spoilers_stowed",
        label: "Spoilers not deployed",
        phase: Phase::FinalApproach,
        points_max: 40,
        evaluator: eval_final_approach_spoilers_stowed,
    },
    Rule {
        id: "final_approach_ias_limit",
        label: "Indicated airspeed ≤ 250 kt (below 10,000 ft)",
        phase: Phase::FinalApproach,
        points_max: 100,
        evaluator: eval_final_approach_ias_limit,
    },
    // ============================================================
    // LANDING (50 pts) — Touchdown
    // ============================================================
    Rule {
        id: "landing_smooth",
        label: "Touchdown suave o firme (≥ -400 fpm; duro < -400)",
        phase: Phase::Landing,
        points_max: 50,
        evaluator: eval_landing_smooth,
    },
    // ============================================================
    // TAXI-IN (80 pts) — Rollout + rodaje a gate
    // ============================================================
    Rule {
        id: "taxi_in_landing_light_off",
        label: "Landing lights turned off",
        phase: Phase::TaxiIn,
        points_max: 20,
        evaluator: eval_taxi_in_landing_light_off,
    },
    Rule {
        id: "taxi_in_speed",
        label: "Ground speed never more than 30 kt",
        phase: Phase::TaxiIn,
        points_max: 30,
        evaluator: eval_taxi_in_speed,
    },
    Rule {
        id: "taxi_in_flaps_retracted",
        label: "Flaps retracted",
        phase: Phase::TaxiIn,
        points_max: 20,
        evaluator: eval_taxi_in_flaps_retracted,
    },
    Rule {
        id: "taxi_in_taxi_light",
        label: "Taxi light used",
        phase: Phase::TaxiIn,
        points_max: 10,
        evaluator: eval_taxi_in_taxi_light,
    },
    // ============================================================
    // ARRIVED (190 pts) — En gate, motores apagados
    // ============================================================
    Rule {
        id: "arrived_parking_brake",
        label: "Parking brake applied",
        phase: Phase::Arrived,
        points_max: 100,
        evaluator: eval_arrived_parking_brake,
    },
    Rule {
        id: "arrived_engines_off",
        label: "All engines off",
        phase: Phase::Arrived,
        points_max: 50,
        evaluator: eval_arrived_engines_off,
    },
    Rule {
        id: "arrived_flaps_up",
        label: "Flaps not deployed",
        phase: Phase::Arrived,
        points_max: 20,
        evaluator: eval_arrived_flaps_up,
    },
    Rule {
        id: "arrived_spoilers_stowed",
        label: "Spoilers not deployed",
        phase: Phase::Arrived,
        points_max: 20,
        evaluator: eval_arrived_spoilers_stowed,
    },
];

// =============================================================================
// Helpers — constructores de ScoreItem
// =============================================================================

fn pass(rule: &Rule, evidence: serde_json::Value) -> ScoreItem {
    ScoreItem {
        phase: rule.phase.as_str().to_string(),
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        points_earned: rule.points_max,
        points_max: rule.points_max,
        passed: true,
        severity: "info".to_string(),
        status: "done".to_string(),
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
        status: "missed".to_string(),
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
        status: if passed { "done" } else { "missed" }.to_string(),
        evidence,
    }
}

/// (v3.7.0) Marca una regla como "no evaluable" — los datos necesarios
/// no están en el track. Típico para VAS imports sin lights/pitch.
/// `item.points_max = 0` → no infla el denominador del score total.
fn skip(rule: &Rule, reason: &str) -> ScoreItem {
    ScoreItem {
        phase: rule.phase.as_str().to_string(),
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        points_earned: 0,
        points_max: 0,
        passed: true,
        severity: "skipped".to_string(),
        status: "na".to_string(),
        evidence: json!({ "reason": reason }),
    }
}

// =============================================================================
// Helpers — slices del track por sub-phase derivada
// =============================================================================

// (v3.21.0) FIX CRÍTICO de fases: para vuelos con `scoring_phase`
// canónico por-sample (todos los del watcher SimConnect + ACARS nuevos)
// filtramos DIRECTO por el nombre de fase del rubric
// (initial_climb / final_approach / pre_departure / arrived / …). Antes
// estos helpers consultaban los labels VIEJOS del badge ("climbing" /
// "approach" / "parking") que el watcher YA no persiste → todas esas
// fases salían "No data to evaluate". Para vuelos legacy (sin tag
// por-sample) caemos a la vía vieja (badge + banda AGL) preservando el
// scoring de los imports VAS.

/// Pre-Departure (antes del pushback).
fn samples_pre_departure<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("pre_departure");
    }
    ctx.samples_in_phase("preflight")
}

/// Take-Off Acceleration (0..1000 ft AGL).
fn samples_takeoff_accel<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("takeoff_accel");
    }
    ctx.samples_in_phase("takeoff")
        .into_iter()
        .chain(ctx.samples_in_phase("climbing").into_iter())
        .filter(|s| s.alt_ft.map(|a| a < 1000).unwrap_or(false))
        .collect()
}

/// Initial Climb (1000..10000 ft AGL).
fn samples_initial_climb<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("initial_climb");
    }
    ctx.samples_in_phase("climbing")
        .into_iter()
        .filter(|s| s.alt_ft.map(|a| (1000..10000).contains(&a)).unwrap_or(false))
        .collect()
}

/// Initial Approach (1800..10000 ft, descending).
fn samples_initial_approach<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("initial_approach");
    }
    ctx.samples_in_phase("approach")
        .into_iter()
        .chain(ctx.samples_in_phase("descent").into_iter())
        .filter(|s| s.alt_ft.map(|a| a > 1800 && a < 10000).unwrap_or(false))
        .collect()
}

/// Final Approach (below 1800 ft AGL).
fn samples_final_approach<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("final_approach");
    }
    ctx.samples_in_phase("approach")
        .into_iter()
        .filter(|s| s.alt_ft.map(|a| a <= 1800).unwrap_or(false))
        .collect()
}

/// Taxi-In (post-landing).
fn samples_taxi_in<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("taxi_in");
    }
    ctx.samples_in_phase("taxi_in")
        .into_iter()
        .chain(ctx.samples_in_phase("landed_rollout").into_iter())
        .collect()
}

/// (v4.6.2) Taxi-In "asentado" — excluye la cola de desaceleración del
/// rollout (la bajada monótona de velocidad en pista, que el detector de
/// fase mete en "taxi_in" porque separa fases por velocidad, no por
/// geometría de pista). Devuelve las muestras DESDE que el avión deja de
/// frenar (ya maniobrando en calle de rodaje). Ahí tiene sentido medir la
/// velocidad de taxi y exigir que las luces de aterrizaje ya estén
/// apagadas (after-landing flow hecho).
fn samples_taxi_in_settled<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    let s = samples_taxi_in(ctx);
    if s.len() <= 1 {
        return s;
    }
    let mut start = 0usize;
    while start + 1 < s.len() {
        match (s[start].gs_kt, s[start + 1].gs_kt) {
            (Some(a), Some(b)) if b < a => start += 1,
            _ => break,
        }
    }
    s[start..].to_vec()
}

/// (v4.6.2) Núcleo del crucero — sólo muestras dentro de 3000 ft del techo
/// del vuelo. El detector de fase también etiqueta "cruise" a las
/// nivelaciones BAJAS (banda 1800..10000 ft nivelado: escalones de ATC,
/// tramos nivelados del descenso), donde puede haber flaps/gear/luces
/// desplegados legítimamente. Para evaluar la config de crucero (avión
/// "limpio") sólo miramos el crucero REAL (cerca del techo), evitando
/// falsos fallos por esas nivelaciones bajas.
fn samples_cruise_core<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    let cruise = ctx.samples_in_phase("cruise");
    if cruise.is_empty() {
        return cruise;
    }
    let max_alt = cruise.iter().filter_map(|s| s.alt_ft).max().unwrap_or(0);
    if max_alt <= 0 {
        return cruise;
    }
    let floor = max_alt - 3000;
    let core: Vec<&TrackSample> = cruise
        .iter()
        .copied()
        .filter(|s| s.alt_ft.map(|a| a >= floor).unwrap_or(false))
        .collect();
    if core.is_empty() {
        cruise
    } else {
        core
    }
}

/// Arrived (engines off / parking / deboarding).
fn samples_arrived<'a>(ctx: &'a FlightContext) -> Vec<&'a TrackSample> {
    if ctx.has_scoring_phase() {
        return ctx.samples_in_phase("arrived");
    }
    ctx.samples_in_phase("parking")
        .into_iter()
        .chain(ctx.samples_in_phase("deboarding").into_iter())
        .collect()
}

// =============================================================================
// Helpers — aggregations de campos opcionales
// =============================================================================

/// True si **algún** sample del slice tiene el extractor Some — usado
/// para saber si la regla tiene datos para evaluar o debe ser skipped.
fn any_present<F, T>(samples: &[&TrackSample], extract: F) -> bool
where
    F: Fn(&TrackSample) -> Option<T>,
{
    samples.iter().any(|s| extract(s).is_some())
}

/// Max absoluto de un campo f64 sobre los samples.
fn max_abs_f64<F>(samples: &[&TrackSample], extract: F) -> Option<f64>
where
    F: Fn(&TrackSample) -> Option<f64>,
{
    samples
        .iter()
        .filter_map(|s| extract(s))
        .map(f64::abs)
        .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
}

/// Max de un campo f64 (con signo).
fn max_f64<F>(samples: &[&TrackSample], extract: F) -> Option<f64>
where
    F: Fn(&TrackSample) -> Option<f64>,
{
    samples
        .iter()
        .filter_map(|s| extract(s))
        .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
}

/// Min de un campo f64 (con signo).
fn min_f64<F>(samples: &[&TrackSample], extract: F) -> Option<f64>
where
    F: Fn(&TrackSample) -> Option<f64>,
{
    samples
        .iter()
        .filter_map(|s| extract(s))
        .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.min(v))))
}

/// Max de un campo i64.
fn max_i64<F>(samples: &[&TrackSample], extract: F) -> Option<i64>
where
    F: Fn(&TrackSample) -> Option<i64>,
{
    samples
        .iter()
        .filter_map(|s| extract(s))
        .max()
}

/// % de samples cuyo bool sea Some(true). Si ninguno tiene dato, None.
fn pct_bool_true<F>(samples: &[&TrackSample], extract: F) -> Option<f32>
where
    F: Fn(&TrackSample) -> Option<bool>,
{
    let present: Vec<bool> = samples.iter().filter_map(|s| extract(s)).collect();
    if present.is_empty() {
        return None;
    }
    let true_count = present.iter().filter(|v| **v).count();
    Some(true_count as f32 / present.len() as f32)
}

// =============================================================================
// Evaluadores — GENERAL
// =============================================================================

fn eval_pitch_up_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    if !any_present(&ctx.track.iter().collect::<Vec<_>>(), |s| s.pitch_deg) {
        return skip(rule, "no_pitch_data");
    }
    let max_up = max_f64(&ctx.track.iter().collect::<Vec<_>>(), |s| s.pitch_deg).unwrap_or(0.0);
    let evidence = json!({ "max_pitch_up_deg": max_up });
    match max_up {
        v if v <= 20.0 => pass(rule, evidence),
        v if v <= 25.0 => partial(rule, (rule.points_max * 2) / 3, evidence),
        v if v <= 30.0 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_pitch_down_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    if !any_present(&ctx.track.iter().collect::<Vec<_>>(), |s| s.pitch_deg) {
        return skip(rule, "no_pitch_data");
    }
    let min_down = min_f64(&ctx.track.iter().collect::<Vec<_>>(), |s| s.pitch_deg).unwrap_or(0.0);
    let evidence = json!({ "min_pitch_down_deg": min_down });
    match min_down {
        v if v >= -20.0 => pass(rule, evidence),
        v if v >= -25.0 => partial(rule, (rule.points_max * 2) / 3, evidence),
        v if v >= -30.0 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_bank_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let track: Vec<&TrackSample> = ctx.track.iter().collect();
    if !any_present(&track, |s| s.bank_deg) {
        return skip(rule, "no_bank_data");
    }
    // (v3.29.0 #8) Robustez: medimos el alabeo SOSTENIDO con el
    // percentil 98 de |bank|, no el máximo absoluto. El usuario reportó
    // que la regla era poco fiable: un único sample espurio (glitch del
    // simvar o un alabeo de medio segundo en un viraje normal) tumbaba
    // los puntos. El p98 ignora 1-2 picos aislados y refleja el alabeo
    // real del vuelo. Guardamos también el máximo en la evidencia para
    // que el usuario vea ambos números.
    let mut banks: Vec<f64> = track
        .iter()
        .filter_map(|s| s.bank_deg.map(f64::abs))
        .collect();
    banks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((banks.len() as f64 * 0.98).ceil() as usize)
        .saturating_sub(1)
        .min(banks.len() - 1);
    let p98 = banks[idx];
    let max_bank = banks.last().copied().unwrap_or(0.0);
    let evidence = json!({ "bank_p98_deg": p98 as i64, "max_bank_deg": max_bank as i64 });
    match p98 {
        v if v <= 30.0 => pass(rule, evidence),
        v if v <= 45.0 => partial(rule, (rule.points_max * 2) / 3, evidence),
        v if v <= 60.0 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_g_force_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    if !any_present(&ctx.track.iter().collect::<Vec<_>>(), |s| s.g_force) {
        return skip(rule, "no_g_force_data");
    }
    let max_g = max_f64(&ctx.track.iter().collect::<Vec<_>>(), |s| s.g_force).unwrap_or(1.0);
    let evidence = json!({ "max_g_force": max_g });
    match max_g {
        v if v <= 2.0 => pass(rule, evidence),
        v if v <= 2.5 => partial(rule, (rule.points_max * 2) / 3, evidence),
        v if v <= 3.0 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_no_overspeed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // (v3.7.0) Proxy de overspeed: IAS > 350 kt bajo 10k ft. SimConnect
    // expone `OVERSPEED INDICATOR` pero requiere otro simvar; usamos
    // IAS+ALT como heurística estable. Si no hay IAS, skip.
    let track_refs: Vec<&TrackSample> = ctx.track.iter().collect();
    if !any_present(&track_refs, |s| s.ias_kt) {
        return skip(rule, "no_ias_data");
    }
    let over: Vec<(i64, i64)> = track_refs
        .iter()
        .filter_map(|s| match (s.alt_ft, s.ias_kt) {
            (Some(alt), Some(ias)) if alt < 10000 && ias > 350 => Some((alt, ias)),
            _ => None,
        })
        .collect();
    let evidence = json!({ "overspeed_samples": over.len(), "examples": over.iter().take(3).collect::<Vec<_>>() });
    match over.len() {
        0 => pass(rule, evidence),
        1..=3 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_no_stall(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // (v3.21.0) STALL WARNING ahora se captura por sample. Si ningún
    // sample lo tiene (vuelo viejo / VAS import), skip honesto.
    let track: Vec<&TrackSample> = ctx.track.iter().collect();
    if !any_present(&track, |s| s.stall_warning) {
        return skip(rule, "stall_indicator_not_captured");
    }
    let stalls = track
        .iter()
        .filter(|s| s.stall_warning == Some(true))
        .count();
    let evidence = json!({ "stall_warning_samples": stalls });
    match stalls {
        0 => pass(rule, evidence),
        1..=2 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_max_service_ceiling(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Sin la design max alt del avión, no podemos comparar. Heurística:
    // si max_altitude_ft > 50,000 ft probablemente está fuera de specs
    // de un avión comercial típico. Aceptable como guard amplio.
    let Some(max_alt) = ctx.max_altitude_ft else {
        return skip(rule, "no_max_alt_data");
    };
    let evidence = json!({ "max_altitude_ft": max_alt });
    match max_alt {
        a if a <= 45000 => pass(rule, evidence),
        a if a <= 51000 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

// =============================================================================
// Evaluadores — PRE-DEPARTURE
// =============================================================================

fn eval_predep_parking_brake(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let all = samples_pre_departure(ctx);
    if all.is_empty() {
        return skip(rule, "no_pre_departure_phase");
    }
    // (v4.6.2) Solo el periodo ESTÁTICO en el gate (gs < 1 kt). Soltar el
    // freno para el pushback (ya en movimiento) es lo normal y no debe
    // penalizar. Antes medíamos sobre TODA la fase pre_departure — que
    // incluye el pushback con freno suelto — y el % bajaba → ✗ injusto.
    let static_samples: Vec<&TrackSample> = all
        .iter()
        .copied()
        .filter(|s| s.gs_kt.map(|g| g < 1).unwrap_or(true))
        .collect();
    let samples = if static_samples.is_empty() {
        all
    } else {
        static_samples
    };
    if !any_present(&samples, |s| s.parking_brake) {
        return skip(rule, "no_parking_brake_data");
    }
    let pct = pct_bool_true(&samples, |s| s.parking_brake).unwrap_or(0.0);
    let evidence = json!({ "parking_brake_set_pct_static": (pct * 100.0) as i32 });
    // (v4.15.0 #3) Umbrales tolerantes a la lectura INTERMITENTE del simvar
    // BRAKE PARKING POSITION en aviones complejos (PMDG/iFly), que provoca
    // falsos negativos: sobre el periodo estático en el gate, con que el
    // freno haya estado puesto la mayor parte del tiempo basta para ✓.
    if pct >= 0.5 {
        pass(rule, evidence)
    } else if pct >= 0.25 {
        partial(rule, (rule.points_max * 2) / 3, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_predep_engines_off(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // (v4.6.1 FIX) La fase pre_departure INCLUYE el arranque de motores en
    // el gate (es lo normal: cold & dark → start → pushback). Antes
    // tomábamos el N1 MÁXIMO de la fase, que siempre era alto (motores ya
    // arrancados) → la regla SIEMPRE marcaba ✗ aunque el usuario empezara
    // bien en frío. El intento real de esta regla es premiar el arranque
    // EN FRÍO: que los motores hayan estado apagados AL INICIO. Por eso
    // usamos el N1 MÍNIMO de la fase: si en algún momento estuvieron
    // apagados (<5%), el usuario empezó cold & dark → ✓. Solo si NUNCA
    // estuvieron apagados (spawn "ready-to-fly" con motores en marcha)
    // baja la nota. (El APU NO cuenta: eng_n1 lee TURB ENG N1 de los
    // motores 1-4, no el APU.)
    let samples = samples_pre_departure(ctx);
    if samples.is_empty() {
        return skip(rule, "no_pre_departure_phase");
    }
    if !any_present(&samples, |s| s.eng_n1_max) {
        return skip(rule, "no_engine_data");
    }
    let min_n1 = samples
        .iter()
        .filter_map(|s| s.eng_n1_max)
        .fold(f64::INFINITY, f64::min);
    let evidence = json!({ "min_n1_pct_pre_departure": min_n1 });
    if min_n1 < 5.0 {
        pass(rule, evidence)
    } else if min_n1 < 20.0 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_predep_flaps_up(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_pre_departure(ctx);
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    let max_flaps = max_i64(&samples, |s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "max_flaps_pct_during_predep": max_flaps });
    if max_flaps <= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_predep_spoilers_stowed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_pre_departure(ctx);
    if !any_present(&samples, |s| s.spoilers_pct) {
        return skip(rule, "no_spoilers_data");
    }
    let max_sp = max_i64(&samples, |s| s.spoilers_pct).unwrap_or(0);
    let evidence = json!({ "max_spoilers_pct_during_predep": max_sp });
    if max_sp <= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_predep_ample_time(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // Duración de la phase preflight (entered_at..exited_at). Si dura
    // ≥ 5 min, pass. Menos, partial; nada, skip.
    let Some(range) = ctx
        .phases
        .iter()
        .find(|p| p.phase == "pre_departure" || p.phase == "preflight")
    else {
        return skip(rule, "no_preflight_phase");
    };
    let entered = chrono::DateTime::parse_from_rfc3339(&range.entered_at).ok();
    let exited = range
        .exited_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let (Some(a), Some(b)) = (entered, exited) else {
        return skip(rule, "preflight_phase_unclosed");
    };
    let secs = (b - a).num_seconds();
    let evidence = json!({ "preflight_seconds": secs });
    match secs {
        s if s >= 300 => pass(rule, evidence),
        s if s >= 120 => partial(rule, (rule.points_max * 2) / 3, evidence),
        s if s >= 60 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

// =============================================================================
// Evaluadores — PUSHBACK
// =============================================================================

fn eval_pushback_nav_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("pushback");
    if samples.is_empty() {
        return skip(rule, "no_pushback_phase");
    }
    if !any_present(&samples, |s| s.light_nav) {
        return skip(rule, "no_light_nav_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_nav).unwrap_or(0.0);
    let evidence = json!({ "nav_light_on_pct": (pct * 100.0) as i32 });
    if pct >= 0.9 {
        pass(rule, evidence)
    } else if pct >= 0.5 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_pushback_beacon_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("pushback");
    if samples.is_empty() {
        return skip(rule, "no_pushback_phase");
    }
    if !any_present(&samples, |s| s.light_beacon) {
        return skip(rule, "no_light_beacon_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_beacon).unwrap_or(0.0);
    let evidence = json!({ "beacon_light_on_pct": (pct * 100.0) as i32 });
    if pct >= 0.9 {
        pass(rule, evidence)
    } else if pct >= 0.5 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_pushback_doors_closed(_ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    skip(rule, "doors_open_simvar_not_captured")
}

// =============================================================================
// Evaluadores — TAXI-OUT
// =============================================================================

fn eval_taxi_out_speed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_out");
    if samples.is_empty() {
        return skip(rule, "no_taxi_out_phase");
    }
    let max_gs = max_i64(&samples, |s| s.gs_kt).unwrap_or(0);
    let evidence = json!({ "max_gs_kt": max_gs });
    match max_gs {
        n if n <= 30 => pass(rule, evidence),
        n if n <= 40 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 50 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_taxi_out_flaps_set(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_out");
    if samples.is_empty() {
        return skip(rule, "no_taxi_out_phase");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    let max_flaps = max_i64(&samples, |s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "max_flaps_pct_taxi_out": max_flaps });
    if max_flaps >= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_taxi_out_transponder(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_out");
    if samples.is_empty() {
        return skip(rule, "no_taxi_out_phase");
    }
    if !any_present(&samples, |s| s.transponder_code) {
        return skip(rule, "no_transponder_data");
    }
    let max_code = max_i64(&samples, |s| s.transponder_code).unwrap_or(0);
    let evidence = json!({ "max_transponder_code": max_code });
    // 1200 (VFR) o 7000 son códigos non-IFR — el ATC asigna IFR > 0 y
    // distinto a 1200/7000. Aceptamos cualquier código > 0 que NO sea
    // 1200/7000 como "IFR set".
    if max_code > 0 && max_code != 1200 && max_code != 7000 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", rule.points_max / 2, evidence)
    }
}

fn eval_taxi_out_taxi_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let all = ctx.samples_in_phase("taxi_out");
    if all.is_empty() {
        return skip(rule, "no_taxi_out_phase");
    }
    if !any_present(&all, |s| s.light_taxi) {
        return skip(rule, "no_light_taxi_data");
    }
    // (v4.15.0 #3) Margen de tolerancia: la taxi light solo aplica cuando
    // el avión RUEDA (gs ≥ 5 kt) — detenido en un hold-short con la luz
    // apagada no es falta. Además damos un colchón de ~30-45s de rodaje
    // continuo antes de penalizar: si el avión rodó poco tiempo (pocas
    // muestras en movimiento), se le da el beneficio de la duda.
    let moving: Vec<&TrackSample> = all
        .iter()
        .copied()
        .filter(|s| s.gs_kt.map(|g| g >= 5).unwrap_or(false))
        .collect();
    if moving.is_empty() {
        // Nunca rodó de verdad (gate/empuje) → no penalizamos.
        return skip(rule, "no_taxi_movement");
    }
    // Aproximación del colchón temporal: con muestreo ~3-5s, ~10 muestras
    // en movimiento ≈ 30-50s. Por debajo de eso, rodaje demasiado breve
    // para exigir la luz → ✓.
    if moving.len() <= 10 {
        let evidence = json!({ "moving_samples": moving.len(), "grace": true });
        return pass(rule, evidence);
    }
    let pct = pct_bool_true(&moving, |s| s.light_taxi).unwrap_or(0.0);
    let evidence = json!({ "taxi_light_on_pct_moving": (pct * 100.0) as i32 });
    if pct >= 0.7 {
        pass(rule, evidence)
    } else if pct >= 0.35 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_taxi_out_nav_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("taxi_out");
    if samples.is_empty() {
        return skip(rule, "no_taxi_out_phase");
    }
    if !any_present(&samples, |s| s.light_nav) {
        return skip(rule, "no_light_nav_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_nav).unwrap_or(0.0);
    let evidence = json!({ "nav_light_on_pct": (pct * 100.0) as i32 });
    if pct >= 0.9 {
        pass(rule, evidence)
    } else if pct >= 0.5 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

// =============================================================================
// Evaluadores — TAKE-OFF
// =============================================================================

fn eval_takeoff_landing_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("takeoff");
    if samples.is_empty() {
        return skip(rule, "no_takeoff_phase");
    }
    if !any_present(&samples, |s| s.light_landing) {
        return skip(rule, "no_light_landing_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_landing).unwrap_or(0.0);
    let evidence = json!({ "landing_light_on_pct": (pct * 100.0) as i32 });
    if pct >= 0.8 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_takeoff_strobe_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("takeoff");
    if samples.is_empty() {
        return skip(rule, "no_takeoff_phase");
    }
    if !any_present(&samples, |s| s.light_strobe) {
        return skip(rule, "no_light_strobe_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_strobe).unwrap_or(0.0);
    let evidence = json!({ "strobe_light_on_pct": (pct * 100.0) as i32 });
    if pct >= 0.8 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_takeoff_pitch_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = ctx.samples_in_phase("takeoff");
    if samples.is_empty() {
        return skip(rule, "no_takeoff_phase");
    }
    if !any_present(&samples, |s| s.pitch_deg) {
        return skip(rule, "no_pitch_data");
    }
    let max_pitch = max_f64(&samples, |s| s.pitch_deg).unwrap_or(0.0);
    let evidence = json!({ "max_pitch_deg_during_takeoff": max_pitch });
    if max_pitch <= 15.0 {
        pass(rule, evidence)
    } else if max_pitch <= 20.0 {
        partial(rule, (rule.points_max * 2) / 3, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

// =============================================================================
// Evaluadores — TAKE-OFF ACCELERATION (0..1000 ft AGL)
// =============================================================================

fn eval_takeoff_accel_gear_up(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let to = samples_takeoff_accel(ctx);
    if to.is_empty() {
        return skip(rule, "no_takeoff_accel_data");
    }
    if !any_present(&to, |s| s.gear_down) {
        return skip(rule, "no_gear_data");
    }
    // (v4.6.2) El tren está ABAJO en la carrera y el liftoff (correcto) y
    // se retrae justo después. Antes mirábamos SOLO el último sample de la
    // aceleración, que con muestreo escaso podía caer en el liftoff (tren
    // abajo) → ✗ injusto, "obviamente subo el tren al despegar". Ahora
    // aceptamos si el tren se retrajo en CUALQUIER punto del low-climb, o
    // si ya está arriba al empezar el initial climb (~1000 ft).
    let retracted_in_to = to.iter().any(|s| s.gear_down == Some(false));
    let ic = samples_initial_climb(ctx);
    let retracted_by_climb = ic.iter().take(3).any(|s| s.gear_down == Some(false));
    let last_gear_up = to.last().and_then(|s| s.gear_down) == Some(false);
    let retracted = retracted_in_to || retracted_by_climb || last_gear_up;
    let evidence = json!({
        "retracted_in_takeoff_accel": retracted_in_to,
        "retracted_by_initial_climb": retracted_by_climb,
    });
    if retracted {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_takeoff_accel_ias_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_takeoff_accel(ctx);
    if samples.is_empty() {
        return skip(rule, "no_takeoff_accel_data");
    }
    if !any_present(&samples, |s| s.ias_kt) {
        return skip(rule, "no_ias_data");
    }
    let max_ias = max_i64(&samples, |s| s.ias_kt).unwrap_or(0);
    let evidence = json!({ "max_ias_kt_below_1000ft": max_ias });
    match max_ias {
        n if n <= 250 => pass(rule, evidence),
        n if n <= 280 => partial(rule, rule.points_max / 2, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

// =============================================================================
// Evaluadores — INITIAL CLIMB (1000..10000 ft)
// =============================================================================

fn eval_initial_climb_flaps_retracted(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_climb(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_climb_data");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    // (v4.15.0 #3) "Flaps completamente limpios antes de 10.000 ft": basta
    // con que en ALGÚN momento del ascenso inicial los flaps llegaran a
    // configuración limpia (mín de flaps ≤ 2%). Antes mirábamos solo el
    // ÚLTIMO sample, que podía ser ruidoso y dar falsos negativos si la
    // retracción terminaba justo en el borde de los 10k.
    let min_flaps = samples
        .iter()
        .filter_map(|s| s.flaps_pct)
        .min()
        .unwrap_or(0);
    let evidence = json!({ "min_flaps_pct_initial_climb": min_flaps });
    if min_flaps <= 2 {
        pass(rule, evidence)
    } else if min_flaps <= 15 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_initial_climb_landing_light_off(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_climb(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_climb_data");
    }
    if !any_present(&samples, |s| s.light_landing) {
        return skip(rule, "no_light_landing_data");
    }
    // (v3.29.0 #8) Banda de gracia: NO fallar en seco en el borde de los
    // 10.000 ft. El corte real es ~10k con margen práctico hasta ~11k.
    // La fase initial_climb tope a 10.000 ft AGL, así que si en su tope
    // la landing light SIGUE encendida damos un WARN parcial, no un fail
    // (el enforcement duro por encima de 10k lo hace la regla de cruise,
    // que además tolera el apagado tardío vía porcentaje). Si ya está
    // apagada, pase completo.
    let last_ll = samples.last().and_then(|s| s.light_landing).unwrap_or(true);
    let evidence = json!({ "landing_light_on_at_top_of_climb": last_ll });
    if !last_ll {
        pass(rule, evidence)
    } else {
        partial(rule, rule.points_max / 2, evidence)
    }
}

// =============================================================================
// Evaluadores — CRUISE (above 10k ft)
// =============================================================================

fn eval_cruise_gear_up(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_cruise_core(ctx);
    if samples.is_empty() {
        return skip(rule, "no_cruise_phase");
    }
    if !any_present(&samples, |s| s.gear_down) {
        return skip(rule, "no_gear_data");
    }
    let any_down = samples.iter().any(|s| s.gear_down == Some(true));
    let evidence = json!({ "gear_down_during_cruise": any_down });
    if !any_down {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_cruise_landing_light_off(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let core = samples_cruise_core(ctx);
    if core.is_empty() {
        return skip(rule, "no_cruise_phase");
    }
    if !any_present(&core, |s| s.light_landing) {
        return skip(rule, "no_light_landing_data");
    }
    // (v4.15.0 #3) Margen regulatorio hasta 11.000 ft: la landing light
    // solo se considera "tarde" (falta) si sigue encendida POR ENCIMA de
    // 11.000 ft. Entre 10k y 11k es zona de gracia (apagado tardío normal).
    let samples: Vec<&TrackSample> = core
        .iter()
        .copied()
        .filter(|s| s.alt_ft.map(|a| a > 11000).unwrap_or(false))
        .collect();
    if samples.is_empty() {
        // No hubo crucero por encima de 11k → nada que penalizar aquí.
        return pass(rule, json!({ "no_samples_above_11000ft": true }));
    }
    let pct_on = pct_bool_true(&samples, |s| s.light_landing).unwrap_or(0.0);
    let evidence = json!({ "landing_light_on_pct_above_11000ft": (pct_on * 100.0) as i32 });
    if pct_on <= 0.05 {
        pass(rule, evidence)
    } else if pct_on <= 0.3 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_cruise_flaps_up(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_cruise_core(ctx);
    if samples.is_empty() {
        return skip(rule, "no_cruise_phase");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    let max_flaps = max_i64(&samples, |s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "max_flaps_pct_cruise": max_flaps });
    if max_flaps <= 2 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_cruise_doors_closed(_ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    skip(rule, "doors_open_simvar_not_captured")
}

// =============================================================================
// Evaluadores — INITIAL APPROACH (1800..10000 ft)
// =============================================================================

fn eval_initial_approach_pitch_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_approach_data");
    }
    if !any_present(&samples, |s| s.pitch_deg) {
        return skip(rule, "no_pitch_data");
    }
    let min_pitch = min_f64(&samples, |s| s.pitch_deg).unwrap_or(0.0);
    let evidence = json!({ "min_pitch_deg_initial_approach": min_pitch });
    if min_pitch >= -15.0 {
        pass(rule, evidence)
    } else if min_pitch >= -20.0 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_initial_approach_flaps_deployed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_approach_data");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    // En el último sample (cerca de 1800 ft) flaps debería estar >0.
    let last_flaps = samples.last().and_then(|s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "flaps_pct_at_1800ft": last_flaps });
    if last_flaps >= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_initial_approach_ias_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_approach_data");
    }
    if !any_present(&samples, |s| s.ias_kt) {
        return skip(rule, "no_ias_data");
    }
    let max_ias = max_i64(&samples, |s| s.ias_kt).unwrap_or(0);
    let evidence = json!({ "max_ias_kt_initial_approach": max_ias, "limit_kt": 261 });
    // (v4.15.0 #3) Límite sub-10.000 ft = 250 kt (FAR 91.117) + margen
    // técnico de tolerancia a +11 kt (261) para absorber ráfagas/turbulencia
    // en el descenso sin penalizar injustamente.
    match max_ias {
        n if n <= 261 => pass(rule, evidence),
        n if n <= 280 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 300 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_initial_approach_landing_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_initial_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_initial_approach_data");
    }
    if !any_present(&samples, |s| s.light_landing) {
        return skip(rule, "no_light_landing_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_landing).unwrap_or(0.0);
    let evidence = json!({ "landing_light_on_pct_initial_approach": (pct * 100.0) as i32 });
    if pct >= 0.7 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

// =============================================================================
// Evaluadores — FINAL APPROACH (below 1800 ft AGL)
// =============================================================================

fn eval_final_approach_pitch_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_final_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_final_approach_data");
    }
    if !any_present(&samples, |s| s.pitch_deg) {
        return skip(rule, "no_pitch_data");
    }
    let min_pitch = min_f64(&samples, |s| s.pitch_deg).unwrap_or(0.0);
    let evidence = json!({ "min_pitch_deg_final_approach": min_pitch });
    if min_pitch >= -15.0 {
        pass(rule, evidence)
    } else if min_pitch >= -20.0 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_final_approach_flaps_deployed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_final_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_final_approach_data");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    let min_flaps = samples.iter().filter_map(|s| s.flaps_pct).min().unwrap_or(0);
    let evidence = json!({ "min_flaps_pct_final_approach": min_flaps });
    if min_flaps >= 20 {
        pass(rule, evidence)
    } else if min_flaps >= 5 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_final_approach_gear_down(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_final_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_final_approach_data");
    }
    if !any_present(&samples, |s| s.gear_down) {
        return skip(rule, "no_gear_data");
    }
    let pct_down = pct_bool_true(&samples, |s| s.gear_down).unwrap_or(0.0);
    let evidence = json!({ "gear_down_pct_final_approach": (pct_down * 100.0) as i32 });
    if pct_down >= 0.95 {
        pass(rule, evidence)
    } else if pct_down >= 0.5 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_final_approach_spoilers_stowed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_final_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_final_approach_data");
    }
    if !any_present(&samples, |s| s.spoilers_pct) {
        return skip(rule, "no_spoilers_data");
    }
    // Spoilers armed (no deployed) — flight spoilers en armado son OK,
    // pero deployed (>30%) durante final approach es malo. Aceptamos ≤ 30%.
    let max_sp = max_i64(&samples, |s| s.spoilers_pct).unwrap_or(0);
    let evidence = json!({ "max_spoilers_pct_final_approach": max_sp });
    if max_sp <= 30 {
        pass(rule, evidence)
    } else if max_sp <= 60 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_final_approach_ias_limit(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_final_approach(ctx);
    if samples.is_empty() {
        return skip(rule, "no_final_approach_data");
    }
    if !any_present(&samples, |s| s.ias_kt) {
        return skip(rule, "no_ias_data");
    }
    let max_ias = max_i64(&samples, |s| s.ias_kt).unwrap_or(0);
    let evidence = json!({ "max_ias_kt_final_approach": max_ias, "limit_kt": 261 });
    // (v4.15.0 #3) Igual que initial approach: 250 kt sub-10k + margen
    // técnico a 261 kt para no penalizar por ráfagas/turbulencia.
    match max_ias {
        n if n <= 261 => pass(rule, evidence),
        n if n <= 280 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 300 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

// =============================================================================
// Evaluadores — LANDING (touchdown)
// =============================================================================

fn eval_landing_smooth(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let Some(fpm) = ctx.landing_fpm else {
        return skip(rule, "no_landing_fpm");
    };
    // (v3.29.0 #8) Banda + categoría legible en la evidencia para que el
    // usuario entienda por qué ganó/perdió puntos.
    let category = match fpm {
        f if f > -240 => "smooth",
        f if f > -400 => "firm",
        f if f > -600 => "hard",
        f if f > -1000 => "very_hard",
        _ => "structural_damage",
    };
    let evidence = json!({ "landing_fpm": fpm, "category": category });
    // Escala alineada con aviación real y con el color del FlightBook:
    //   suave  ≳ -240 fpm       → pase completo
    //   firme   -240..-400      → 3/4
    //   duro    -400..-600      → 1/2
    //   muy duro -600..-1000    → 1/5 (probable daño)
    //   peor que -1000          → 0 (daño estructural)
    // (v4.6.0) En el checklist (✓/✗) un aterrizaje SUAVE o FIRME es un
    // pase: -388 fpm es un toque normal, no un fallo. Solo "duro"
    // (≤ -400) o peor cuentan como no cumplido. Antes firme caía en
    // `partial` (3/4) → status "missed" → ✗, lo que el usuario reportó
    // como injusto.
    match fpm {
        f if f > -400 => pass(rule, evidence), // suave o firme → ✓
        f if f > -600 => partial(rule, rule.points_max / 2, evidence), // duro
        f if f > -1000 => partial(rule, rule.points_max / 5, evidence), // muy duro
        _ => fail(rule, "fail", 0, evidence), // daño estructural
    }
}

// =============================================================================
// Evaluadores — TAXI-IN
// =============================================================================

fn eval_taxi_in_landing_light_off(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // (v4.6.2) Usamos el taxi-in ASENTADO: el after-landing flow (apagar
    // luces de aterrizaje) ocurre mientras aún frenas en pista, y ese
    // tramo cae en "taxi_in". Excluyéndolo, medimos las luces ya
    // maniobrando en calle de rodaje, donde deben estar apagadas.
    let samples = samples_taxi_in_settled(ctx);
    if samples.is_empty() {
        return skip(rule, "no_taxi_in_data");
    }
    if !any_present(&samples, |s| s.light_landing) {
        return skip(rule, "no_light_landing_data");
    }
    let pct_on = pct_bool_true(&samples, |s| s.light_landing).unwrap_or(0.0);
    let evidence = json!({ "landing_light_on_pct_taxi_in": (pct_on * 100.0) as i32 });
    if pct_on <= 0.1 {
        pass(rule, evidence)
    } else if pct_on <= 0.4 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_taxi_in_speed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_taxi_in(ctx);
    if samples.is_empty() {
        return skip(rule, "no_taxi_in_data");
    }
    // (v4.6.0) La cola del rollout (desaceleración en pista) cae dentro de
    // "taxi_in" porque separamos las fases por velocidad, no por geometría
    // de pista. Esa bajada monótona desde ~40 kt NO es "taxi rápido". La
    // excluimos: saltamos la racha inicial en la que la velocidad SOLO
    // baja y medimos la máxima a partir de que el avión deja de frenar (ya
    // maniobrando en calle de rodaje). Así un taxi rápido real (acelera en
    // taxiway) sí se detecta, pero el frenado post-touchdown no penaliza.
    let gss: Vec<i64> = samples.iter().filter_map(|s| s.gs_kt).collect();
    if gss.is_empty() {
        return skip(rule, "no_gs_data");
    }
    let mut start = 0usize;
    while start + 1 < gss.len() && gss[start + 1] < gss[start] {
        start += 1;
    }
    let max_gs = gss[start..].iter().copied().max().unwrap_or(0);
    let evidence = json!({
        "max_gs_kt_taxi_in": max_gs,
        "rollout_samples_skipped": start,
    });
    match max_gs {
        n if n <= 30 => pass(rule, evidence),
        n if n <= 40 => partial(rule, (rule.points_max * 2) / 3, evidence),
        n if n <= 50 => partial(rule, rule.points_max / 3, evidence),
        _ => fail(rule, "fail", 0, evidence),
    }
}

fn eval_taxi_in_flaps_retracted(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_taxi_in(ctx);
    if samples.is_empty() {
        return skip(rule, "no_taxi_in_data");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    // En el ÚLTIMO sample del taxi-in los flaps deberían estar ya
    // retraídos. Si nunca se retraen, fail.
    let last_flaps = samples.last().and_then(|s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "flaps_pct_end_taxi_in": last_flaps });
    if last_flaps <= 2 {
        pass(rule, evidence)
    } else if last_flaps <= 15 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_taxi_in_taxi_light(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_taxi_in(ctx);
    if samples.is_empty() {
        return skip(rule, "no_taxi_in_data");
    }
    if !any_present(&samples, |s| s.light_taxi) {
        return skip(rule, "no_light_taxi_data");
    }
    let pct = pct_bool_true(&samples, |s| s.light_taxi).unwrap_or(0.0);
    let evidence = json!({ "taxi_light_on_pct_taxi_in": (pct * 100.0) as i32 });
    if pct >= 0.8 {
        pass(rule, evidence)
    } else if pct >= 0.4 {
        partial(rule, rule.points_max / 2, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

// =============================================================================
// Evaluadores — ARRIVED (en gate, motores off)
// =============================================================================

fn eval_arrived_parking_brake(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_arrived(ctx);
    if samples.is_empty() {
        // Fallback: usar el último sample del track (si no hay phase
        // "parking" explícita).
        if ctx.track.is_empty() {
            return skip(rule, "no_arrived_data");
        }
        let last = ctx.track.last().unwrap();
        if let Some(b) = last.parking_brake {
            let evidence = json!({ "parking_brake_at_end": b });
            return if b { pass(rule, evidence) } else { fail(rule, "fail", 0, evidence) };
        }
        return skip(rule, "no_parking_brake_data");
    }
    if !any_present(&samples, |s| s.parking_brake) {
        return skip(rule, "no_parking_brake_data");
    }
    // (v3.29.0 #8) El requisito es "freno de parking aplicado AL LLEGAR",
    // no "mantenido el 100% de la fase". Flujo real: el piloto pone el
    // freno, apaga motores y luego pone calzos (chocks); tras los calzos,
    // SOLTAR el freno es legítimo. Basta con que el freno haya estado
    // puesto en CUALQUIER momento de la llegada para dar el pase. Antes,
    // el % sobre toda la fase fallaba en falso al soltar tras calzos.
    let any_set = samples.iter().any(|s| s.parking_brake == Some(true));
    let pct = pct_bool_true(&samples, |s| s.parking_brake).unwrap_or(0.0);
    let evidence = json!({
        "parking_brake_applied_at_arrival": any_set,
        "parking_brake_set_pct": (pct * 100.0) as i32
    });
    if any_set {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_arrived_engines_off(_ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    // El cierre del vuelo en flight_log se dispara cuando engines_off,
    // así que si llegamos a "arrived" implícitamente los motores se
    // apagaron. Pass (1/1) — la prueba más fuerte es que el row exista.
    pass(rule, json!({ "reason": "flight_finished_implies_engines_off" }))
}

fn eval_arrived_flaps_up(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_arrived(ctx);
    if samples.is_empty() {
        if let Some(last) = ctx.track.last() {
            if let Some(f) = last.flaps_pct {
                let evidence = json!({ "flaps_pct_at_end": f });
                return if f <= 5 { pass(rule, evidence) } else { fail(rule, "fail", 0, evidence) };
            }
        }
        return skip(rule, "no_flaps_data");
    }
    if !any_present(&samples, |s| s.flaps_pct) {
        return skip(rule, "no_flaps_data");
    }
    let last_flaps = samples.last().and_then(|s| s.flaps_pct).unwrap_or(0);
    let evidence = json!({ "flaps_pct_at_arrived": last_flaps });
    if last_flaps <= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}

fn eval_arrived_spoilers_stowed(ctx: &FlightContext, rule: &Rule) -> ScoreItem {
    let samples = samples_arrived(ctx);
    if samples.is_empty() {
        if let Some(last) = ctx.track.last() {
            if let Some(s) = last.spoilers_pct {
                let evidence = json!({ "spoilers_pct_at_end": s });
                return if s <= 5 { pass(rule, evidence) } else { fail(rule, "fail", 0, evidence) };
            }
        }
        return skip(rule, "no_spoilers_data");
    }
    if !any_present(&samples, |s| s.spoilers_pct) {
        return skip(rule, "no_spoilers_data");
    }
    let last_sp = samples.last().and_then(|s| s.spoilers_pct).unwrap_or(0);
    let evidence = json!({ "spoilers_pct_at_arrived": last_sp });
    if last_sp <= 5 {
        pass(rule, evidence)
    } else {
        fail(rule, "fail", 0, evidence)
    }
}
