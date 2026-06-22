//! (v6.1) Mantenimiento por aeronave — fuente ÚNICA compartida por el Hangar
//! (silueta con zonas de desgaste) y por Finanzas (panel de servicio tipo EFB).
//!
//! El desgaste de cada componente se deriva de los VUELOS de esa matrícula
//! desde el último servicio (estilo EFB de PMDG, pero homogéneo para 737/777/
//! A320 y cualquier otro): cada vuelo desgasta; aterrizar gasta neumáticos y
//! frenos; aterrizar DURO gasta más; las horas gastan aceite/EGT. Hacer un
//! servicio (cambiar neumáticos, refill aceite, reset EGT…) guarda una fila en
//! `maintenance_log`: resetea ese componente y su coste entra en la economía
//! de la aerolínea (ver `airline_economy::recompute`).

use crate::airline_economy::{self, normalize_reg};
use crate::flight_log::{self, FlightLogEntry};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

/// Especificación de un componente: cuánto se desgasta y qué cuesta servirlo.
struct CompSpec {
    id: &'static str,
    /// Desgaste (%) por vuelo, por aterrizaje duro extra y por hora de vuelo.
    per_flight: f64,
    hard_extra: f64,
    per_hour: f64,
    /// Coste del servicio (USD).
    cost: f64,
    /// Zona de la silueta que ilumina (engines/gears/fuselage/nose/tail).
    zone: &'static str,
}

/// Componentes mantenibles (ids alineados con el EFB de PMDG). El frontend
/// pone las etiquetas (i18n) y la zona de la silueta.
const COMPONENTS: &[CompSpec] = &[
    CompSpec { id: "tires",        per_flight: 6.0, hard_extra: 8.0,  per_hour: 0.0, cost: 8_000.0,  zone: "gears" },
    CompSpec { id: "brakes",       per_flight: 5.0, hard_extra: 10.0, per_hour: 0.0, cost: 12_000.0, zone: "gears" },
    CompSpec { id: "engine_oil",   per_flight: 1.5, hard_extra: 0.0,  per_hour: 1.2, cost: 1_500.0,  zone: "engines" },
    CompSpec { id: "hydraulics",   per_flight: 1.0, hard_extra: 1.0,  per_hour: 0.8, cost: 2_500.0,  zone: "fuselage" },
    CompSpec { id: "fire_bottles", per_flight: 0.3, hard_extra: 0.0,  per_hour: 0.1, cost: 4_000.0,  zone: "engines" },
    CompSpec { id: "oxygen",       per_flight: 0.5, hard_extra: 0.0,  per_hour: 0.2, cost: 1_200.0,  zone: "fuselage" },
    CompSpec { id: "egt",          per_flight: 2.0, hard_extra: 0.0,  per_hour: 1.5, cost: 3_000.0,  zone: "engines" },
    CompSpec { id: "idg",          per_flight: 1.2, hard_extra: 0.0,  per_hour: 0.6, cost: 2_000.0,  zone: "engines" },
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintComponent {
    pub id: String,
    pub zone: String,
    /// Desgaste 0-100.
    pub wear_pct: f64,
    /// "ok" (<50) / "watch" (50-80) / "due" (>=80).
    pub status: String,
    pub action_cost: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AircraftMaint {
    pub registration: String,
    pub model: Option<String>,
    pub flights: i64,
    /// Peor componente — para ordenar la flota y pintar la zona roja.
    pub overall_wear: f64,
    pub components: Vec<MaintComponent>,
}

fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Último servicio por (matrícula, componente).
async fn last_services(pool: &SqlitePool) -> Result<HashMap<(String, String), DateTime<Utc>>> {
    let rows = sqlx::query(
        r#"SELECT registration, component, MAX(serviced_at) AS last
             FROM maintenance_log GROUP BY registration, component"#,
    )
    .fetch_all(pool)
    .await?;
    let mut map = HashMap::new();
    for r in rows {
        let reg: String = r.get("registration");
        let comp: String = r.get("component");
        let last: String = r.get("last");
        if let Some(dt) = parse_dt(&last) {
            map.insert((reg, comp), dt);
        }
    }
    Ok(map)
}

/// Calcula el desgaste de un componente para una matrícula desde su último
/// servicio (o desde siempre si nunca se sirvió).
fn wear_for(
    spec: &CompSpec,
    reg_flights: &[&FlightLogEntry],
    last_service: Option<&DateTime<Utc>>,
) -> f64 {
    let mut flights_since = 0.0;
    let mut hard_since = 0.0;
    let mut hours_since = 0.0;
    for f in reg_flights {
        if let Some(svc) = last_service {
            if let Some(started) = parse_dt(&f.started_at) {
                if started <= *svc {
                    continue; // anterior al servicio → no cuenta
                }
            }
        }
        flights_since += 1.0;
        if let Some(fpm) = f.landing_fpm {
            if fpm < -600 {
                hard_since += 1.0;
            }
        }
        if let Some(s) = f.flight_time_s {
            hours_since += (s as f64 / 3600.0).max(0.0);
        }
    }
    (spec.per_flight * flights_since + spec.hard_extra * hard_since + spec.per_hour * hours_since)
        .clamp(0.0, 100.0)
}

fn status_of(wear: f64) -> &'static str {
    if wear >= 80.0 {
        "due"
    } else if wear >= 50.0 {
        "watch"
    } else {
        "ok"
    }
}

/// Multiplicador del COSTE de servicio según el nivel de mantenimiento de la
/// aerolínea: premium usa repuestos/talleres mejores (más caro), básico más
/// barato. Así premium ≠ standard ≠ básico, como pidió el usuario.
fn level_cost_mult(level: i64) -> f64 {
    match level {
        0 => 0.75,
        2 => 1.45,
        _ => 1.0,
    }
}

/// Calcula los componentes (desgaste + coste) de una matrícula. `mult` escala
/// el coste de servicio por el nivel de mantenimiento.
fn components_for(
    reg: &str,
    reg_flights: &[&FlightLogEntry],
    services: &HashMap<(String, String), DateTime<Utc>>,
    mult: f64,
) -> (Vec<MaintComponent>, f64) {
    let mut comps = Vec::with_capacity(COMPONENTS.len());
    let mut overall = 0.0_f64;
    for spec in COMPONENTS {
        let last = services.get(&(reg.to_string(), spec.id.to_string()));
        let wear = wear_for(spec, reg_flights, last);
        overall = overall.max(wear);
        comps.push(MaintComponent {
            id: spec.id.to_string(),
            zone: spec.zone.to_string(),
            wear_pct: wear,
            status: status_of(wear).to_string(),
            action_cost: spec.cost * mult,
        });
    }
    (comps, overall)
}

/// Mantenimiento de TODA la flota de una aerolínea (matrículas que ha volado).
pub async fn fleet_maintenance(pool: &SqlitePool, airline_key: &str) -> Result<Vec<AircraftMaint>> {
    let flights = flight_log::list_entries(pool).await?;
    let learned = airline_economy::learn_airline_icaos(&flights);
    let services = last_services(pool).await?;
    // El nivel de mantenimiento de la aerolínea escala el coste de servicio.
    let mult = level_cost_mult(
        airline_economy::get_policy(pool, airline_key)
            .await
            .map(|p| p.maintenance_level)
            .unwrap_or(1),
    );

    // Agrupa los vuelos de ESTA aerolínea por matrícula.
    let mut by_reg: HashMap<String, Vec<&FlightLogEntry>> = HashMap::new();
    let mut model_of: HashMap<String, Option<String>> = HashMap::new();
    // Matrícula ORIGINAL (con guion) para mostrar — el match es por normalizada.
    let mut orig_of: HashMap<String, String> = HashMap::new();
    for f in &flights {
        let Some(canon) = airline_economy::canonical_airline(
            f.airline_icao.as_deref(),
            f.callsign.as_deref(),
            f.aircraft_airline.as_deref(),
            f.aircraft_title.as_deref(),
            &learned,
        ) else {
            continue;
        };
        if canon.key != airline_key {
            continue;
        }
        let Some(reg_raw) = f.aircraft_registration.as_deref() else {
            continue;
        };
        let reg = normalize_reg(reg_raw);
        if reg.is_empty() {
            continue;
        }
        by_reg.entry(reg.clone()).or_default().push(f);
        orig_of.entry(reg.clone()).or_insert_with(|| reg_raw.trim().to_string());
        model_of.entry(reg).or_insert_with(|| {
            f.aircraft_model
                .as_deref()
                .or(f.aircraft_atc_type.as_deref())
                .map(flight_log::normalize_model)
        });
    }

    let mut out = Vec::with_capacity(by_reg.len());
    for (reg, reg_flights) in by_reg {
        let (comps, overall) = components_for(&reg, &reg_flights, &services, mult);
        out.push(AircraftMaint {
            registration: orig_of.get(&reg).cloned().unwrap_or_else(|| reg.clone()),
            model: model_of.get(&reg).cloned().flatten(),
            flights: reg_flights.len() as i64,
            overall_wear: overall,
            components: comps,
        });
    }
    // Peor desgaste primero (los que necesitan atención arriba).
    out.sort_by(|a, b| b.overall_wear.partial_cmp(&a.overall_wear).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// Mantenimiento de UNA matrícula (la usa el Hangar para pintar la silueta con
/// la MISMA data que Finanzas). Sin filtrar por aerolínea.
pub async fn single_aircraft_maintenance(
    pool: &SqlitePool,
    registration: &str,
) -> Result<Option<AircraftMaint>> {
    let target = normalize_reg(registration);
    if target.is_empty() {
        return Ok(None);
    }
    let flights = flight_log::list_entries(pool).await?;
    let services = last_services(pool).await?;
    let reg_flights: Vec<&FlightLogEntry> = flights
        .iter()
        .filter(|f| {
            f.aircraft_registration
                .as_deref()
                .map(|r| normalize_reg(r) == target)
                .unwrap_or(false)
        })
        .collect();
    if reg_flights.is_empty() {
        return Ok(None);
    }
    let model = reg_flights[0]
        .aircraft_model
        .as_deref()
        .or(reg_flights[0].aircraft_atc_type.as_deref())
        .map(flight_log::normalize_model);
    // Coste a tarifa estándar (el Hangar sólo muestra desgaste; servir es en
    // Finanzas, donde se aplica el multiplicador del nivel).
    let (comps, overall) = components_for(&target, &reg_flights, &services, 1.0);
    let display_reg = reg_flights[0]
        .aircraft_registration
        .as_deref()
        .map(|r| r.trim().to_string())
        .unwrap_or(target);
    Ok(Some(AircraftMaint {
        registration: display_reg,
        model,
        flights: reg_flights.len() as i64,
        overall_wear: overall,
        components: comps,
    }))
}

/// Registra un servicio: resetea el componente (cuenta desgaste sólo de los
/// vuelos posteriores) y su coste (según el NIVEL de la aerolínea) entra en la
/// economía. `airline_key` permite cobrar el precio premium/standard/básico.
pub async fn service_aircraft(
    pool: &SqlitePool,
    registration: &str,
    component: &str,
    airline_key: &str,
) -> Result<()> {
    let reg = normalize_reg(registration);
    let mult = level_cost_mult(
        airline_economy::get_policy(pool, airline_key)
            .await
            .map(|p| p.maintenance_level)
            .unwrap_or(1),
    );
    let cost = COMPONENTS
        .iter()
        .find(|c| c.id == component)
        .map(|c| c.cost * mult)
        .unwrap_or(0.0);
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO maintenance_log (registration, component, cost, serviced_at)
           VALUES (?,?,?,?)"#,
    )
    .bind(&reg)
    .bind(component)
    .bind(cost)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Una entrada del historial de mantenimiento (servicios hechos por el jugador).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintRecord {
    pub component: String,
    pub cost: f64,
    pub serviced_at: String,
}

/// Historial de mantenimiento REAL de una matrícula (lo que se hizo en
/// Finanzas) — para que el Hangar (Mantenimiento/Historial/Documentos) muestre
/// lo mismo. Más reciente primero.
pub async fn maintenance_history(pool: &SqlitePool, registration: &str) -> Result<Vec<MaintRecord>> {
    let reg = normalize_reg(registration);
    let rows = sqlx::query(
        r#"SELECT component, cost, serviced_at
             FROM maintenance_log
            WHERE registration = ?
            ORDER BY serviced_at DESC"#,
    )
    .bind(&reg)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MaintRecord {
            component: r.get("component"),
            cost: r.get("cost"),
            serviced_at: r.get("serviced_at"),
        })
        .collect())
}

/// Coste de mantenimiento acumulado por matrícula — lo usa la economía para
/// sumar lo gastado en servicios a la aerolínea correspondiente.
pub async fn maintenance_cost_by_reg(pool: &SqlitePool) -> Result<HashMap<String, f64>> {
    let rows = sqlx::query(
        r#"SELECT registration, SUM(cost) AS total
             FROM maintenance_log GROUP BY registration"#,
    )
    .fetch_all(pool)
    .await?;
    let mut map = HashMap::new();
    for r in rows {
        let reg: String = r.get("registration");
        let total: f64 = r.get("total");
        map.insert(reg, total);
    }
    Ok(map)
}
