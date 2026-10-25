//! (v8.3 · Etapa 6.1) Ubicación de cada avión y traslados (vuelos de
//! posicionamiento sin pasajeros).
//!
//! Un avión está donde aterrizó por última vez: el destino de su último vuelo
//! en `flight_log` o de su último traslado. Si el OFP sale de otro aeropuerto,
//! el prevuelo ofrece "Solicitar traslado": dura el tiempo real del vuelo y su
//! costo (combustible con el avión vacío, tripulación, mantenimiento y tasas)
//! se carga a la aerolínea en Finanzas; no aparece en el FlightBook.
//!
//! Si el usuario vuela igual desde otro aeropuerto sin pedirlo, al cerrar el
//! vuelo se cobra un traslado automático. Sólo para vuelos desde que se activó
//! la función (`ferry_auto_since`): el historial anterior no se toca. Los
//! automáticos son datos derivados — se regeneran en cada recálculo de
//! Finanzas, así que editar o borrar un vuelo los corrige solos.

use crate::airline_economy::{self, normalize_reg, Size};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

/// Fecha desde la que se cobran traslados automáticos (se fija la 1ª vez).
const KEY_AUTO_SINCE: &str = "ferry_auto_since";

/// (24/10) Un traslado pedido llega en 5 minutos (antes tardaba el bloque
/// real del vuelo). El costo sigue siendo el del trayecto entero.
pub const REQUESTED_FERRY_MINUTES: i64 = 5;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ferry {
    pub id: i64,
    pub registration: String,
    pub from_icao: String,
    pub to_icao: String,
    pub distance_nm: f64,
    pub requested_at: String,
    pub arrives_at: String,
    pub cost_total: f64,
    /// "requested" | "auto"
    pub source: String,
    pub flight_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FerryQuote {
    pub from_icao: String,
    pub to_icao: String,
    pub distance_nm: f64,
    /// Bloque real del trayecto (base del costo).
    pub block_minutes: i64,
    /// (24/10) Lo que tarda en llegar si se pide.
    pub arrive_minutes: i64,
    pub cost_fuel: f64,
    pub cost_crew: f64,
    pub cost_maintenance: f64,
    pub cost_fees: f64,
    pub cost_total: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AircraftLocation {
    /// Matrícula normalizada (A-Z0-9).
    pub registration: String,
    /// Aeropuerto donde está — o al que va, si hay un traslado en curso.
    pub icao: Option<String>,
    pub airport_name: Option<String>,
    /// Aterrizaje o llegada del traslado que lo dejó ahí.
    pub since: Option<String>,
    /// El usuario lo está volando (vuelo sin cerrar).
    pub in_flight: bool,
    /// Traslado en curso (aún no ha llegado).
    pub ferry: Option<Ferry>,
    /// Modelo de su último vuelo (para estimar el costo del traslado).
    pub model: Option<String>,
    /// (6.2d) La ubicación la dio un vuelo REAL del avión (OpenSky) o su
    /// aterrizaje en vivo tras reservarlo.
    pub real: bool,
    /// (6.2d) Código Mode-S del avión real (si está en la base de aviones).
    pub icao24: Option<String>,
    /// (6.2d) Reserva activa.
    pub reservation: Option<Reservation>,
    /// (6.2d) Estado en vivo del avión real (sólo lo rellena `location_live`).
    pub live: Option<LiveInfo>,
}

/// (6.2d) Reserva activa de un avión del usuario.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reservation {
    pub id: i64,
    pub reserved_at: String,
    pub expires_at: String,
    /// Estaba volando de verdad al reservarlo.
    pub airborne: bool,
    /// Aterrizaje detectado en vivo (antes de que llegue el lote del día).
    pub landed_icao: Option<String>,
    pub landed_at: Option<String>,
}

/// (6.2d) El avión real, ahora mismo (OpenSky en vivo).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInfo {
    pub airborne: bool,
    pub callsign: Option<String>,
}

/// (6.2d) Una reserva vence si no se usa en estos días.
const RESERVATION_DAYS: i64 = 7;

// ───────────────────────────── Lógica pura ─────────────────────────────

#[derive(Debug, Clone)]
struct FlightEv {
    id: i64,
    started: DateTime<Utc>,
    ended: Option<DateTime<Utc>>,
    origin: Option<String>,
    dest: Option<String>,
    model: Option<String>,
    /// (6.2d) Vuelo REAL (OpenSky) del avión del usuario. Sin aeropuerto de
    /// llegada no lo mueve. `known_at` = cuándo lo supo SimFleet (descarga).
    real: bool,
    known_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct FerryEv {
    ferry: Ferry,
    requested: DateTime<Utc>,
    arrives: DateTime<Utc>,
}

/// (6.2d) Reserva: los vuelos reales que SALEN dentro de su ventana no
/// mueven el avión; su aterrizaje en vivo (si se detectó) lo deja ahí.
#[derive(Debug, Clone)]
struct ResEv {
    reserved: DateTime<Utc>,
    /// Fin (liberada, cancelada o vencida). `None` = activa.
    until: Option<DateTime<Utc>>,
    landed: Option<(DateTime<Utc>, String)>,
}

impl ResEv {
    fn blocks(&self, t: DateTime<Utc>) -> bool {
        t >= self.reserved && self.until.map_or(true, |u| t < u)
    }
}

enum Ev<'a> {
    Flight(&'a FlightEv),
    Ferry(&'a FerryEv),
    /// Aterrizaje detectado en vivo de un avión reservado.
    Landed(DateTime<Utc>, &'a str),
}

impl Ev<'_> {
    fn start(&self) -> DateTime<Utc> {
        match self {
            Ev::Flight(f) => f.started,
            Ev::Ferry(f) => f.requested,
            Ev::Landed(t, _) => *t,
        }
    }
}

/// Vuelos (del usuario y reales), traslados y aterrizajes de reservas de UNA
/// matrícula, en orden cronológico. Los vuelos reales que salen durante una
/// reserva se descartan: el avión es del usuario.
fn timeline<'a>(flights: &'a [FlightEv], ferries: &'a [FerryEv], res: &'a [ResEv]) -> Vec<Ev<'a>> {
    let mut evs: Vec<Ev> = flights
        .iter()
        .filter(|f| !(f.real && res.iter().any(|r| r.blocks(f.started))))
        .map(Ev::Flight)
        .chain(ferries.iter().map(Ev::Ferry))
        .chain(res.iter().filter_map(|r| r.landed.as_ref().map(|(t, icao)| Ev::Landed(*t, icao.as_str()))))
        .collect();
    evs.sort_by_key(|e| e.start());
    evs
}

/// Estado del avión tras aplicar eventos en orden.
#[derive(Debug, Default)]
struct Fold {
    at: Option<String>,
    since: Option<String>,
    in_flight: bool,
    ferry: Option<Ferry>,
    /// La ubicación la dio un vuelo real (o un aterrizaje en vivo).
    real: bool,
}

impl Fold {
    fn apply(&mut self, ev: &Ev, now: DateTime<Utc>) {
        match ev {
            Ev::Flight(f) if f.real => {
                // Un vuelo real sin aeropuerto de llegada no mueve el avión.
                if let Some(dest) = &f.dest {
                    self.at = Some(dest.clone());
                    self.since = f.ended.map(|e| e.to_rfc3339());
                    self.in_flight = false;
                    self.ferry = None;
                    self.real = true;
                }
            }
            Ev::Flight(f) => {
                self.ferry = None;
                self.real = false;
                match f.ended {
                    None => {
                        self.in_flight = true;
                        self.at = None;
                        self.since = None;
                    }
                    Some(ended) => {
                        self.in_flight = false;
                        self.at = f.dest.clone();
                        self.since = self.at.as_ref().map(|_| ended.to_rfc3339());
                    }
                }
            }
            Ev::Ferry(f) => {
                self.at = Some(f.ferry.to_icao.clone());
                self.in_flight = false;
                self.real = false;
                if f.arrives > now {
                    self.ferry = Some(f.ferry.clone());
                    self.since = None;
                } else {
                    self.ferry = None;
                    self.since = Some(f.ferry.arrives_at.clone());
                }
            }
            Ev::Landed(t, icao) => {
                self.at = Some(icao.to_string());
                self.since = Some(t.to_rfc3339());
                self.in_flight = false;
                self.ferry = None;
                self.real = true;
            }
        }
    }
}

/// Dónde está el avión ahora.
fn locate(reg: &str, flights: &[FlightEv], ferries: &[FerryEv], res: &[ResEv], now: DateTime<Utc>) -> AircraftLocation {
    let model = flights
        .iter()
        .filter(|f| f.model.is_some())
        .max_by_key(|f| (!f.real, f.started))
        .and_then(|f| f.model.clone());
    let mut st = Fold::default();
    for ev in timeline(flights, ferries, res) {
        st.apply(&ev, now);
    }
    AircraftLocation {
        registration: reg.to_string(),
        icao: st.at,
        since: st.since,
        in_flight: st.in_flight,
        ferry: st.ferry,
        model,
        real: st.real,
        ..Default::default()
    }
}

/// Traslado automático necesario: (vuelo, desde, hacia, modelo, salida del vuelo).
#[derive(Debug, Clone, PartialEq)]
struct AutoNeed {
    flight_id: i64,
    from: String,
    to: String,
    model: Option<String>,
    started: DateTime<Utc>,
}

/// Vuelos del usuario (cerrados, desde `since`) que salieron de un aeropuerto
/// distinto de donde estaba el avión. Para cada uno, la ubicación se calcula
/// sólo con lo que SimFleet sabía al salir: los vuelos reales que llegaron
/// después en una descarga no generan cobros retroactivos. `ferries` son sólo
/// los traslados PEDIDOS: un pedido cubre el vuelo aunque saliera antes de que
/// "llegara".
fn plan_auto(flights: &[FlightEv], ferries: &[FerryEv], res: &[ResEv], since: DateTime<Utc>) -> Vec<AutoNeed> {
    let evs = timeline(flights, ferries, res);
    let mut out = Vec::new();
    for f in flights.iter().filter(|f| !f.real && f.ended.is_some() && f.started >= since) {
        let Some(origin) = &f.origin else { continue };
        let mut st = Fold::default();
        for ev in evs.iter().filter(|e| e.start() < f.started) {
            if let Ev::Flight(r) = ev {
                if r.real && r.known_at.map_or(true, |k| k > f.started) {
                    continue;
                }
            }
            st.apply(ev, f.started);
        }
        if let Some(here) = st.at.filter(|h| h != origin) {
            out.push(AutoNeed {
                flight_id: f.id,
                from: here,
                to: origin.clone(),
                model: f.model.clone(),
                started: f.started,
            });
        }
    }
    out
}

fn cruise_kt(size: Size) -> f64 {
    // Igual que la estimación de distancia de Finanzas.
    match size {
        Size::Wide => 470.0,
        Size::Narrow => 440.0,
        Size::Regional => 300.0,
    }
}

/// Combustible de rodaje + ascenso (kg), aparte del crucero por milla.
fn taxi_climb_kg(size: Size) -> f64 {
    match size {
        Size::Wide => 2500.0,
        Size::Narrow => 900.0,
        Size::Regional => 300.0,
    }
}

/// Tripulación mínima por hora de bloque (pilotos + TCP mínimos).
fn crew_rate(size: Size) -> f64 {
    match size {
        Size::Wide => 3200.0,
        Size::Narrow => 1800.0,
        Size::Regional => 900.0,
    }
}

/// Costo de un traslado de `d` NM. El avión va vacío (15 % menos de consumo
/// en crucero); bloque = crucero + 30 min de rodaje, ascenso y descenso.
fn quote_for(from: &str, to: &str, d: f64, model: Option<&str>) -> FerryQuote {
    let (size, _) = airline_economy::classify(model);
    let block_h = d / cruise_kt(size) + 0.5;
    let fuel_kg = taxi_climb_kg(size) + d * airline_economy::burn_per_nm(size) * 0.85;
    let cost_fuel = fuel_kg * airline_economy::FUEL_USD_PER_KG;
    let cost_crew = block_h * crew_rate(size);
    let cost_maintenance = block_h * airline_economy::maint_rate(size);
    let cost_fees = airline_economy::landing_fee(size) + d * 1.2;
    FerryQuote {
        from_icao: from.to_string(),
        to_icao: to.to_string(),
        distance_nm: d,
        block_minutes: (block_h * 60.0).round() as i64,
        arrive_minutes: REQUESTED_FERRY_MINUTES,
        cost_fuel,
        cost_crew,
        cost_maintenance,
        cost_fees,
        cost_total: cost_fuel + cost_crew + cost_maintenance + cost_fees,
    }
}

// ───────────────────────────── Base de datos ─────────────────────────────

fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.with_timezone(&Utc))
}

fn clean_icao(s: Option<String>) -> Option<String> {
    s.map(|x| x.trim().to_ascii_uppercase()).filter(|x| !x.is_empty())
}

/// Vuelos con matrícula, agrupados por matrícula normalizada.
async fn load_flights(pool: &SqlitePool) -> Result<HashMap<String, Vec<FlightEv>>> {
    let rows = sqlx::query(
        "SELECT id, started_at, ended_at, origin_icao, destination_icao, aircraft_registration,
                aircraft_model, aircraft_atc_type
           FROM flight_log
          WHERE aircraft_registration IS NOT NULL AND TRIM(aircraft_registration) <> ''",
    )
    .fetch_all(pool)
    .await?;
    let mut out: HashMap<String, Vec<FlightEv>> = HashMap::new();
    for r in rows {
        let reg = normalize_reg(&r.get::<String, _>("aircraft_registration"));
        let Some(started) = parse_dt(&r.get::<String, _>("started_at")) else { continue };
        if reg.is_empty() {
            continue;
        }
        let model = r
            .get::<Option<String>, _>("aircraft_model")
            .or_else(|| r.get::<Option<String>, _>("aircraft_atc_type"))
            .filter(|m| !m.trim().is_empty());
        out.entry(reg).or_default().push(FlightEv {
            id: r.get("id"),
            started,
            ended: r.get::<Option<String>, _>("ended_at").as_deref().and_then(parse_dt),
            origin: clean_icao(r.get("origin_icao")),
            dest: clean_icao(r.get("destination_icao")),
            model,
            real: false,
            known_at: None,
        });
    }
    Ok(out)
}

/// (6.2d) Vuelos REALES de los aviones del usuario (OpenSky), por matrícula.
/// `known_at` = cuándo se bajó su ventana (para no cobrar retroactivo).
async fn load_real_flights(pool: &SqlitePool) -> Result<HashMap<String, Vec<FlightEv>>> {
    let mine = crate::my_fleet::my_aircraft(pool).await.unwrap_or_default();
    let mut out: HashMap<String, Vec<FlightEv>> = HashMap::new();
    if mine.is_empty() {
        return Ok(out);
    }
    for r in sqlx::query(
        "SELECT rf.first_seen AS fs, rf.last_seen AS ls, rf.dep_icao AS dep, rf.arr_icao AS arr,
                a.registration AS reg, a.typecode AS tc, w.fetched_at AS known
           FROM real_flight rf
           JOIN aircraft_db a ON a.icao24 = rf.icao24
           LEFT JOIN opensky_window w ON w.begin_ts = rf.window_begin
          WHERE a.registration IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let reg = normalize_reg(&r.get::<String, _>("reg"));
        if !mine.contains_key(&reg) {
            continue;
        }
        let (Some(started), Some(ended)) = (
            DateTime::from_timestamp(r.get::<i64, _>("fs"), 0),
            DateTime::from_timestamp(r.get::<i64, _>("ls"), 0),
        ) else {
            continue;
        };
        out.entry(reg).or_default().push(FlightEv {
            id: 0,
            started,
            ended: Some(ended),
            origin: clean_icao(r.get("dep")),
            dest: clean_icao(r.get("arr")),
            model: r.get("tc"),
            real: true,
            known_at: r.get::<Option<String>, _>("known").as_deref().and_then(parse_dt),
        });
    }
    Ok(out)
}

/// (6.2d) Reservas por matrícula (todas, para la historia) y la activa.
async fn load_reservations(pool: &SqlitePool) -> Result<HashMap<String, (Vec<ResEv>, Option<Reservation>)>> {
    let mut out: HashMap<String, (Vec<ResEv>, Option<Reservation>)> = HashMap::new();
    for r in sqlx::query("SELECT * FROM aircraft_reservation ORDER BY reserved_at").fetch_all(pool).await? {
        let Some(reserved) = parse_dt(&r.get::<String, _>("reserved_at")) else { continue };
        let status: String = r.get("status");
        let ended = r.get::<Option<String>, _>("ended_at").as_deref().and_then(parse_dt);
        let landed_icao: Option<String> = r.get("landed_icao");
        let landed_at: Option<String> = r.get("landed_at");
        let ev = ResEv {
            reserved,
            until: if status == "active" { None } else { ended.or(Some(reserved)) },
            landed: match (landed_at.as_deref().and_then(parse_dt), landed_icao.clone()) {
                (Some(t), Some(icao)) => Some((t, icao)),
                _ => None,
            },
        };
        let e = out.entry(r.get("registration")).or_default();
        e.0.push(ev);
        if status == "active" {
            e.1 = Some(Reservation {
                id: r.get("id"),
                reserved_at: r.get("reserved_at"),
                expires_at: r.get("expires_at"),
                airborne: r.get::<i64, _>("airborne") != 0,
                landed_icao,
                landed_at,
            });
        }
    }
    Ok(out)
}

/// (6.2d) Vence las reservas viejas y libera las de aviones que el usuario
/// ya voló después de reservarlos.
async fn sync_reservations(pool: &SqlitePool) -> Result<()> {
    let now = Utc::now();
    let active = sqlx::query("SELECT id, registration, reserved_at, expires_at FROM aircraft_reservation WHERE status = 'active'")
        .fetch_all(pool)
        .await?;
    if active.is_empty() {
        return Ok(());
    }
    let flights = load_flights(pool).await?;
    for r in active {
        let id: i64 = r.get("id");
        let reg: String = r.get("registration");
        let Some(reserved) = parse_dt(&r.get::<String, _>("reserved_at")) else { continue };
        let expires = parse_dt(&r.get::<String, _>("expires_at")).unwrap_or(reserved + Duration::days(RESERVATION_DAYS));
        let flown = flights
            .get(&reg)
            .and_then(|fl| fl.iter().filter(|f| f.started > reserved).map(|f| f.started).min());
        let end = match flown {
            Some(t) => Some(("released", t)),
            None if now > expires => Some(("expired", expires)),
            None => None,
        };
        if let Some((status, t)) = end {
            sqlx::query("UPDATE aircraft_reservation SET status = ?, ended_at = ? WHERE id = ?")
                .bind(status)
                .bind(t.to_rfc3339())
                .bind(id)
                .execute(pool)
                .await?;
            tracing::info!(target: "ferry", "reserva #{id} ({reg}): {status}");
        }
    }
    Ok(())
}

/// (6.2d) icao24 (Mode-S) de una matrícula del usuario según la base de aviones.
async fn icao24_of(pool: &SqlitePool, reg: &str) -> Option<String> {
    crate::my_fleet::my_icao24(pool)
        .await
        .ok()?
        .into_iter()
        .find(|(_, r)| r == reg)
        .map(|(hex, _)| hex)
}

/// Traslados activos agrupados por matrícula. `only_requested` deja fuera los
/// automáticos (para recalcularlos).
async fn load_ferries(pool: &SqlitePool, only_requested: bool) -> Result<HashMap<String, Vec<FerryEv>>> {
    let sql = if only_requested {
        "SELECT * FROM aircraft_ferry WHERE status = 'active' AND source = 'requested'"
    } else {
        "SELECT * FROM aircraft_ferry WHERE status = 'active'"
    };
    let rows = sqlx::query(sql).fetch_all(pool).await?;
    let mut out: HashMap<String, Vec<FerryEv>> = HashMap::new();
    for r in rows {
        let ferry = Ferry {
            id: r.get("id"),
            registration: r.get("registration"),
            from_icao: r.get("from_icao"),
            to_icao: r.get("to_icao"),
            distance_nm: r.get("distance_nm"),
            requested_at: r.get("requested_at"),
            arrives_at: r.get("arrives_at"),
            cost_total: r.get("cost_total"),
            source: r.get("source"),
            flight_id: r.get("flight_id"),
        };
        let (Some(requested), Some(arrives)) = (parse_dt(&ferry.requested_at), parse_dt(&ferry.arrives_at)) else {
            continue;
        };
        out.entry(ferry.registration.clone())
            .or_default()
            .push(FerryEv { ferry, requested, arrives });
    }
    Ok(out)
}

async fn airport_point(pool: &SqlitePool, icao: &str) -> Result<Option<(f64, f64, String)>> {
    let row = sqlx::query("SELECT latitude, longitude, name FROM airports WHERE icao = ?")
        .bind(icao.trim().to_ascii_uppercase())
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get("latitude"), r.get("longitude"), r.get("name"))))
}

async fn airport_name(pool: &SqlitePool, icao: Option<&str>) -> Option<String> {
    let icao = icao?;
    airport_point(pool, icao).await.ok().flatten().map(|(_, _, n)| n)
}

/// Costo y duración de un traslado entre dos aeropuertos.
pub async fn quote(pool: &SqlitePool, from: &str, to: &str, model: Option<&str>) -> Result<FerryQuote> {
    let a = airport_point(pool, from).await?;
    let b = airport_point(pool, to).await?;
    let (Some((lat1, lon1, _)), Some((lat2, lon2, _))) = (a, b) else {
        return Err(anyhow!(crate::tr!("ferry.err.noCoords")));
    };
    let d = crate::flight_log::haversine_nm(lat1, lon1, lat2, lon2);
    Ok(quote_for(&from.trim().to_ascii_uppercase(), &to.trim().to_ascii_uppercase(), d, model))
}

/// Ubicación de una matrícula (vuelos del usuario + reales + traslados +
/// reservas).
pub async fn location(pool: &SqlitePool, registration: &str) -> Result<AircraftLocation> {
    sync_reservations(pool).await?;
    let reg = normalize_reg(registration);
    let mut flights = load_flights(pool).await?.remove(&reg).unwrap_or_default();
    flights.extend(load_real_flights(pool).await?.remove(&reg).unwrap_or_default());
    let ferries = load_ferries(pool, false).await?.remove(&reg).unwrap_or_default();
    let (res, active) = load_reservations(pool).await?.remove(&reg).unwrap_or_default();
    let mut loc = locate(&reg, &flights, &ferries, &res, Utc::now());
    loc.airport_name = airport_name(pool, loc.icao.as_deref()).await;
    loc.reservation = active;
    loc.icao24 = icao24_of(pool, &reg).await;
    Ok(loc)
}

/// (6.2d) Ubicación + estado en vivo del avión real (OpenSky), si la build
/// trae claves y el avión está en la base de aviones.
pub async fn location_live(pool: &SqlitePool, http: &reqwest::Client, registration: &str) -> Result<AircraftLocation> {
    let mut loc = location(pool, registration).await?;
    if let (true, Some(hex)) = (crate::opensky::is_configured(), loc.icao24.as_deref()) {
        match crate::opensky::live_state(http, hex).await {
            Ok(st) => {
                loc.live = Some(LiveInfo {
                    airborne: st.as_ref().is_some_and(|s| !s.on_ground),
                    callsign: st.and_then(|s| s.callsign),
                })
            }
            Err(e) => tracing::debug!(target: "ferry", "estado en vivo de {hex}: {e:#}"),
        }
    }
    Ok(loc)
}

/// Ubicación de todas las matrículas con vuelos o traslados (para el Hangar).
pub async fn locations(pool: &SqlitePool) -> Result<HashMap<String, AircraftLocation>> {
    sync_reservations(pool).await?;
    let mut flights = load_flights(pool).await?;
    for (reg, real) in load_real_flights(pool).await? {
        flights.entry(reg).or_default().extend(real);
    }
    let mut ferries = load_ferries(pool, false).await?;
    let mut reservations = load_reservations(pool).await?;
    let regs: Vec<String> = flights.keys().chain(ferries.keys()).cloned().collect();
    let now = Utc::now();
    let mut out = HashMap::new();
    for reg in regs {
        if out.contains_key(&reg) {
            continue;
        }
        let f = flights.remove(&reg).unwrap_or_default();
        let t = ferries.remove(&reg).unwrap_or_default();
        let (res, active) = reservations.remove(&reg).unwrap_or_default();
        let mut loc = locate(&reg, &f, &t, &res, now);
        loc.reservation = active;
        out.insert(reg.clone(), loc);
    }
    Ok(out)
}

/// Pide un traslado de `registration` a `to_icao`, desde donde esté. Con
/// `http`, comprueba antes que el avión real no esté volando ahora.
pub async fn request(
    pool: &SqlitePool,
    http: Option<&reqwest::Client>,
    registration: &str,
    to_icao: &str,
    model: Option<&str>,
) -> Result<Ferry> {
    let to = to_icao.trim().to_ascii_uppercase();
    let loc = match http {
        Some(h) => location_live(pool, h, registration).await?,
        None => location(pool, registration).await?,
    };
    if loc.in_flight {
        return Err(anyhow!(crate::tr!("ferry.err.inFlight")));
    }
    // En el aire no se traslada — salvo que ya esté reservado en tierra (el
    // avión del usuario se quedó; el real sigue volando aparte).
    let waiting_landing = loc.reservation.as_ref().is_some_and(|r| r.airborne && r.landed_icao.is_none());
    if loc.live.as_ref().is_some_and(|l| l.airborne) && (loc.reservation.is_none() || waiting_landing) {
        return Err(anyhow!(if waiting_landing {
            crate::tr!("ferry.err.waitingLanding")
        } else {
            crate::tr!("ferry.err.airborne")
        }));
    }
    if loc.ferry.is_some() {
        return Err(anyhow!(crate::tr!("ferry.err.enroute")));
    }
    let Some(from) = loc.icao.clone() else {
        return Err(anyhow!(crate::tr!("ferry.err.noLocation")));
    };
    if from == to {
        return Err(anyhow!(crate::tr!("ferry.err.sameAirport", icao = to)));
    }
    let model = model.filter(|m| !m.trim().is_empty()).map(str::to_string).or(loc.model.clone());
    let q = quote(pool, &from, &to, model.as_deref()).await?;
    let now = Utc::now();
    let arrives = now + Duration::minutes(q.arrive_minutes);
    let id = insert(pool, &loc.registration, &q, model.as_deref(), now, arrives, "requested", None).await?;
    tracing::info!(
        target: "ferry",
        "traslado pedido #{id} {} {from}→{to} · llega en {} min (bloque {} min) · {:.0} USD",
        loc.registration,
        q.arrive_minutes,
        q.block_minutes,
        q.cost_total
    );
    Ok(Ferry {
        id,
        registration: loc.registration,
        from_icao: from,
        to_icao: to,
        distance_nm: q.distance_nm,
        requested_at: now.to_rfc3339(),
        arrives_at: arrives.to_rfc3339(),
        cost_total: q.cost_total,
        source: "requested".into(),
        flight_id: None,
    })
}

/// Cancela un traslado pedido que aún no ha llegado (no se cobra).
pub async fn cancel(pool: &SqlitePool, id: i64) -> Result<()> {
    let row = sqlx::query("SELECT source, status, arrives_at FROM aircraft_ferry WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!(crate::tr!("ferry.err.notFound")))?;
    let source: String = row.get("source");
    let status: String = row.get("status");
    let arrived = parse_dt(&row.get::<String, _>("arrives_at")).is_some_and(|t| t <= Utc::now());
    if source != "requested" || status != "active" || arrived {
        return Err(anyhow!(crate::tr!("ferry.err.cannotCancel")));
    }
    sqlx::query("UPDATE aircraft_ferry SET status = 'cancelled', cancelled_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    tracing::info!(target: "ferry", "traslado #{id} cancelado");
    Ok(())
}

/// (6.2d) Reserva un avión del usuario: al aterrizar queda para él y sus
/// vuelos reales posteriores dejan de moverlo (hasta volarlo, cancelar o 7
/// días). Si está volando ahora, se vigila en vivo su aterrizaje.
pub async fn reserve(pool: &SqlitePool, http: &reqwest::Client, registration: &str) -> Result<Reservation> {
    sync_reservations(pool).await?;
    let loc = location_live(pool, http, registration).await?;
    if loc.reservation.is_some() {
        return Err(anyhow!(crate::tr!("ferry.err.alreadyReserved")));
    }
    let now = Utc::now();
    let expires = now + Duration::days(RESERVATION_DAYS);
    let airborne = loc.live.as_ref().is_some_and(|l| l.airborne);
    let res = sqlx::query(
        "INSERT INTO aircraft_reservation (registration, icao24, reserved_at, expires_at, airborne)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&loc.registration)
    .bind(&loc.icao24)
    .bind(now.to_rfc3339())
    .bind(expires.to_rfc3339())
    .bind(airborne as i64)
    .execute(pool)
    .await?;
    let id = res.last_insert_rowid();
    tracing::info!(target: "ferry", "reserva #{id} {} (en el aire: {airborne})", loc.registration);
    Ok(Reservation {
        id,
        reserved_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
        airborne,
        landed_icao: None,
        landed_at: None,
    })
}

/// (6.2d) Cancela una reserva activa: el avión vuelve a operar.
pub async fn cancel_reservation(pool: &SqlitePool, id: i64) -> Result<()> {
    let n = sqlx::query("UPDATE aircraft_reservation SET status = 'cancelled', ended_at = ? WHERE id = ? AND status = 'active'")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(anyhow!(crate::tr!("ferry.err.reservationNotFound")));
    }
    Ok(())
}

/// (6.2d) Aeropuerto más cercano (≤ 5 NM) a una posición.
async fn nearest_airport(pool: &SqlitePool, lat: f64, lon: f64) -> Option<String> {
    let rows = sqlx::query(
        "SELECT icao, latitude, longitude FROM airports
          WHERE latitude BETWEEN ? AND ? AND longitude BETWEEN ? AND ?",
    )
    .bind(lat - 0.2)
    .bind(lat + 0.2)
    .bind(lon - 0.3)
    .bind(lon + 0.3)
    .fetch_all(pool)
    .await
    .ok()?;
    rows.into_iter()
        .map(|r| {
            let d = crate::flight_log::haversine_nm(lat, lon, r.get("latitude"), r.get("longitude"));
            (d, r.get::<String, _>("icao"))
        })
        .filter(|(d, _)| *d <= 5.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, icao)| icao)
}

/// (6.2d) Para las reservas de aviones que estaban volando: mira en vivo si
/// ya aterrizaron y dónde (así el usuario lo puede usar sin esperar al lote
/// del día siguiente). Lo llama un bucle cada 10 min.
pub async fn poll_reservation_landings(pool: &SqlitePool, http: &reqwest::Client) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT id, registration, icao24 FROM aircraft_reservation
          WHERE status = 'active' AND airborne = 1 AND landed_icao IS NULL AND icao24 IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    let mut landed = 0;
    for r in rows {
        let id: i64 = r.get("id");
        let hex: String = r.get("icao24");
        let Ok(Some(st)) = crate::opensky::live_state(http, &hex).await else { continue };
        let (true, Some(lat), Some(lon)) = (st.on_ground, st.lat, st.lon) else { continue };
        let Some(icao) = nearest_airport(pool, lat, lon).await else { continue };
        sqlx::query("UPDATE aircraft_reservation SET landed_icao = ?, landed_at = ? WHERE id = ?")
            .bind(&icao)
            .bind(Utc::now().to_rfc3339())
            .bind(id)
            .execute(pool)
            .await?;
        tracing::info!(target: "ferry", "reserva #{id} ({}): aterrizó en {icao}", r.get::<String, _>("registration"));
        landed += 1;
    }
    Ok(landed)
}

#[allow(clippy::too_many_arguments)]
async fn insert<'e, E>(
    exec: E,
    reg: &str,
    q: &FerryQuote,
    model: Option<&str>,
    requested: DateTime<Utc>,
    arrives: DateTime<Utc>,
    source: &str,
    flight_id: Option<i64>,
) -> Result<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let res = sqlx::query(
        "INSERT INTO aircraft_ferry
            (registration, from_icao, to_icao, distance_nm, model, requested_at, arrives_at,
             cost_fuel, cost_crew, cost_maintenance, cost_fees, cost_total, source, flight_id)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(reg)
    .bind(&q.from_icao)
    .bind(&q.to_icao)
    .bind(q.distance_nm)
    .bind(model)
    .bind(requested.to_rfc3339())
    .bind(arrives.to_rfc3339())
    .bind(q.cost_fuel)
    .bind(q.cost_crew)
    .bind(q.cost_maintenance)
    .bind(q.cost_fees)
    .bind(q.cost_total)
    .bind(source)
    .bind(flight_id)
    .execute(exec)
    .await?;
    Ok(res.last_insert_rowid())
}

async fn auto_since(pool: &SqlitePool) -> Result<DateTime<Utc>> {
    let stored: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY_AUTO_SINCE)
        .fetch_optional(pool)
        .await?;
    if let Some(t) = stored.as_deref().and_then(parse_dt) {
        return Ok(t);
    }
    let now = Utc::now();
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(KEY_AUTO_SINCE)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
    tracing::info!(target: "ferry", "cobro de traslados automáticos activo desde {now}");
    Ok(now)
}

/// Regenera los traslados automáticos: borra los que había y crea uno por
/// cada vuelo (cerrado, desde `ferry_auto_since`) que salió de otro aeropuerto.
/// Lo llama el recálculo de Finanzas antes de sumar costos.
pub async fn sync_auto_ferries(pool: &SqlitePool) -> Result<usize> {
    let since = auto_since(pool).await?;
    sync_reservations(pool).await?;
    let mut flights = load_flights(pool).await?;
    // (6.2d) Los vuelos reales del avión también lo mueven (sólo lo que se
    // sabía al salir cada vuelo del usuario).
    for (reg, real) in load_real_flights(pool).await? {
        if let Some(fl) = flights.get_mut(&reg) {
            fl.extend(real);
        }
    }
    let requested = load_ferries(pool, true).await?;
    let reservations = load_reservations(pool).await?;
    let mut needs = Vec::new();
    for (reg, fl) in &flights {
        let req = requested.get(reg).map(Vec::as_slice).unwrap_or(&[]);
        let res = reservations.get(reg).map(|(r, _)| r.as_slice()).unwrap_or(&[]);
        needs.extend(plan_auto(fl, req, res, since).into_iter().map(|n| (reg.clone(), n)));
    }
    let mut quotes = Vec::with_capacity(needs.len());
    for (reg, n) in needs {
        match quote(pool, &n.from, &n.to, n.model.as_deref()).await {
            Ok(q) => quotes.push((reg, n, q)),
            Err(e) => tracing::debug!(target: "ferry", "sin traslado automático para el vuelo {}: {e}", n.flight_id),
        }
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM aircraft_ferry WHERE source = 'auto'").execute(&mut *tx).await?;
    for (reg, n, q) in &quotes {
        // Llega justo cuando sale el vuelo.
        let requested = n.started - Duration::minutes(q.block_minutes);
        insert(&mut *tx, reg, q, n.model.as_deref(), requested, n.started, "auto", Some(n.flight_id)).await?;
    }
    tx.commit().await?;
    Ok(quotes.len())
}

/// Todos los traslados activos (pedidos y automáticos). El recálculo de
/// Finanzas los carga a la aerolínea de cada matrícula y guarda esa
/// aerolínea en `airline_key` (dentro de su propia transacción).
pub async fn active(pool: &SqlitePool) -> Result<Vec<Ferry>> {
    Ok(load_ferries(pool, false)
        .await?
        .into_values()
        .flatten()
        .map(|f| f.ferry)
        .collect())
}

/// Traslados activos de una aerolínea (para la curva de saldo). Usa la
/// aerolínea que fijó el último recálculo (`assign_airlines`).
pub async fn for_airline(pool: &SqlitePool, airline_key: &str) -> Result<Vec<Ferry>> {
    let rows = sqlx::query(
        "SELECT id, registration, from_icao, to_icao, distance_nm, requested_at, arrives_at,
                cost_total, source, flight_id
           FROM aircraft_ferry
          WHERE status = 'active' AND airline_key = ?
          ORDER BY requested_at",
    )
    .bind(airline_key)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Ferry {
            id: r.get("id"),
            registration: r.get("registration"),
            from_icao: r.get("from_icao"),
            to_icao: r.get("to_icao"),
            distance_nm: r.get("distance_nm"),
            requested_at: r.get("requested_at"),
            arrives_at: r.get("arrives_at"),
            cost_total: r.get("cost_total"),
            source: r.get("source"),
            flight_id: r.get("flight_id"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        parse_dt(s).unwrap()
    }

    fn flight(id: i64, start: &str, end: Option<&str>, o: &str, d: &str) -> FlightEv {
        FlightEv {
            id,
            started: t(start),
            ended: end.map(t),
            origin: Some(o.into()),
            dest: Some(d.into()),
            model: Some("A320".into()),
            real: false,
            known_at: None,
        }
    }

    fn ferry(id: i64, from: &str, to: &str, req: &str, arr: &str) -> FerryEv {
        FerryEv {
            ferry: Ferry {
                id,
                registration: "N657JB".into(),
                from_icao: from.into(),
                to_icao: to.into(),
                distance_nm: 100.0,
                requested_at: req.into(),
                arrives_at: arr.into(),
                cost_total: 1000.0,
                source: "requested".into(),
                flight_id: None,
            },
            requested: t(req),
            arrives: t(arr),
        }
    }

    #[test]
    fn location_is_the_last_landing_or_ferry() {
        let now = t("2026-10-22T12:00:00Z");
        let fl = vec![
            flight(1, "2026-10-20T10:00:00Z", Some("2026-10-20T12:00:00Z"), "SKRG", "KFLL"),
            flight(2, "2026-10-21T10:00:00Z", Some("2026-10-21T13:00:00Z"), "KFLL", "KBOS"),
        ];
        let loc = locate("N657JB", &fl, &[], &[], now);
        assert_eq!(loc.icao.as_deref(), Some("KBOS"));
        assert_eq!(loc.since.as_deref(), Some("2026-10-21T13:00:00+00:00"));
        assert!(loc.ferry.is_none() && !loc.in_flight);

        // Traslado en curso → va hacia KJFK, aún sin "since".
        let tr = vec![ferry(9, "KBOS", "KJFK", "2026-10-22T11:30:00Z", "2026-10-22T12:30:00Z")];
        let loc = locate("N657JB", &fl, &tr, &[], now);
        assert_eq!(loc.icao.as_deref(), Some("KJFK"));
        assert_eq!(loc.ferry.as_ref().map(|f| f.id), Some(9));
        // Ya llegó.
        let loc = locate("N657JB", &fl, &tr, &[], t("2026-10-22T13:00:00Z"));
        assert!(loc.ferry.is_none());
        assert_eq!(loc.since.as_deref(), Some("2026-10-22T12:30:00Z"));

        // Vuelo abierto → en vuelo, sin aeropuerto.
        let mut open = fl.clone();
        open.push(flight(3, "2026-10-22T11:00:00Z", None, "KBOS", "KMIA"));
        let loc = locate("N657JB", &open, &[], &[], now);
        assert!(loc.in_flight && loc.icao.is_none());
    }

    #[test]
    fn auto_ferry_only_for_jumps_after_activation() {
        let since = t("2026-10-21T00:00:00Z");
        let fl = vec![
            // Salto viejo (antes de activar): no se cobra.
            flight(1, "2026-10-19T10:00:00Z", Some("2026-10-19T12:00:00Z"), "SKRG", "KFLL"),
            flight(2, "2026-10-20T10:00:00Z", Some("2026-10-20T12:00:00Z"), "MDSD", "KBOS"),
            // Salto nuevo KBOS → sale de KJFK: se cobra KBOS→KJFK.
            flight(3, "2026-10-21T10:00:00Z", Some("2026-10-21T12:00:00Z"), "KJFK", "KMIA"),
            // Sale de donde aterrizó: nada.
            flight(4, "2026-10-21T15:00:00Z", Some("2026-10-21T17:00:00Z"), "KMIA", "KATL"),
            // Vuelo abierto que salta: se cobra al cerrarlo, no antes.
            flight(5, "2026-10-22T10:00:00Z", None, "KORD", "KDEN"),
        ];
        let needs = plan_auto(&fl, &[], &[], since);
        assert_eq!(needs.len(), 1);
        assert_eq!((needs[0].flight_id, needs[0].from.as_str(), needs[0].to.as_str()), (3, "KBOS", "KJFK"));

        // Con un traslado PEDIDO a KJFK antes del vuelo 3, no hay cobro extra
        // (aunque saliera antes de que "llegara").
        let tr = vec![ferry(7, "KBOS", "KJFK", "2026-10-21T09:00:00Z", "2026-10-21T10:30:00Z")];
        assert!(plan_auto(&fl, &tr, &[], since).is_empty());

        // Pedido a otro aeropuerto (KEWR) → se cobra KEWR→KJFK.
        let tr = vec![ferry(8, "KBOS", "KEWR", "2026-10-21T08:00:00Z", "2026-10-21T09:00:00Z")];
        let needs = plan_auto(&fl, &tr, &[], since);
        assert_eq!((needs[0].from.as_str(), needs[0].to.as_str()), ("KEWR", "KJFK"));
    }

    fn real(start: &str, end: &str, o: Option<&str>, d: Option<&str>, known: &str) -> FlightEv {
        FlightEv {
            id: 0,
            started: t(start),
            ended: Some(t(end)),
            origin: o.map(str::to_string),
            dest: d.map(str::to_string),
            model: Some("A320".into()),
            real: true,
            known_at: Some(t(known)),
        }
    }

    /// (6.2d) Los vuelos reales mueven el avión; sin llegada no; dentro de
    /// una reserva no; y el cobro automático sólo usa lo que ya se sabía.
    #[test]
    fn real_flights_move_the_aircraft_without_retroactive_charges() {
        let now = t("2026-10-23T12:00:00Z");
        let mine = flight(1, "2026-10-21T08:00:00Z", Some("2026-10-21T10:00:00Z"), "KFLL", "KBOS");
        // Vuelo real KBOS→KJFK el 21/10 a las 12:00, sabido el 22/10 a las 07:00.
        let r1 = real("2026-10-21T12:00:00Z", "2026-10-21T13:00:00Z", Some("KBOS"), Some("KJFK"), "2026-10-22T07:00:00Z");
        // Vuelo real sin aeropuerto de llegada: no mueve.
        let r2 = real("2026-10-21T15:00:00Z", "2026-10-21T16:00:00Z", Some("KJFK"), None, "2026-10-22T07:00:00Z");
        let loc = locate("N657JB", &[mine.clone(), r1.clone(), r2.clone()], &[], &[], now);
        assert_eq!(loc.icao.as_deref(), Some("KJFK"));
        assert!(loc.real);

        let since = t("2026-10-20T00:00:00Z");
        // El usuario vuela desde KBOS el 21/10 a las 18:00 (aún no se sabía del
        // vuelo real a KJFK): sin cobro retroactivo.
        let mine2 = flight(2, "2026-10-21T18:00:00Z", Some("2026-10-21T20:00:00Z"), "KBOS", "KMIA");
        assert!(plan_auto(&[mine.clone(), r1.clone(), mine2], &[], &[], since).is_empty());
        // El 22/10 a las 10:00 ya se sabía → salir de KBOS se cobra KJFK→KBOS.
        let mine3 = flight(3, "2026-10-22T10:00:00Z", Some("2026-10-22T12:00:00Z"), "KBOS", "KMIA");
        let needs = plan_auto(&[mine.clone(), r1.clone(), mine3], &[], &[], since);
        assert_eq!(needs.len(), 1);
        assert_eq!((needs[0].from.as_str(), needs[0].to.as_str()), ("KJFK", "KBOS"));

        // Reservado el 21/10 a las 11:00: el vuelo real de las 12:00 no lo mueve.
        let res = vec![ResEv { reserved: t("2026-10-21T11:00:00Z"), until: None, landed: None }];
        let loc = locate("N657JB", &[mine.clone(), r1.clone()], &[], &res, now);
        assert_eq!(loc.icao.as_deref(), Some("KBOS"));
        // Aterrizaje en vivo detectado tras reservar (estaba volando): queda ahí.
        let res = vec![ResEv {
            reserved: t("2026-10-21T11:00:00Z"),
            until: None,
            landed: Some((t("2026-10-21T11:40:00Z"), "KPVD".into())),
        }];
        let loc = locate("N657JB", &[mine, r1], &[], &res, now);
        assert_eq!(loc.icao.as_deref(), Some("KPVD"));
        assert!(loc.real);
    }

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (icao, lat, lon) in [("KBOS", 42.3656, -71.0096), ("KJFK", 40.6413, -73.7781), ("KMIA", 25.7959, -80.2870)] {
            sqlx::query("INSERT INTO airports (icao, name, latitude, longitude) VALUES (?, ?, ?, ?)")
                .bind(icao)
                .bind(format!("{icao} Intl"))
                .bind(lat)
                .bind(lon)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    async fn add_flight(pool: &SqlitePool, start: &str, end: &str, o: &str, d: &str) {
        sqlx::query(
            "INSERT INTO flight_log (started_at, ended_at, origin_lat, origin_lon, origin_icao,
                                     destination_icao, aircraft_registration, aircraft_model)
             VALUES (?, ?, 0, 0, ?, ?, 'N657-JB', 'FenixA320 IAE')",
        )
        .bind(start)
        .bind(end)
        .bind(o)
        .bind(d)
        .execute(pool)
        .await
        .unwrap();
    }

    /// De punta a punta con la base real: pedir, no poder pedir otro en curso,
    /// cancelar, y cobro automático sólo para vuelos tras la activación.
    #[tokio::test]
    async fn request_cancel_and_auto_charge() {
        let pool = db().await;
        add_flight(&pool, "2026-10-20T10:00:00Z", "2026-10-20T13:00:00Z", "KMIA", "KBOS").await;
        let loc = location(&pool, "N657JB").await.unwrap();
        assert_eq!(loc.icao.as_deref(), Some("KBOS"));
        assert_eq!(loc.airport_name.as_deref(), Some("KBOS Intl"));

        assert!(request(&pool, None, "N657JB", "KBOS", None).await.is_err(), "ya está ahí");
        let f = request(&pool, None, "N657-JB", "kjfk", None).await.unwrap();
        assert_eq!((f.from_icao.as_str(), f.to_icao.as_str()), ("KBOS", "KJFK"));
        // (24/10) Llega en 5 minutos, pero cuesta el trayecto entero.
        let q = quote(&pool, "KBOS", "KJFK", None).await.unwrap();
        assert!(q.block_minutes > REQUESTED_FERRY_MINUTES);
        assert_eq!(f.cost_total, q.cost_total);
        let wait = parse_dt(&f.arrives_at).unwrap() - parse_dt(&f.requested_at).unwrap();
        assert_eq!(wait, Duration::minutes(REQUESTED_FERRY_MINUTES));
        assert!(request(&pool, None, "N657JB", "KMIA", None).await.is_err(), "ya va en traslado");
        assert_eq!(location(&pool, "N657JB").await.unwrap().ferry.map(|x| x.id), Some(f.id));

        cancel(&pool, f.id).await.unwrap();
        let loc = location(&pool, "N657JB").await.unwrap();
        assert_eq!(loc.icao.as_deref(), Some("KBOS"));
        assert!(loc.ferry.is_none());
        assert!(cancel(&pool, f.id).await.is_err(), "ya cancelado");

        // Activación = ahora → el salto viejo no se cobra; uno posterior sí.
        assert_eq!(sync_auto_ferries(&pool).await.unwrap(), 0);
        let soon = Utc::now() + Duration::hours(1);
        let later = soon + Duration::hours(3);
        add_flight(&pool, &soon.to_rfc3339(), &later.to_rfc3339(), "KJFK", "KMIA").await;
        assert_eq!(sync_auto_ferries(&pool).await.unwrap(), 1);
        // Idempotente: regenerar no duplica.
        assert_eq!(sync_auto_ferries(&pool).await.unwrap(), 1);
        let autos = active(&pool).await.unwrap();
        assert_eq!(autos.len(), 1);
        assert_eq!((autos[0].from_icao.as_str(), autos[0].to_icao.as_str(), autos[0].source.as_str()), ("KBOS", "KJFK", "auto"));
        // Llega justo cuando sale el vuelo.
        assert_eq!(parse_dt(&autos[0].arrives_at), Some(soon));
    }

    /// (6.2d) El aterrizaje en vivo se asigna al aeropuerto más cercano (≤ 5 NM).
    #[tokio::test]
    async fn live_landing_picks_the_nearest_airport() {
        let pool = db().await;
        assert_eq!(nearest_airport(&pool, 42.37, -71.02).await.as_deref(), Some("KBOS"));
        assert_eq!(nearest_airport(&pool, 40.64, -73.78).await.as_deref(), Some("KJFK"));
        assert_eq!(nearest_airport(&pool, 41.5, -72.5).await, None, "lejos de todo");
    }

    #[test]
    fn ferry_cost_has_fuel_crew_maintenance_and_fees() {
        // KBOS → KJFK ≈ 162 NM en A320.
        let q = quote_for("KBOS", "KJFK", 162.0, Some("A320"));
        assert_eq!(q.block_minutes, 52);
        assert!(q.cost_fuel > 1000.0 && q.cost_crew > 1000.0);
        assert!(q.cost_maintenance > 0.0 && q.cost_fees > 650.0);
        let sum = q.cost_fuel + q.cost_crew + q.cost_maintenance + q.cost_fees;
        assert!((q.cost_total - sum).abs() < 1e-6);
        assert!(q.cost_total > 3000.0 && q.cost_total < 8000.0, "{}", q.cost_total);
        // Un fuselaje ancho cuesta bastante más en la misma ruta.
        assert!(quote_for("KBOS", "KJFK", 162.0, Some("B77W")).cost_total > q.cost_total * 2.0);
    }
}
