//! (24/10) Precios reales de mantenimiento (`mx-catalogs/prices.json`).
//!
//! Antes todo salía de tres precios por ATA (900 / 7.500 / 60.000 USD) y 115
//! USD/h fijos: una luz de mapa costaba 58 USD de "mano de obra" inventada.
//! Ahora, para Fenix/PMDG TechLog y el Tech Log propio:
//! - Pieza: precio de INTERCAMBIO / unidad revisada (lo que paga una
//!   aerolínea por un rotable) + su mano de obra de cambio (`mh`).
//! - Reparar: taller (fracción `rep` del precio) + la misma mano de obra.
//! - Mano de obra: tarifa de la región del aeropuerto donde está el avión.
//! - Checks: horas-hombre × tarifa × (1 + material).
//! - Consumibles: por unidad servida (aceite por cuarto, Skydrol…).
//! - Defectos sin pieza ("findings"): horas del MEL × tarifa + material.
//!
//! Cada cifra lleva su fuente en el JSON ("est" = estimación sin precio
//! público). Widebody: las piezas narrowbody × `widebodyFactor`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;
use sqlx::SqlitePool;

/// Tarifa de mano de obra sin ubicación conocida (USD/h).
pub const DEFAULT_LABOR: f64 = 110.0;

/// Flota a efectos de precio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fleet {
    A320,
    B737,
    Widebody,
    Other,
}

impl Fleet {
    /// De la familia del Tech Log propio (`mx::catalog::Family::id`).
    pub fn from_family(id: &str) -> Fleet {
        match id {
            "a320" => Fleet::A320,
            "b737" => Fleet::B737,
            "b777" | "awide" => Fleet::Widebody,
            _ => Fleet::Other,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Part {
    usd: f64,
    b737: Option<f64>,
    /// Precio propio de fuselaje ancho (piezas sólo del 777).
    wide: Option<f64>,
    rep: Option<f64>,
    mh: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cons {
    per_unit: Option<f64>,
    flat: Option<f64>,
    #[serde(default)]
    mh: f64,
}

#[derive(Debug, Deserialize)]
struct Labor {
    default: f64,
    regions: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
struct Defaults {
    rep: f64,
    mh: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    labor: Labor,
    widebody_factor: f64,
    defaults: Defaults,
    checks: HashMap<String, Value>,
    consumables: HashMap<String, Cons>,
    finding_material: HashMap<String, Value>,
    parts: HashMap<String, Part>,
}

fn data() -> &'static File {
    static DATA: OnceLock<File> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../mx-catalogs/prices.json")).expect("mx-catalogs/prices.json válido")
    })
}

/// Precio de una pieza para una flota.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartPrice {
    /// Intercambio / unidad revisada (USD).
    pub usd: f64,
    /// Fracción del precio que cuesta repararla en taller.
    pub repair: f64,
    /// Horas-hombre de quitar y poner.
    pub mh: f64,
}

pub fn part(key: &str, fleet: Fleet) -> Option<PartPrice> {
    let d = data();
    let p = d.parts.get(key)?;
    let usd = match fleet {
        Fleet::B737 => p.b737.unwrap_or(p.usd),
        Fleet::Widebody => p.wide.unwrap_or(p.usd * d.widebody_factor),
        Fleet::A320 | Fleet::Other => p.usd,
    };
    Some(PartPrice {
        usd,
        repair: p.rep.unwrap_or(d.defaults.rep),
        mh: p.mh.unwrap_or(d.defaults.mh),
    })
}

/// Cambiar la pieza: intercambio + mano de obra. `None` si no está en la tabla.
pub fn replace_cost(key: &str, fleet: Fleet, rate: f64) -> Option<f64> {
    part(key, fleet).map(|p| (p.usd + p.mh * rate).round())
}

/// Repararla: taller + mano de obra. `None` si no está en la tabla.
pub fn repair_cost(key: &str, fleet: Fleet, rate: f64) -> Option<f64> {
    part(key, fleet).map(|p| (p.usd * p.repair + p.mh * rate).round())
}

/// Pieza sin precio en la tabla: el precio por ATA de antes + mano de obra
/// por defecto (red de seguridad para catálogos futuros).
pub fn fallback_part(base: f64, repair: bool, rate: f64) -> f64 {
    let d = data();
    let part = if repair { base * d.defaults.rep } else { base };
    (part + d.defaults.mh * rate).round()
}

/// Reponer un consumible (`key` sin posición) en la cantidad servida.
pub fn consumable_cost(key: &str, amount: f64, rate: f64) -> f64 {
    let Some(c) = data().consumables.get(key) else {
        return (200.0 + rate * 0.5).round();
    };
    let material = match (c.per_unit, c.flat) {
        (Some(u), _) => {
            if amount <= 0.0 {
                return 0.0;
            }
            u * amount
        }
        (None, Some(f)) => f,
        (None, None) => 0.0,
    };
    if material <= 0.0 && c.mh <= 0.0 {
        return 0.0;
    }
    (material + c.mh * rate).round()
}

/// Un check: horas-hombre × tarifa × (1 + material). (25/10) En un fuselaje
/// ancho (777) las horas por el mismo factor que sus piezas.
pub fn check_cost(key: &str, fleet: Fleet, rate: f64) -> f64 {
    let d = data();
    let c = d.checks.get(key);
    let f = |k: &str| c.and_then(|v| v.get(k)).and_then(|x| x.as_f64());
    let size = if fleet == Fleet::Widebody { d.widebody_factor } else { 1.0 };
    let mh = f("mh").unwrap_or(15.0) * size;
    let material = f("material").unwrap_or(0.36);
    (mh * rate * (1.0 + material)).round()
}

/// Material de un defecto sin pieza del catálogo (`FINDING:<clave>` de TechLog).
pub fn finding_material(key: &str) -> f64 {
    let m = &data().finding_material;
    m.get(key)
        .or_else(|| m.get("default"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

/// Defecto rectificado: horas del MEL × tarifa + material.
pub fn defect_cost(hours: f64, material: f64, rate: f64) -> f64 {
    (hours * rate + material).round()
}

/// Región de un aeropuerto por la primera letra de su ICAO.
pub fn region_of(icao: &str) -> &'static str {
    match icao.trim().chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('K' | 'C' | 'P') => "NA",
        Some('E' | 'L' | 'B') => "EU",
        Some('Y' | 'N' | 'A') => "OC",
        Some('R' | 'V' | 'W' | 'Z' | 'U') => "AS",
        Some('O') => "ME",
        Some('M' | 'T' | 'S') => "LA",
        Some('D' | 'F' | 'G' | 'H') => "AF",
        _ => "",
    }
}

/// Tarifa (USD/h) del aeropuerto `icao`.
pub fn rate_for_icao(icao: Option<&str>) -> f64 {
    let l = &data().labor;
    icao.and_then(|i| l.regions.get(region_of(i)))
        .copied()
        .unwrap_or(l.default)
}

/// Tarifa de mano de obra donde está ahora el avión `registration`.
pub async fn labor_rate(pool: &SqlitePool, registration: &str) -> f64 {
    let icao = crate::ferry::location(pool, registration).await.ok().and_then(|l| l.icao);
    rate_for_icao(icao.as_deref())
}

/// (24/10) Una sola vez: los cobros ya hechos de TechLog y del Tech Log
/// propio pasan a precios reales (y se recalcula la economía).
pub async fn reprice_history_once(pool: &SqlitePool) {
    // v3: + defectos que TechLog ya no tiene (por título) y cobros dobles de
    // "Servicing required" (nunca salió una versión con v2).
    const KEY: &str = "maint_prices_v3_at";
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if done.is_some() {
        return;
    }
    let mx = crate::mx::store::reprice_history(pool).await;
    let tl = crate::techlog::reprice_history(pool).await;
    match (&mx, &tl) {
        (Ok(a), Ok(b)) => {
            tracing::info!(target: "economy", "precios reales: {a} cobro(s) del Tech Log propio y {b} de TechLog recalculados");
            let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))")
                .bind(KEY)
                .bind(chrono::Utc::now().to_rfc3339())
                .execute(pool)
                .await;
            if a + b > 0 {
                if let Err(e) = crate::airline_economy::recompute(pool).await {
                    tracing::warn!(target: "economy", "recalcular tras precios reales: {e:#}");
                }
            }
        }
        _ => tracing::warn!(target: "economy", "precios reales: no se pudo recalcular el historial ({mx:?} / {tl:?})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_loads_and_has_its_anchors() {
        // Cifras con fuente pública.
        assert_eq!(part("GEN", Fleet::A320).unwrap().usd, 44_000.0, "IDG revisado (AJW)");
        assert_eq!(part("BATTERY", Fleet::B737).unwrap().usd, 20_880.0, "Saft 4579 (SkyGeek)");
        assert_eq!(part("BRAKE", Fleet::B737).unwrap().usd, 48_000.0, "el 737 tiene su precio");
        assert_eq!(part("BRAKE", Fleet::Widebody).unwrap().usd, 52_000.0 * 1.8);
        assert_eq!(part("AIMS", Fleet::Widebody).unwrap().usd, 250_000.0, "pieza sólo del 777: su precio");
        assert!(part("NO_EXISTE", Fleet::A320).is_none());
    }

    #[test]
    fn a_job_is_the_part_plus_its_labour_where_the_aircraft_is() {
        // Neumático principal: 2.600 + 7,5 h.
        assert_eq!(replace_cost("TYRE_MAIN", Fleet::A320, 110.0), Some(3_425.0));
        assert_eq!(repair_cost("TYRE_MAIN", Fleet::A320, 85.0), Some((1_300.0f64 + 7.5 * 85.0).round()));
        assert_eq!(rate_for_icao(Some("KMIA")), 110.0);
        assert_eq!(rate_for_icao(Some("MDSD")), 85.0, "Caribe");
        assert_eq!(rate_for_icao(Some("SKBO")), 85.0, "Sudamérica");
        assert_eq!(rate_for_icao(Some("RJTT")), 95.0);
        assert_eq!(rate_for_icao(Some("OMDB")), 95.0);
        assert_eq!(rate_for_icao(Some("EHAM")), 110.0);
        assert_eq!(rate_for_icao(None), DEFAULT_LABOR);
    }

    #[test]
    fn consumables_checks_and_findings() {
        // Aceite: 23,31 USD/qt + 0,25 h.
        assert_eq!(consumable_cost("ENG_OIL", 4.0, 110.0), (4.0f64 * 23.31 + 27.5).round());
        assert_eq!(consumable_cost("ENG_OIL", 0.0, 110.0), 0.0, "nada servido, nada cobrado");
        // Catering, agua y baños hechos SIN GSX (con GSX se cobra su recibo).
        assert_eq!(consumable_cost("CATERING", 100.0, 110.0), 320.0);
        assert_eq!(consumable_cost("WASTE", 100.0, 110.0), 70.0);
        // A-check: 165 h × 110 × 1,36.
        assert_eq!(check_cost("A", Fleet::A320, 110.0), (165.0f64 * 110.0 * 1.36).round());
        assert_eq!(check_cost("A", Fleet::B737, 110.0), check_cost("A", Fleet::A320, 110.0));
        assert_eq!(check_cost("A", Fleet::Widebody, 110.0), (165.0f64 * 1.8 * 110.0 * 1.36).round());
        assert!(check_cost("C", Fleet::A320, 110.0) > 900_000.0);
        // La luz de mapa: 0,5 h + la lámpara (antes 58 USD de mano de obra).
        assert_eq!(defect_cost(0.5, finding_material("MAP_LIGHT"), 85.0), 193.0);
        assert_eq!(finding_material("OTRA_COSA"), 100.0);
    }

    /// Con una COPIA real (no corre por defecto):
    ///   REPRICE_DB=<copia .db> cargo test --lib reprice_real_copy -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn reprice_real_copy() {
        let Ok(path) = std::env::var("REPRICE_DB") else { return };
        let pool = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let q = "SELECT registration, component, cost FROM maintenance_log WHERE component LIKE 'mx:%' OR component LIKE 'techlog:%' ORDER BY id";
        let before: Vec<(String, String, f64)> = sqlx::query_as(q).fetch_all(&pool).await.unwrap();
        reprice_history_once(&pool).await;
        let after: Vec<(String, String, f64)> = sqlx::query_as(q).fetch_all(&pool).await.unwrap();
        for (b, a) in before.iter().zip(&after) {
            println!("{:8} {:45} {:>8.0} → {:>8.0}", b.0, b.1, b.2, a.2);
        }
        reprice_history_once(&pool).await;
        let again: Vec<(String, String, f64)> = sqlx::query_as(q).fetch_all(&pool).await.unwrap();
        assert_eq!(after, again, "sólo una vez");
    }

    /// Todas las piezas de los catálogos A320 y 737 (propios y de TechLog
    /// instalados) tienen precio.
    #[test]
    fn every_a320_and_737_part_has_a_price() {
        for raw in [include_str!("../mx-catalogs/a320.json"), include_str!("../mx-catalogs/b737.json")] {
            let v: Value = serde_json::from_str(raw).unwrap();
            let missing: Vec<&str> = v["components"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|c| c["key"].as_str())
                .filter(|k| part(k, Fleet::A320).is_none())
                .collect();
            assert!(missing.is_empty(), "sin precio: {missing:?}");
        }
    }
}
