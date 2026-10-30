//! (24/10) Tarjeta de aeropuerto del FlightBook (clic en un aeropuerto del
//! mapa): despegues y aterrizajes, pasajeros que subieron y bajaron, dinero
//! gastado ahí (servicios de GSX ya repartidos por vuelo + tasas de
//! aterrizaje), aterrizajes (mejor y media), tus aviones aparcados ahí, gate
//! más usado, ruta más volada y última visita. La foto viene de Wikipedia y se
//! guarda en caché local.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::airline_economy::{classify, landing_fee, normalize_reg};
use crate::flight_log::FlightLogEntry;

/// Dinero gastado en el aeropuerto por un grupo de servicios.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MoneyRow {
    /// catering | boarding | ground | water | bus | fuel | deice | other | landing
    pub group: String,
    pub usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AirportCard {
    pub icao: String,
    pub name: Option<String>,
    pub municipality: Option<String>,
    pub iso_country: Option<String>,
    /// (26/10) Para el foco del mapa y la foto satélite de respaldo.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub departures: i64,
    pub arrivals: i64,
    pub pax_boarded: i64,
    pub pax_disembarked: i64,
    pub money: Vec<MoneyRow>,
    pub money_total: f64,
    /// fpm del aterrizaje más suave ahí (negativo) y la media.
    pub best_fpm: Option<i64>,
    pub avg_fpm: Option<i64>,
    pub landings_measured: i64,
    /// Hora del mejor aterrizaje (para encontrar su clip).
    pub best_landing_at: Option<String>,
    /// Tus aviones aparcados ahí ahora (matrícula tal como la usas).
    pub parked: Vec<String>,
    pub top_gate: Option<String>,
    pub top_gate_count: i64,
    pub top_route: Option<String>,
    pub top_route_count: i64,
    pub last_visit_at: Option<String>,
    pub last_visit_label: Option<String>,
}

/// Grupo de un servicio de GSX (por tipo de recibo y descripción).
fn service_group(kind: &str, service: &str) -> &'static str {
    let s = service.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|x| s.contains(x));
    match kind {
        "catering" => "catering",
        "fuel" => "fuel",
        "bus" => "bus",
        _ if has(&["boarding", "deboard", "disembark"]) => "boarding",
        _ if has(&["water", "lavatory"]) => "water",
        _ if has(&["de-ic", "deic", "anti-ic"]) => "deice",
        _ if has(&["pushback", "jetway", "stairs", "gpu", "tow"]) => "ground",
        _ => "other",
    }
}

const GROUP_ORDER: [&str; 9] = ["catering", "boarding", "ground", "water", "bus", "deice", "fuel", "other", "landing"];

/// Agrega los vuelos (completados) y las líneas de GSX que tocan `icao`.
/// `pax`: pasajeros del P&L por vuelo (estimados si no se registraron);
/// `gsx`: (tipo, servicio, USD) de las líneas cobradas en ese aeropuerto.
fn tally(icao: &str, entries: &[FlightLogEntry], pax: &HashMap<i64, i64>, gsx: &[(String, String, f64)]) -> AirportCard {
    let is = |x: &Option<String>| x.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(icao));
    let mut card = AirportCard { icao: icao.to_string(), ..Default::default() };
    let mut money: HashMap<&'static str, f64> = HashMap::new();
    let mut gates: HashMap<String, i64> = HashMap::new();
    let mut routes: HashMap<String, (i64, String)> = HashMap::new();
    let mut fpms: Vec<(i64, String)> = Vec::new();
    let mut last: Option<(String, String)> = None;
    for e in entries {
        let dep = is(&e.origin_icao);
        let arr = is(&e.destination_icao);
        if !dep && !arr {
            continue;
        }
        let when = if arr { e.ended_at.clone().unwrap_or_else(|| e.started_at.clone()) } else { e.started_at.clone() };
        if dep {
            card.departures += 1;
            card.pax_boarded += pax.get(&e.id).copied().unwrap_or(0);
            if let Some(g) = e.departure_gate.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
                *gates.entry(g.to_string()).or_default() += 1;
            }
        }
        if arr {
            card.arrivals += 1;
            card.pax_disembarked += pax.get(&e.id).copied().unwrap_or(0);
            if let Some(g) = e.arrival_gate.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
                *gates.entry(g.to_string()).or_default() += 1;
            }
            // Vuelos de antes de v3.10 guardaron el fpm en positivo (como
            // el FlightBook: siempre −|fpm|).
            if let Some(f) = e.landing_fpm.filter(|f| *f != 0) {
                fpms.push((-f.abs(), when.clone()));
            }
            // Tasa de aterrizaje del vuelo (la que cobra Finanzas).
            if pax.contains_key(&e.id) {
                let (size, _) = classify(e.aircraft_model.as_deref().or(e.aircraft_atc_type.as_deref()));
                *money.entry("landing").or_default() += landing_fee(size);
            }
        }
        if let (Some(o), Some(d)) = (e.origin_icao.as_deref(), e.destination_icao.as_deref()) {
            let r = routes.entry(format!("{}→{}", o.to_uppercase(), d.to_uppercase())).or_insert((0, String::new()));
            r.0 += 1;
            if when > r.1 {
                r.1 = when.clone();
            }
        }
        if last.as_ref().map_or(true, |(t, _)| when > *t) {
            let label = e
                .callsign
                .clone()
                .or_else(|| e.flight_number.clone())
                .or_else(|| e.aircraft_registration.clone())
                .unwrap_or_default();
            last = Some((when, label));
        }
    }
    for (kind, service, usd) in gsx {
        *money.entry(service_group(kind, service)).or_default() += usd;
    }
    card.money = GROUP_ORDER
        .iter()
        .filter_map(|g| money.get(g).filter(|v| **v > 0.0).map(|v| MoneyRow { group: (*g).into(), usd: v.round() }))
        .collect();
    card.money_total = card.money.iter().map(|m| m.usd).sum();
    if let Some((f, at)) = fpms.iter().max_by_key(|(f, _)| *f) {
        card.best_fpm = Some(*f);
        card.best_landing_at = Some(at.clone());
        card.avg_fpm = Some((fpms.iter().map(|(f, _)| *f as f64).sum::<f64>() / fpms.len() as f64).round() as i64);
        card.landings_measured = fpms.len() as i64;
    }
    if let Some((g, n)) = gates.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))) {
        card.top_gate = Some(g);
        card.top_gate_count = n;
    }
    if let Some((r, (n, _))) = routes.into_iter().max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| a.1 .1.cmp(&b.1 .1))) {
        card.top_route = Some(r);
        card.top_route_count = n;
    }
    if let Some((at, label)) = last {
        card.last_visit_at = Some(at);
        card.last_visit_label = Some(label).filter(|l| !l.is_empty());
    }
    card
}

/// La tarjeta de `icao` (vuelos completados del FlightBook). (26/10) Con
/// `only` (filtro de aerolínea del FlightBook) sólo cuentan esos vuelos: sus
/// cobros de GSX, sus aterrizajes y los aviones aparcados que volaron con ella.
pub async fn card(pool: &SqlitePool, icao: &str, only: Option<&HashSet<i64>>) -> Result<AirportCard> {
    let icao = icao.trim().to_ascii_uppercase();
    let mut entries = crate::flight_log::list_entries(pool).await?;
    if let Some(ids) = only {
        entries.retain(|e| ids.contains(&e.id));
    }
    let pax: HashMap<i64, i64> = sqlx::query_as::<_, (i64, Option<i64>)>("SELECT flight_id, pax FROM flight_pnl")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, p)| (id, p.unwrap_or(0)))
        .collect();
    let gsx: Vec<(String, String, f64)> = sqlx::query_as::<_, (i64, String, String, f64)>(
        "SELECT flight_id, kind, service, amount_usd FROM gsx_charge WHERE airport = ?1 AND flight_id IS NOT NULL",
    )
    .bind(&icao)
    .fetch_all(pool)
    .await?
    .into_iter()
    .filter(|(id, ..)| only.map_or(true, |ids| ids.contains(id)))
    .map(|(_, kind, service, usd)| (kind, service, usd))
    .collect();
    let mut card = tally(&icao, &entries, &pax, &gsx);
    if let Some(a) = crate::airports::lookup_airports(pool, std::slice::from_ref(&icao)).await?.into_iter().next() {
        card.name = Some(a.name);
        card.municipality = a.municipality;
        card.iso_country = a.iso_country;
    }
    if let Some((lat, lon)) = sqlx::query_as::<_, (f64, f64)>("SELECT latitude, longitude FROM airports WHERE icao = ?1")
        .bind(&icao)
        .fetch_optional(pool)
        .await?
    {
        card.latitude = Some(lat);
        card.longitude = Some(lon);
    }
    // Tus aviones aparcados ahí (con la matrícula tal como la escribes).
    let shown: HashMap<String, String> = entries
        .iter()
        .filter_map(|e| e.aircraft_registration.as_deref())
        .map(|r| (normalize_reg(r), r.trim().to_string()))
        .collect();
    let mine: HashSet<&String> = shown.keys().collect();
    if let Ok(locs) = crate::ferry::locations(pool).await {
        let mut parked: Vec<String> = locs
            .into_iter()
            .filter(|(reg, l)| {
                mine.contains(reg) && l.icao.as_deref() == Some(icao.as_str()) && l.ferry.is_none() && !l.in_flight
            })
            .map(|(reg, _)| shown.get(&reg).cloned().unwrap_or(reg))
            .collect();
        parked.sort();
        card.parked = parked;
    }
    Ok(card)
}

/// Foto del aeropuerto (Wikipedia) en caché: `<dir>/<ICAO>.jpg`. Sin foto
/// deja `<ICAO>.none` y no vuelve a buscar en 30 días.
pub async fn photo(http: &reqwest::Client, dir: &Path, icao: &str, name: Option<&str>) -> Result<Option<PathBuf>> {
    let icao = icao.trim().to_ascii_uppercase();
    if icao.len() != 4 || !icao.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Ok(None);
    }
    let file = dir.join(format!("{icao}.jpg"));
    if file.is_file() {
        return Ok(Some(file));
    }
    let none = dir.join(format!("{icao}.none"));
    let fresh_none = std::fs::metadata(&none)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_secs() < 30 * 86_400));
    if fresh_none {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    match crate::airport_thumbs::fetch_airport_photo(http, &icao, name.unwrap_or_default(), 500).await? {
        Some(bytes) => {
            std::fs::write(&file, bytes)?;
            Ok(Some(file))
        }
        None => {
            std::fs::write(&none, b"")?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight(id: i64, from: &str, to: &str, at: &str) -> FlightLogEntry {
        FlightLogEntry {
            id,
            started_at: at.into(),
            ended_at: Some(at.replace("T10", "T13")),
            origin_icao: Some(from.into()),
            destination_icao: Some(to.into()),
            aircraft_model: Some("B738".into()),
            status: "completed".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_card_counts_only_what_happened_at_that_airport() {
        let mut a = flight(1, "CYYC", "CYYZ", "2026-10-20T10:00:00Z");
        a.departure_gate = Some("17".into());
        a.callsign = Some("WJA168".into());
        let mut b = flight(2, "CYYZ", "CYYC", "2026-10-21T10:00:00Z");
        b.arrival_gate = Some("17".into());
        b.landing_fpm = Some(-212);
        b.callsign = Some("WJA169".into());
        let mut c = flight(3, "CYVR", "CYYC", "2026-10-22T10:00:00Z");
        c.landing_fpm = Some(87); // guardado en positivo por una versión vieja
        c.arrival_gate = Some("22".into());
        let d = flight(4, "KJFK", "KBOS", "2026-10-23T10:00:00Z"); // no toca CYYC
        let e = flight(5, "CYYC", "CYYZ", "2026-10-19T10:00:00Z");
        let pax = HashMap::from([(1, 150), (2, 140), (3, 120), (4, 100), (5, 160)]);
        let gsx = vec![
            ("catering".to_string(), "Light snack".to_string(), 245.0),
            ("handling".to_string(), "Passenger boarding".to_string(), 800.0),
            ("handling".to_string(), "Pushback service (75 t MTOW)".to_string(), 420.0),
            ("handling".to_string(), "Jetway connection fee".to_string(), 100.0),
            ("fuel".to_string(), "Jet-A1 fuel".to_string(), 312.0),
        ];
        let card = tally("CYYC", &[a, b, c, d, e], &pax, &gsx);
        assert_eq!((card.departures, card.arrivals), (2, 2));
        assert_eq!((card.pax_boarded, card.pax_disembarked), (310, 260));
        assert_eq!((card.best_fpm, card.avg_fpm, card.landings_measured), (Some(-87), Some(-150), 2));
        assert_eq!((card.top_gate.as_deref(), card.top_gate_count), (Some("17"), 2));
        assert_eq!((card.top_route.as_deref(), card.top_route_count), (Some("CYYC→CYYZ"), 2));
        assert_eq!(card.last_visit_at.as_deref(), Some("2026-10-22T13:00:00Z"));
        let groups: Vec<(&str, f64)> = card.money.iter().map(|m| (m.group.as_str(), m.usd)).collect();
        assert_eq!(
            groups,
            vec![("catering", 245.0), ("boarding", 800.0), ("ground", 520.0), ("fuel", 312.0), ("landing", 1300.0)]
        );
        assert_eq!(card.money_total, 3177.0);
    }

    /// Con una COPIA real (no corre por defecto):
    ///   CARD_DB=<copia .db> CARD_ICAO=CYYC cargo test --lib airport_card_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn airport_card_real() {
        let (Ok(path), Ok(icao)) = (std::env::var("CARD_DB"), std::env::var("CARD_ICAO")) else { return };
        let pool = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for i in icao.split(',') {
            println!("{:#?}", card(&pool, i, None).await.unwrap());
        }
    }

    /// Con red (no corre por defecto):
    ///   PHOTO_ICAOS="KJFK=John F. Kennedy International Airport,CYYC" cargo test --lib airport_photo_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn airport_photo_real() {
        let Ok(icaos) = std::env::var("PHOTO_ICAOS") else { return };
        let dir = std::env::temp_dir().join(format!("simfleet-airport-photos-{}", std::process::id()));
        // Como la app: Wikipedia rechaza peticiones sin User-Agent.
        let http = reqwest::Client::builder()
            .user_agent("SimFleet/3.0 (+https://github.com/Daniiel18/msfs-addons-browser)")
            .build()
            .unwrap();
        for item in icaos.split(',') {
            // "KJFK" o "KJFK=John F. Kennedy International Airport" (el ICAO
            // solo a veces es una página de desambiguación).
            let (icao, name) = item.split_once('=').map_or((item, None), |(i, n)| (i, Some(n)));
            let p = photo(&http, &dir, icao, name).await.unwrap();
            let kb = p.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len() / 1024);
            println!("{icao}: {kb:?} KB");
        }
        // PHOTO_KEEP=1 deja las fotos para mirarlas (se borra a mano).
        if std::env::var("PHOTO_KEEP").is_ok() {
            println!("fotos en {}", dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_landing_fee_needs_a_flight_with_finances() {
        let a = flight(1, "KJFK", "CYYC", "2026-10-20T10:00:00Z");
        let card = tally("CYYC", &[a], &HashMap::new(), &[]);
        assert!(card.money.is_empty());
        assert_eq!(card.money_total, 0.0);
    }
}
