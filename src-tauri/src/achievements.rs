//! (v7) **Logros / insignias** — se calculan íntegramente a partir del historial
//! de vuelos que la app ya tiene (`flight_log`), sin telemetría nueva ni tablas
//! nuevas. Cada logro tiene niveles ascendentes (bronce → diamante) y devuelve
//! su valor actual + el nivel alcanzado, para que la UI pinte la medalla.
//!
//! Reglas de datos (importante para no inventar progreso):
//!   · Universo = `status='completed'`, excluyendo basura igual que `list_entries`
//!     (origen en 0,0 y micro-vuelos).
//!   · `landing_fpm`, `bounced`, `max_altitude_ft` y `score_*` sólo existen para
//!     vuelos hechos EN la app (`source='simconnect'`) → esos logros quedan
//!     naturalmente acotados a ellos.
//!   · `distance_nm`, `flight_time_s`, aerolínea y pasajeros existen también en
//!     los imports de VAS-ACARS (los vuelos de la aerolínea virtual).
//!   · El país sale de la tabla `airports` (`iso_country`) con UNA consulta.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::SqlitePool;

use crate::airline_economy;
use crate::airports;
use crate::flight_log::{self, FlightLogEntry};

/// Progreso de un logro, listo para la UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementProgress {
    /// Id estable — la UI resuelve nombre/descripción por i18n (`ach.<id>.*`).
    pub id: &'static str,
    /// Grupo: flights | landings | exploration | aircraft | airline | milestones.
    pub category: &'static str,
    /// Clave del icono para la medalla (mapa de paths en el front).
    pub icon: &'static str,
    /// Valor actual de la métrica.
    pub current: f64,
    /// Umbrales ascendentes de cada nivel.
    pub tiers: Vec<f64>,
    /// Índice del nivel alcanzado (0 = bronce…); `None` si aún no desbloquea.
    pub tier: Option<usize>,
    /// Siguiente umbral a alcanzar; `None` si ya está al máximo.
    pub next_threshold: Option<f64>,
    /// ¿Tiene al menos el primer nivel?
    pub unlocked: bool,
}

/// Definición estática de un logro.
struct Def {
    id: &'static str,
    category: &'static str,
    icon: &'static str,
    metric: Metric,
    tiers: &'static [f64],
}

/// Métrica que alimenta cada logro (se calcula toda de una pasada).
#[derive(Clone, Copy)]
enum Metric {
    TotalFlights,
    TotalHours,
    TotalNm,
    LongestNm,
    LongHaul,
    TotalLandings,
    ButterLandings,
    KissLandings,
    ButterStreak,
    DistinctAirports,
    DistinctCountries,
    InternationalFlights,
    DistinctTypes,
    HoursOnType,
    MaxAltitude,
    DistinctAirlines,
    SkyteamFlights,
    SkyteamHubs,
    PilotLevel,
    GradeAFlights,
    PerfectFlights,
    NightFlights,
    TotalPassengers,
}

const DEFS: &[Def] = &[
    // — Vuelos —
    Def { id: "flights_total", category: "flights", icon: "plane", metric: Metric::TotalFlights, tiers: &[10.0, 50.0, 100.0, 250.0, 500.0] },
    Def { id: "hours_total", category: "flights", icon: "clock", metric: Metric::TotalHours, tiers: &[10.0, 50.0, 100.0, 250.0, 500.0] },
    Def { id: "distance_total", category: "flights", icon: "route", metric: Metric::TotalNm, tiers: &[5_000.0, 25_000.0, 100_000.0, 500_000.0] },
    Def { id: "longest_flight", category: "flights", icon: "trend", metric: Metric::LongestNm, tiers: &[1_000.0, 2_500.0, 4_000.0, 6_000.0] },
    // — Aterrizajes (sólo vuelos hechos en la app) —
    Def { id: "landings_total", category: "landings", icon: "landing", metric: Metric::TotalLandings, tiers: &[10.0, 50.0, 100.0, 250.0] },
    Def { id: "butter_landings", category: "landings", icon: "feather", metric: Metric::ButterLandings, tiers: &[5.0, 25.0, 100.0, 250.0] },
    Def { id: "greaser_kiss", category: "landings", icon: "sparkle", metric: Metric::KissLandings, tiers: &[1.0, 10.0, 50.0] },
    Def { id: "butter_streak", category: "landings", icon: "flame", metric: Metric::ButterStreak, tiers: &[3.0, 5.0, 10.0, 20.0] },
    // — Exploración —
    Def { id: "distinct_airports", category: "exploration", icon: "pin", metric: Metric::DistinctAirports, tiers: &[10.0, 25.0, 50.0, 100.0, 200.0] },
    Def { id: "distinct_countries", category: "exploration", icon: "globe", metric: Metric::DistinctCountries, tiers: &[3.0, 10.0, 25.0, 50.0] },
    Def { id: "international_flights", category: "exploration", icon: "border", metric: Metric::InternationalFlights, tiers: &[1.0, 25.0, 100.0] },
    // — Aviones —
    Def { id: "distinct_types", category: "aircraft", icon: "layers", metric: Metric::DistinctTypes, tiers: &[3.0, 10.0, 25.0] },
    Def { id: "hours_on_type", category: "aircraft", icon: "wrench", metric: Metric::HoursOnType, tiers: &[25.0, 100.0, 500.0] },
    Def { id: "max_altitude", category: "aircraft", icon: "alt", metric: Metric::MaxAltitude, tiers: &[30_000.0, 38_000.0, 43_000.0] },
    // — Aerolínea —
    Def { id: "distinct_airlines", category: "airline", icon: "swap", metric: Metric::DistinctAirlines, tiers: &[3.0, 10.0, 25.0] },
    Def { id: "skyteam_flights", category: "airline", icon: "star", metric: Metric::SkyteamFlights, tiers: &[1.0, 25.0, 100.0] },
    Def { id: "skyteam_hubs", category: "airline", icon: "building", metric: Metric::SkyteamHubs, tiers: &[1.0, 5.0, 10.0, 15.0] },
    // — Hitos —
    Def { id: "long_haul_flights", category: "milestones", icon: "takeoff", metric: Metric::LongHaul, tiers: &[1.0, 10.0, 50.0] },
    Def { id: "pilot_level", category: "milestones", icon: "award", metric: Metric::PilotLevel, tiers: &[5.0, 10.0, 25.0, 50.0] },
    Def { id: "grade_a_flights", category: "milestones", icon: "trophy", metric: Metric::GradeAFlights, tiers: &[1.0, 10.0, 50.0, 100.0] },
    Def { id: "perfect_flights", category: "milestones", icon: "check", metric: Metric::PerfectFlights, tiers: &[1.0, 10.0, 50.0] },
    Def { id: "night_flights", category: "milestones", icon: "moon", metric: Metric::NightFlights, tiers: &[1.0, 10.0, 50.0] },
    Def { id: "total_passengers", category: "milestones", icon: "users", metric: Metric::TotalPassengers, tiers: &[1_000.0, 10_000.0, 100_000.0] },
];

/// ICAO de aerolíneas miembro de SkyTeam (la alianza del usuario).
const SKYTEAM_AIRLINES: &[&str] = &[
    "AFR", // Air France
    "KLM", // KLM
    "DAL", // Delta
    "KAL", // Korean Air
    "AMX", // Aeroméxico
    "AEA", // Air Europa
    "ARG", // Aerolíneas Argentinas
    "AFL", // Aeroflot
    "CAL", // China Airlines
    "CES", // China Eastern
    "CSA", // Czech Airlines
    "GIA", // Garuda Indonesia
    "ITY", // ITA Airways
    "KQA", // Kenya Airways
    "MEA", // Middle East Airlines
    "SVA", // Saudia
    "ROT", // TAROM
    "HVN", // Vietnam Airlines
    "CXA", // Xiamen Air
    "VIR", // Virgin Atlantic
    "SAS", // SAS
];

/// Hubs principales de las aerolíneas SkyTeam (ICAO de aeropuerto).
const SKYTEAM_HUBS: &[&str] = &[
    "EHAM", // Ámsterdam — KLM
    "LFPG", "LFPO", // París — Air France
    "KATL", "KDTW", "KMSP", "KSLC", "KJFK", "KLAX", "KSEA", "KBOS", // Delta
    "RKSI", // Seúl — Korean Air
    "MMMX", // Ciudad de México — Aeroméxico
    "LEMD", // Madrid — Air Europa
    "SAEZ", // Buenos Aires — Aerolíneas Argentinas
    "UUEE", // Moscú — Aeroflot
    "RCTP", // Taipéi — China Airlines
    "ZSPD", // Shanghái — China Eastern
    "WIII", // Yakarta — Garuda
    "LIRF", // Roma — ITA
    "HKJK", // Nairobi — Kenya Airways
    "OLBA", // Beirut — MEA
    "OEJN", "OERK", // Yeda / Riad — Saudia
    "LROP", // Bucarest — TAROM
    "VVTS", "VVNB", // Ho Chi Minh / Hanói — Vietnam Airlines
    "ZSAM", // Xiamen — Xiamen Air
    "EGLL", // Londres — Virgin Atlantic
    "EKCH", "ESSA", "ENGM", // Copenhague / Estocolmo / Oslo — SAS
    "LKPR", // Praga — Czech Airlines
];

/// ¿La fila es "basura"? Mismo criterio que la limpieza del log: origen en el
/// punto (0,0) del mapa, o micro-vuelo de prueba.
fn is_junk(e: &FlightLogEntry) -> bool {
    // (v7 fix) Los imports del logbook de MSFS traen origen (0,0) como
    // placeholder pero SÍ tienen ICAOs válidos y cuentan en FlightBook/Hangar,
    // así que NO son basura: excluir el chequeo (0,0) para esa fuente. Antes,
    // todo vuelo importado del logbook quedaba fuera de TODOS los logros.
    if e.source != "msfs-logbook" && e.origin_lat.abs() < 0.01 && e.origin_lon.abs() < 0.01 {
        return true;
    }
    let tiny_dist = e.distance_nm.map(|d| d < 2.0).unwrap_or(false);
    let tiny_time = e.flight_time_s.map(|s| s < 60).unwrap_or(false);
    tiny_dist && tiny_time
}

/// Hora local aproximada = hora UTC + longitud/15. Sirve para "vuelo nocturno"
/// sin depender de una zona horaria real (que no guardamos).
fn is_night(e: &FlightLogEntry) -> bool {
    // Los imports del logbook de MSFS no traen coordenadas fiables.
    if e.source == "msfs-logbook" {
        return false;
    }
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(e.started_at.trim()) else {
        return false;
    };
    use chrono::Timelike;
    let utc_h = dt.naive_utc().hour() as f64 + dt.naive_utc().minute() as f64 / 60.0;
    let local = (utc_h + e.origin_lon / 15.0).rem_euclid(24.0);
    local >= 21.0 || local < 6.0
}

/// Calcula el progreso de TODOS los logros de una sola pasada por el historial.
pub async fn compute_achievements(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<AchievementProgress>> {
    let all = flight_log::list_entries(pool).await.unwrap_or_default();
    let entries: Vec<FlightLogEntry> = all
        .into_iter()
        .filter(|e| e.status == "completed" && !is_junk(e))
        .collect();

    // Mapa nombre→ICAO aprendido, para agrupar aerolíneas duplicadas.
    let learned = airline_economy::learn_airline_icaos(&entries);
    let skyteam: HashSet<&str> = SKYTEAM_AIRLINES.iter().copied().collect();
    let hubs: HashSet<&str> = SKYTEAM_HUBS.iter().copied().collect();

    let mut total_flights = 0f64;
    let mut total_time_s = 0f64;
    let mut total_nm = 0f64;
    let mut longest_nm = 0f64;
    let mut long_haul = 0f64;
    let mut total_landings = 0f64;
    let mut butter = 0f64;
    let mut kiss = 0f64;
    let mut max_alt = 0f64;
    let mut grade_a = 0f64;
    let mut perfect = 0f64;
    let mut night = 0f64;
    let mut passengers = 0f64;
    let mut skyteam_flights = 0f64;
    let mut airports_seen: HashSet<String> = HashSet::new();
    let mut types_seen: HashSet<String> = HashSet::new();
    let mut time_by_type: HashMap<String, f64> = HashMap::new();
    let mut airlines_seen: HashSet<String> = HashSet::new();
    // (started_at, fpm) para la racha de aterrizajes.
    let mut landing_seq: Vec<(String, i64)> = Vec::new();

    for e in &entries {
        total_flights += 1.0;
        if let Some(s) = e.flight_time_s {
            total_time_s += s as f64;
        }
        if let Some(d) = e.distance_nm {
            total_nm += d;
            if d > longest_nm {
                longest_nm = d;
            }
            if d >= 3000.0 {
                long_haul += 1.0;
            }
        }
        for icao in [e.origin_icao.as_deref(), e.destination_icao.as_deref()]
            .into_iter()
            .flatten()
        {
            let up = icao.trim().to_ascii_uppercase();
            if !up.is_empty() {
                airports_seen.insert(up);
            }
        }
        // Tipo de avión (normalizado) + horas por tipo.
        let raw_model = e
            .aircraft_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(e.aircraft_title.as_deref());
        if let Some(raw) = raw_model.filter(|s| !s.trim().is_empty()) {
            let key = flight_log::normalize_model(raw);
            if !key.is_empty() {
                types_seen.insert(key.clone());
                *time_by_type.entry(key).or_insert(0.0) +=
                    e.flight_time_s.unwrap_or(0) as f64;
            }
        }
        // Aerolínea canónica → distintas + vuelos SkyTeam.
        if let Some(canon) = airline_economy::canonical_airline(
            e.airline_icao.as_deref(),
            e.callsign.as_deref(),
            e.aircraft_airline.as_deref(),
            e.aircraft_title.as_deref(),
            &learned,
        ) {
            airlines_seen.insert(canon.key.clone());
            if let Some(icao) = canon.icao.as_deref() {
                if skyteam.contains(icao.to_ascii_uppercase().as_str()) {
                    skyteam_flights += 1.0;
                }
            }
        }
        // Aterrizajes (sólo vuelos con fpm medido).
        if let Some(fpm) = e.landing_fpm {
            total_landings += 1.0;
            if flight_log::landing_grade(fpm) == "butter" {
                butter += 1.0;
            }
            if fpm > -100 && fpm <= 0 && !e.bounced {
                kiss += 1.0;
            }
            landing_seq.push((e.started_at.clone(), fpm));
        }
        if let Some(a) = e.max_altitude_ft {
            if a as f64 > max_alt {
                max_alt = a as f64;
            }
        }
        if e.score_grade.as_deref() == Some("A") {
            grade_a += 1.0;
        }
        if let (Some(t), Some(m)) = (e.score_total, e.score_max) {
            if m > 0 && t == m {
                perfect += 1.0;
            }
        }
        if is_night(e) {
            night += 1.0;
        }
        if let Some(p) = e.passengers {
            passengers += p as f64;
        }
    }

    // Racha más larga de aterrizajes sin ninguno "duro".
    landing_seq.sort_by(|a, b| a.0.cmp(&b.0));
    let mut streak = 0f64;
    let mut run = 0f64;
    for (_, fpm) in &landing_seq {
        if flight_log::landing_grade(*fpm) == "hard" {
            run = 0.0;
        } else {
            run += 1.0;
            if run > streak {
                streak = run;
            }
        }
    }

    // Países + vuelos internacionales — UNA consulta a `airports`.
    let icaos: Vec<String> = airports_seen.iter().cloned().collect();
    let briefs = airports::lookup_airports(pool, &icaos).await.unwrap_or_default();
    let country_of: HashMap<String, String> = briefs
        .into_iter()
        .filter_map(|b| {
            b.iso_country
                .filter(|c| !c.trim().is_empty())
                .map(|c| (b.icao.to_ascii_uppercase(), c.to_ascii_uppercase()))
        })
        .collect();
    let countries: HashSet<&String> = country_of.values().collect();
    let mut international = 0f64;
    for e in &entries {
        let (Some(o), Some(d)) = (e.origin_icao.as_deref(), e.destination_icao.as_deref())
        else {
            continue;
        };
        let (co, cd) = (
            country_of.get(&o.trim().to_ascii_uppercase()),
            country_of.get(&d.trim().to_ascii_uppercase()),
        );
        if let (Some(a), Some(b)) = (co, cd) {
            if a != b {
                international += 1.0;
            }
        }
    }
    let hubs_visited = airports_seen
        .iter()
        .filter(|a| hubs.contains(a.as_str()))
        .count() as f64;

    let level = flight_log::pilot_profile(pool)
        .await
        .map(|p| p.level as f64)
        .unwrap_or(0.0);

    let hours_on_type = time_by_type.values().cloned().fold(0.0, f64::max) / 3600.0;

    let value_of = |m: Metric| -> f64 {
        match m {
            Metric::TotalFlights => total_flights,
            Metric::TotalHours => total_time_s / 3600.0,
            Metric::TotalNm => total_nm,
            Metric::LongestNm => longest_nm,
            Metric::LongHaul => long_haul,
            Metric::TotalLandings => total_landings,
            Metric::ButterLandings => butter,
            Metric::KissLandings => kiss,
            Metric::ButterStreak => streak,
            Metric::DistinctAirports => airports_seen.len() as f64,
            Metric::DistinctCountries => countries.len() as f64,
            Metric::InternationalFlights => international,
            Metric::DistinctTypes => types_seen.len() as f64,
            Metric::HoursOnType => hours_on_type,
            Metric::MaxAltitude => max_alt,
            Metric::DistinctAirlines => airlines_seen.len() as f64,
            Metric::SkyteamFlights => skyteam_flights,
            Metric::SkyteamHubs => hubs_visited,
            Metric::PilotLevel => level,
            Metric::GradeAFlights => grade_a,
            Metric::PerfectFlights => perfect,
            Metric::NightFlights => night,
            Metric::TotalPassengers => passengers,
        }
    };

    let out = DEFS
        .iter()
        .map(|d| {
            let current = value_of(d.metric);
            // Nivel alcanzado = último umbral superado.
            let mut tier: Option<usize> = None;
            for (i, t) in d.tiers.iter().enumerate() {
                if current >= *t {
                    tier = Some(i);
                }
            }
            let next_threshold = match tier {
                Some(i) if i + 1 < d.tiers.len() => Some(d.tiers[i + 1]),
                Some(_) => None,
                None => d.tiers.first().copied(),
            };
            AchievementProgress {
                id: d.id,
                category: d.category,
                icon: d.icon,
                current,
                tiers: d.tiers.to_vec(),
                tier,
                next_threshold,
                unlocked: tier.is_some(),
            }
        })
        .collect();
    Ok(out)
}
