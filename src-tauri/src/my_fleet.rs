//! (v8.3 · Etapa 6.2d) "Tus aviones": las matrículas del usuario.
//!
//! Son las que voló (FlightBook), las de Fenix/PMDG TechLog y las de sus
//! pinturas instaladas en Community (`atc_id` de cada `[FLTSIM.N]` /
//! `livery.cfg`), sin los paquetes de tráfico IA (FSLTL, AIG…), que traen
//! cientos de matrículas reales que no son del usuario. Estos aviones vuelan
//! solos con sus vuelos reales (sin tocar nunca su mantenimiento), salen
//! como "Tuyo" en la flota y pagan costo fijo en su aerolínea.

use crate::airline_economy::normalize_reg;
use anyhow::Result;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyAircraft {
    /// Matrícula como la trae su fuente ("N657JB", "C-GORP").
    pub registration: String,
    /// De dónde sale: "flights" | "techlog" | "livery".
    pub sources: Vec<String>,
    /// Modelo o título (para estimar tamaño).
    pub model: Option<String>,
}

/// Paquetes de tráfico IA: sus pinturas no son aviones del usuario.
fn is_traffic_package(folder: &str) -> bool {
    let f = folder.to_ascii_lowercase();
    ["traffic", "fsltl", "aig-", "aig_", "-ai-", "aiaircraft", "ai-aircraft", "simtraffic"]
        .iter()
        .any(|k| f.contains(k))
        || f.starts_with("aig")
}

/// Matrículas genéricas de MSFS / repaints que no identifican un avión.
fn is_placeholder(reg: &str) -> bool {
    matches!(reg, "ASXGS" | "ASOBO" | "N12345" | "NXXXX" | "XXXXX" | "TEST" | "REG" | "DEFAULT") || reg.len() < 3
}

type Cache = Mutex<Option<(Instant, HashMap<String, MyAircraft>)>>;
static CACHE: OnceLock<Cache> = OnceLock::new();

/// Invalida la caché (p. ej. tras instalar pinturas o cerrar un vuelo).
pub fn invalidate() {
    if let Ok(mut g) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;
    }
}

fn add(out: &mut HashMap<String, MyAircraft>, reg: &str, source: &str, model: Option<String>) {
    let n = normalize_reg(reg);
    if is_placeholder(&n) {
        return;
    }
    let e = out.entry(n).or_insert_with(|| MyAircraft {
        registration: reg.trim().to_ascii_uppercase(),
        ..Default::default()
    });
    if !e.sources.iter().any(|s| s == source) {
        e.sources.push(source.to_string());
    }
    if e.model.is_none() {
        e.model = model.filter(|m| !m.trim().is_empty());
    }
}

/// Todas las matrículas del usuario (normalizada → avión). Cacheado 10 min.
pub async fn my_aircraft(pool: &SqlitePool) -> Result<HashMap<String, MyAircraft>> {
    let cell = CACHE.get_or_init(|| Mutex::new(None));
    // En tests cada prueba usa su propia base: sin caché compartida.
    if !cfg!(test) {
        if let Ok(g) = cell.lock() {
            if let Some((t, m)) = g.as_ref() {
                if t.elapsed() < CACHE_TTL {
                    return Ok(m.clone());
                }
            }
        }
    }
    let mut out: HashMap<String, MyAircraft> = HashMap::new();
    // 1) Vuelos del FlightBook.
    for r in sqlx::query(
        "SELECT aircraft_registration AS reg, COALESCE(aircraft_model, aircraft_atc_type) AS model
           FROM flight_log WHERE aircraft_registration IS NOT NULL AND TRIM(aircraft_registration) <> ''
          ORDER BY started_at DESC",
    )
    .fetch_all(pool)
    .await?
    {
        add(&mut out, &r.get::<String, _>("reg"), "flights", r.get("model"));
    }
    // 2) Fenix / PMDG TechLog.
    for tl in crate::techlog::airframe_list().await {
        let reg = crate::techlog::display_reg(&tl.registration);
        add(&mut out, &reg, "techlog", Some(tl.variant.clone()));
    }
    // 3) Pinturas instaladas (sin tráfico IA).
    let packs: Vec<(String, String)> = sqlx::query(
        "SELECT folder_name, install_path FROM community_packages
          WHERE content_type IN ('AIRCRAFT', 'LIVERY', 'AIRCRAFT_LIVERY')",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.get("folder_name"), r.get("install_path")))
    .filter(|(folder, _): &(String, String)| !is_traffic_package(folder))
    .collect();
    let liveries = tokio::task::spawn_blocking(move || {
        packs
            .into_iter()
            .flat_map(|(_, path)| crate::community_scanner::list_pack_liveries(&path))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    for l in liveries {
        if let Some(reg) = l.registration.as_deref().filter(|r| !r.trim().is_empty()) {
            add(&mut out, reg, "livery", Some(l.title.clone()));
        }
    }
    if let Ok(mut g) = cell.lock() {
        *g = Some((Instant::now(), out.clone()));
    }
    Ok(out)
}

/// icao24 (Mode-S) de los aviones del usuario que están en la base de aviones.
pub async fn my_icao24(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let mine = my_aircraft(pool).await?;
    if mine.is_empty() {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    for r in sqlx::query("SELECT icao24, registration FROM aircraft_db WHERE registration IS NOT NULL")
        .fetch_all(pool)
        .await?
    {
        let reg = normalize_reg(&r.get::<String, _>("registration"));
        if mine.contains_key(&reg) {
            out.insert(r.get::<String, _>("icao24"), reg);
        }
    }
    Ok(out)
}

/// Aviones del usuario agrupados por el operador que les da la base de
/// aviones (ICAO de aerolínea → matrícula normalizada → avión). Así un avión
/// de TechLog o de una pintura cuenta en la flota de su aerolínea aunque el
/// usuario nunca lo haya volado con ella.
pub async fn mine_by_operator(pool: &SqlitePool) -> Result<HashMap<String, HashMap<String, MyAircraft>>> {
    let mine = my_aircraft(pool).await?;
    let mut out: HashMap<String, HashMap<String, MyAircraft>> = HashMap::new();
    if mine.is_empty() {
        return Ok(out);
    }
    for r in sqlx::query(
        "SELECT registration, operator_icao FROM aircraft_db
          WHERE registration IS NOT NULL AND operator_icao IS NOT NULL AND operator_icao <> ''",
    )
    .fetch_all(pool)
    .await?
    {
        let reg = normalize_reg(&r.get::<String, _>("registration"));
        if let Some(a) = mine.get(&reg) {
            out.entry(r.get::<String, _>("operator_icao").to_ascii_uppercase())
                .or_default()
                .insert(reg, a.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_packages_and_placeholders_are_not_mine() {
        assert!(is_traffic_package("fsltl-traffic-base"));
        assert!(is_traffic_package("aig-aircraft-a320"));
        assert!(is_traffic_package("AIG_Liveries"));
        assert!(!is_traffic_package("fnx-aircraft-320"));
        assert!(!is_traffic_package("pmdg-livery-b738-wja"));
        assert!(is_placeholder("ASXGS"));
        assert!(!is_placeholder("N657JB"));
        let mut m = HashMap::new();
        add(&mut m, "N657-JB", "livery", Some("JetBlue".into()));
        add(&mut m, "n657jb", "flights", None);
        add(&mut m, "ASXGS", "livery", None);
        assert_eq!(m.len(), 1);
        assert_eq!(m["N657JB"].sources, vec!["livery", "flights"]);
        assert_eq!(m["N657JB"].registration, "N657-JB");
    }
}
