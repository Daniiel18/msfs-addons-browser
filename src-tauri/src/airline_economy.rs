//! (v6.1) Economía de aerolíneas — módulo backend, **sin pantalla**.
//!
//! Idea: cada aerolínea arranca con un VALOR BASE realista (su "banco":
//! market-cap real aproximado 2024 o un estimado en miles de millones — nunca
//! cifras de juguete tipo 600 USD), y a partir de ahí se lleva una contabilidad
//! de lo que GANA (billetes por clase, ancillaries: equipaje/snacks) menos lo
//! que PIERDE (combustible, catering, handling, mantenimiento, tasas/impuestos
//! y fees de GSX). El resultado se acumula por aerolínea conforme se vuela.
//!
//! El módulo **consume la data actual** de `flight_log`: en cada consulta se
//! recalcula el P&L por vuelo (idempotente, cacheado en `flight_pnl`) y se
//! agrega el ledger por aerolínea (`airline_economy`). Así "se va actualizando
//! cada aerolínea con la que vaya volando" sin tocar el watcher de vuelo.
//!
//! Integración GSX: lee los recibos reales que GSX deja en
//! `%APPDATA%\Virtuali\GSX\Receipts\{Fuel,Catering,Handling}`. La ruta se
//! resuelve por la variable de entorno `APPDATA` del usuario (NO hardcodeada),
//! así funciona igual para otra máquina/usuario. El nombre del archivo trae
//! `<timestamp>_<ICAO>_<matrícula>` y el cuerpo un `Total$ <importe>`, así que
//! se casa cada recibo con su vuelo por matrícula + aeropuerto + ventana de
//! tiempo. Si hay recibo real, manda sobre el estimado; si no, se estima.

use crate::flight_log::{self, FlightLogEntry};
use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ───────────────────────────── Perfiles de aerolínea ─────────────────────────

#[derive(Clone, Copy)]
struct AirlineProfile {
    name: &'static str,
    /// Valor base USD ("banco") — market-cap real aprox. 2024 o estimado.
    valuation_usd: f64,
    lowcost: bool,
    /// Operador de CARGA (sin pasajeros). Cambia todo el modelo económico.
    cargo: bool,
}

const B: f64 = 1_000_000_000.0; // mil millones, para legibilidad de la tabla

/// Tabla de aerolíneas conocidas, indexada por código ICAO (3 letras).
/// Valoraciones realistas (market-cap/estimado). Marca `lowcost` las LCC.
static KNOWN: Lazy<HashMap<&'static str, AirlineProfile>> = Lazy::new(|| {
    let mut m = HashMap::new();
    let mut add = |icao: &'static str, name: &'static str, val: f64, lc: bool| {
        m.insert(icao, AirlineProfile { name, valuation_usd: val, lowcost: lc, cargo: false });
    };
    // ── Norteamérica (legacy) ──
    add("DAL", "Delta Air Lines", 32.0 * B, false);
    add("UAL", "United Airlines", 16.0 * B, false);
    add("AAL", "American Airlines", 9.0 * B, false);
    add("ASA", "Alaska Airlines", 5.0 * B, false);
    add("ACA", "Air Canada", 5.0 * B, false);
    add("AMX", "Aeroméxico", 3.0 * B, false);
    add("HAL", "Hawaiian Airlines", 1.0 * B, false);
    // ── Norteamérica (low-cost / ULCC) ──
    add("SWA", "Southwest Airlines", 18.0 * B, true);
    add("JBU", "JetBlue Airways", 2.0 * B, true);
    add("NKS", "Spirit Airlines", 1.0 * B, true);
    add("FFT", "Frontier Airlines", 1.5 * B, true);
    add("AAY", "Allegiant Air", 1.5 * B, true);
    add("WJA", "WestJet", 2.5 * B, true);
    add("SCX", "Sun Country", 1.0 * B, true);
    add("VOI", "Volaris", 1.5 * B, true);
    // ── Europa (legacy / red) ──
    add("DLH", "Lufthansa", 8.0 * B, false);
    add("BAW", "British Airways", 14.0 * B, false);
    add("IBE", "Iberia", 6.0 * B, false);
    add("AFR", "Air France", 4.0 * B, false);
    add("KLM", "KLM", 4.0 * B, false);
    add("SWR", "SWISS", 4.0 * B, false);
    add("AUA", "Austrian Airlines", 1.5 * B, false);
    add("TAP", "TAP Air Portugal", 2.0 * B, false);
    add("SAS", "SAS Scandinavian", 1.5 * B, false);
    add("FIN", "Finnair", 1.5 * B, false);
    add("AZA", "ITA Airways", 1.5 * B, false);
    add("THY", "Turkish Airlines", 12.0 * B, false);
    add("AEE", "Aegean Airlines", 1.0 * B, false);
    add("VIR", "Virgin Atlantic", 2.0 * B, false);
    add("AFL", "Aeroflot", 3.0 * B, false);
    // ── Europa (low-cost) ──
    add("RYR", "Ryanair", 25.0 * B, true);
    add("EZY", "easyJet", 5.0 * B, true);
    add("WZZ", "Wizz Air", 3.0 * B, true);
    add("VLG", "Vueling", 2.0 * B, true);
    add("EWG", "Eurowings", 1.5 * B, true);
    add("TRA", "Transavia", 1.0 * B, true);
    add("NAX", "Norwegian", 1.5 * B, true);
    add("EXS", "Jet2", 4.0 * B, true);
    add("VOE", "Volotea", 0.8 * B, true);
    // ── Oriente Medio / Asia / Oceanía ──
    add("UAE", "Emirates", 30.0 * B, false);
    add("QTR", "Qatar Airways", 20.0 * B, false);
    add("ETD", "Etihad Airways", 10.0 * B, false);
    add("SIA", "Singapore Airlines", 14.0 * B, false);
    add("CPA", "Cathay Pacific", 8.0 * B, false);
    add("ANA", "All Nippon Airways", 9.0 * B, false);
    add("JAL", "Japan Airlines", 8.0 * B, false);
    add("QFA", "Qantas", 7.0 * B, false);
    add("ANZ", "Air New Zealand", 2.0 * B, false);
    add("KAL", "Korean Air", 6.0 * B, false);
    add("CCA", "Air China", 12.0 * B, false);
    add("CES", "China Eastern", 8.0 * B, false);
    add("CSN", "China Southern", 9.0 * B, false);
    add("EVA", "EVA Air", 5.0 * B, false);
    add("THA", "Thai Airways", 3.0 * B, false);
    add("MAS", "Malaysia Airlines", 2.0 * B, false);
    add("UAEX", "flydubai", 2.0 * B, true);
    add("FDB", "flydubai", 2.0 * B, true);
    add("AXM", "AirAsia", 2.0 * B, true);
    add("JST", "Jetstar", 2.0 * B, true);
    add("IGO", "IndiGo", 20.0 * B, true);
    add("SVA", "Saudia", 8.0 * B, false); // ICAO correcto de Saudia
    add("PGT", "Pegasus", 3.0 * B, true);
    add("SCO", "Scoot", 1.0 * B, true);
    add("VJC", "VietJet Air", 2.0 * B, true);
    add("GIA", "Garuda Indonesia", 2.0 * B, false);
    add("CTV", "Citilink", 1.0 * B, true);
    add("TTW", "Tigerair Taiwan", 0.8 * B, true);
    add("KQA", "Kenya Airways", 0.5 * B, false);
    add("MAU", "Air Mauritius", 0.4 * B, false);
    add("AIC", "Air India", 9.0 * B, false);
    // ── Latinoamérica ──
    add("LAN", "LATAM Airlines", 8.0 * B, false);
    add("TAM", "LATAM Brasil", 8.0 * B, false);
    add("AVA", "Avianca", 2.0 * B, false);
    add("CMP", "Copa Airlines", 4.0 * B, false);
    add("GLO", "GOL", 1.0 * B, true);
    add("AZU", "Azul", 1.5 * B, true);
    add("VOC", "Viva Aerobus", 1.0 * B, true);
    add("ARG", "Aerolíneas Argentinas", 1.0 * B, false);
    // ── África ──
    add("ETH", "Ethiopian Airlines", 3.0 * B, false);
    add("SAA", "South African Airways", 1.0 * B, false);
    add("MSR", "EgyptAir", 2.0 * B, false);
    add("RAM", "Royal Air Maroc", 1.5 * B, false);

    // ── CARGA ── (sin pasajeros; ingreso por flete)
    let mut addc = |icao: &'static str, name: &'static str, val: f64| {
        m.insert(icao, AirlineProfile { name, valuation_usd: val, lowcost: false, cargo: true });
    };
    addc("FDX", "FedEx Express", 60.0 * B);
    addc("UPS", "UPS Airlines", 90.0 * B);
    addc("GTI", "Atlas Air", 3.0 * B);
    addc("GEC", "Lufthansa Cargo", 2.0 * B);
    addc("CLX", "Cargolux", 2.0 * B);
    addc("BOX", "AeroLogic", 1.5 * B);
    addc("CKS", "Kalitta Air", 1.0 * B);
    addc("ABW", "AirBridgeCargo", 1.0 * B);
    addc("CKK", "China Cargo", 2.0 * B);
    addc("GSS", "Cargolux Italia", 0.5 * B);
    addc("MPH", "Martinair Cargo", 0.5 * B);
    addc("LCO", "LATAM Cargo", 1.0 * B);
    addc("PAC", "Polar Air Cargo", 1.0 * B);
    addc("WGN", "Western Global", 0.5 * B);
    addc("NCA", "Nippon Cargo", 1.0 * B);
    addc("DHK", "DHL Air", 5.0 * B);
    addc("ABX", "ABX Air", 1.0 * B);
    m
});

/// Palabras clave para casar el nombre libre de la aerolínea (`ATC AIRLINE`)
/// con un ICAO de la tabla `KNOWN` cuando el vuelo no trae `airline_icao`.
static NAME_TO_ICAO: Lazy<Vec<(&'static str, &'static str)>> = Lazy::new(|| {
    vec![
        ("delta", "DAL"), ("united", "UAL"), ("american", "AAL"),
        ("alaska", "ASA"), ("air canada", "ACA"), ("aeromex", "AMX"),
        ("hawaiian", "HAL"), ("southwest", "SWA"), ("jetblue", "JBU"),
        ("spirit", "NKS"), ("frontier", "FFT"), ("allegiant", "AAY"),
        ("westjet", "WJA"), ("sun country", "SCX"), ("volaris", "VOI"),
        ("lufthansa", "DLH"), ("british", "BAW"), ("iberia", "IBE"),
        ("air france", "AFR"), ("klm", "KLM"), ("swiss", "SWR"),
        ("austrian", "AUA"), ("tap", "TAP"), ("scandinav", "SAS"),
        ("finnair", "FIN"), ("ita ", "AZA"), ("alitalia", "AZA"),
        ("turkish", "THY"), ("aegean", "AEE"), ("virgin atlantic", "VIR"),
        ("aeroflot", "AFL"), ("ryanair", "RYR"), ("easyjet", "EZY"),
        ("wizz", "WZZ"), ("vueling", "VLG"), ("eurowings", "EWG"),
        ("transavia", "TRA"), ("norwegian", "NAX"), ("jet2", "EXS"),
        ("volotea", "VOE"), ("emirates", "UAE"), ("qatar", "QTR"),
        ("etihad", "ETD"), ("singapore", "SIA"), ("cathay", "CPA"),
        ("nippon", "ANA"), ("ana ", "ANA"), ("japan airlines", "JAL"),
        ("qantas", "QFA"), ("air new zealand", "ANZ"), ("korean", "KAL"),
        ("air china", "CCA"), ("china eastern", "CES"), ("china southern", "CSN"),
        ("eva air", "EVA"), ("thai", "THA"), ("malaysia", "MAS"),
        ("flydubai", "FDB"), ("airasia", "AXM"), ("jetstar", "JST"),
        ("indigo", "IGO"), ("saudia", "SVA"), ("saudi", "SVA"),
        ("pegasus", "PGT"), ("scoot", "SCO"), ("vietjet", "VJC"),
        ("latam", "LAN"), ("avianca", "AVA"), ("copa", "CMP"),
        ("gol", "GLO"), ("azul", "AZU"), ("viva aerobus", "VOC"),
        ("aerolineas argentinas", "ARG"), ("argentinas", "ARG"),
        ("ethiopian", "ETH"), ("south african", "SAA"), ("egyptair", "MSR"),
        ("royal air maroc", "RAM"),
        // Añadidas en la 2ª pasada de detección:
        ("citilink", "CTV"), ("garuda", "GIA"), ("kenya", "KQA"),
        ("mauritius", "MAU"), ("tigerair", "TTW"), ("air india", "AIC"),
        ("china cargo", "CKK"), ("fedex", "FDX"), ("federal express", "FDX"),
        ("ups ", "UPS"), ("cargolux", "CLX"), ("atlas air", "GTI"),
    ]
});

/// Detecta aerolíneas low-cost por palabras clave en el nombre, para las que
/// no están en `KNOWN`. Las LCC dan el mismo asiento pero cobran extras
/// (equipaje, asiento, embarque prioritario): ese es su ancillary fuerte.
fn detect_lowcost(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const LCC: &[&str] = &[
        "low cost", "low-cost", "lowcost", "express", "ryanair", "easyjet",
        "wizz", "vueling", "eurowings", "transavia", "norwegian", "jet2",
        "volotea", "southwest", "jetblue", "spirit", "frontier", "allegiant",
        "sun country", "volaris", "viva", "flydubai", "airasia", "air asia",
        "jetstar", "indigo", "pegasus", "scoot", "vietjet", "gol", "azul",
        "flair", "swoop", "lion air", "cebu", "nok", "peach", "wow", "level",
        "french bee", "norse", "play", "bonza", "breeze", "avelo", "wingo",
        "smartwings", "pobeda", "flynas", "salam", "akasa", "spicejet",
    ];
    LCC.iter().any(|k| n.contains(k))
}

/// Aerolínea resuelta para un vuelo: clave de ledger + perfil económico.
struct Resolved {
    key: String,
    icao: Option<String>,
    name: String,
    valuation: f64,
    lowcost: bool,
    cargo: bool,
}

/// Deriva el ICAO de 3 letras desde el callsign ("DAL115" → "DAL").
fn callsign_icao(cs: Option<&str>) -> Option<String> {
    let cs = cs?.trim();
    let alpha: String = cs.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if alpha.len() == 3 {
        Some(alpha.to_ascii_uppercase())
    } else {
        None
    }
}

/// Aerolínea canónica de un vuelo: clave de agrupación + ICAO + nombre a
/// mostrar. `key` colapsa variantes (ICAO, callsign, livery con nombre
/// parecido) en UNA sola aerolínea — la misma lógica que usan los tags del
/// FlightBook. Pública para que el Hangar y las Finanzas agrupen IGUAL.
pub struct CanonAirline {
    pub key: String,
    pub icao: Option<String>,
    pub name: String,
}

/// Tokens de dev/livery/modelo que NUNCA son una aerolínea. Se ignoran al
/// escanear strings de livery libres ("NordicPaints FlybyWire A320neo SAS…").
fn is_noise_token(up: &str) -> bool {
    const NOISE: &[&str] = &[
        "PMDG", "FENIX", "FBW", "FLYBYWIRE", "FLY", "WIRE", "INIBUILDS", "INI",
        "AEROSOFT", "MADDOG", "LEONARDO", "TFDI", "CAPTAINSIM", "JUSTFLIGHT",
        "HEADWIND", "HORIZON", "NORDICPAINTS", "PAINTS", "PAINT", "LIVERY",
        "DEFAULT", "ASOBO", "MICROSOFT", "BOEING", "AIRBUS", "EMBRAER", "NEO",
        "CEO", "PACK", "PROJECT", "OPENBETA", "BETA", "MOD", "REPAINT",
        // Variantes/sufijos de pintura que NO son la aerolínea:
        "STANDARD", "STD", "OLD", "NEW", "RETRO", "SPECIAL", "LEGACY", "MODERN",
        "OC", "NC", "COLORS", "COLOURS", "SHARKLET", "SHARKLETS", "WINGLET",
        "WINGLETS", "WL", "SCIMITAR", "FICTIONAL", "FAKE", "VA", "VIRTUAL",
        "HD", "4K", "8K", "ULTRA", "V1", "V2", "V3", "REV",
    ];
    NOISE.contains(&up)
}

/// ¿El token parece una MATRÍCULA? (lleva guion con letras, o prefijo país +
/// alfanumérico). Ej. "PK-GQN", "SE-DOY", "5Y-KZG", "N827DN". Para descartarla
/// del nombre de la aerolínea.
fn looks_like_registration(tok: &str) -> bool {
    let up = tok.to_ascii_uppercase();
    if up.contains('-') && up.len() >= 4 && up.len() <= 8 {
        return true;
    }
    // N-number USA: N + dígitos/letras.
    if up.starts_with('N') && up.len() >= 4 && up[1..].chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

/// Escanea un string de livery libre buscando un ICAO de aerolínea conocido
/// (token de 3 letras en KNOWN) o un nombre por palabra clave. Ignora tokens
/// de dev/modelo/matrícula (los que llevan dígitos o están en la lista de
/// ruido). Devuelve el ICAO canónico. Esto es lo que arregla que
/// "PMDG 737-900ER DAL (N827DN…)" se cuente como Delta, no como livery.
fn extract_icao_from_text(text: &str) -> Option<&'static str> {
    let low = text.to_ascii_lowercase();
    // 1) Nombre por palabra clave (cubre nombres largos: "scandinavian"…).
    if let Some((_, ic)) = NAME_TO_ICAO.iter().find(|(kw, _)| low.contains(kw)) {
        return Some(ic);
    }
    // 2) Token de 3 letras que sea un ICAO de aerolínea conocido.
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if raw.len() != 3 || !raw.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let up = raw.to_ascii_uppercase();
        if is_noise_token(&up) {
            continue;
        }
        if let Some((ic, _)) = KNOWN.get_key_value(up.as_str()) {
            return Some(ic);
        }
    }
    None
}

/// Limpia un nombre de livery para mostrarlo cuando NO se reconoció ninguna
/// aerolínea: quita prefijos de dev, modelos, matrículas y paréntesis.
fn clean_livery_name(text: &str) -> String {
    // Corta en el primer paréntesis/corchete/comilla (ahí suele ir matrícula
    // o variante: "Citilink 'Standard' PK-GQN" → "Citilink").
    let head = text
        .split(|c| c == '(' || c == '[' || c == '\'' || c == '"')
        .next()
        .unwrap_or(text);
    let kept: Vec<String> = head
        .split_whitespace()
        // Quita puntuación de los bordes de cada token ('Standard' → Standard).
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string())
        .filter(|t| !t.is_empty())
        .filter(|t| {
            let up = t.to_ascii_uppercase();
            !is_noise_token(&up)
                && !up.chars().any(|c| c.is_ascii_digit())
                && !looks_like_registration(t)
        })
        .collect();
    let joined = kept.join(" ").trim().to_string();
    if joined.is_empty() {
        text.trim().to_string()
    } else {
        joined
    }
}

/// Resuelve el ICAO "directo" de un vuelo (sin el mapa aprendido): campo →
/// callsign → token conocido en el nombre/título.
fn direct_icao(
    airline_icao: Option<&str>,
    callsign: Option<&str>,
    aircraft_airline: Option<&str>,
    aircraft_title: Option<&str>,
) -> Option<String> {
    airline_icao
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_uppercase())
        .or_else(|| callsign_icao(callsign))
        .or_else(|| aircraft_airline.and_then(extract_icao_from_text).map(|s| s.to_string()))
        .or_else(|| aircraft_title.and_then(extract_icao_from_text).map(|s| s.to_string()))
}

/// (v6.1) APRENDE el ICAO de cada aerolínea a partir de los propios vuelos:
/// para cada nombre de livery limpiado que ALGUNA vez aparece junto a un ICAO
/// resoluble, recuerda ese ICAO (el más frecuente). Así, aunque OTRO vuelo de
/// la misma aerolínea sólo traiga el nombre (sin ICAO), se agrupa con el mismo
/// ICAO en vez de crear una aerolínea duplicada. Esto colapsa, sin tabla
/// gigante, los casos "Kenya Airways" (con KQA) + "Kenya Airways" (sin ICAO).
pub fn learn_airline_icaos(entries: &[FlightLogEntry]) -> HashMap<String, String> {
    // nombre_limpio_UPPER -> (icao -> conteo)
    let mut votes: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for e in entries {
        let Some(icao) = direct_icao(
            e.airline_icao.as_deref(),
            e.callsign.as_deref(),
            e.aircraft_airline.as_deref(),
            e.aircraft_title.as_deref(),
        ) else {
            continue;
        };
        let free = e
            .aircraft_airline
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or(e.aircraft_title.as_deref().map(str::trim).filter(|s| !s.is_empty()));
        if let Some(free) = free {
            let key = clean_livery_name(free).to_ascii_uppercase();
            if !key.is_empty() {
                *votes.entry(key).or_default().entry(icao).or_insert(0) += 1;
            }
        }
    }
    votes
        .into_iter()
        .filter_map(|(name, by_icao)| {
            by_icao
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(ic, _)| (name, ic))
        })
        .collect()
}

/// Resuelve la aerolínea canónica de un vuelo. `learned` es el mapa de
/// nombre→ICAO aprendido de toda la base (ver `learn_airline_icaos`) para
/// colapsar duplicados; pasa `&HashMap::new()` si no lo tienes.
pub fn canonical_airline(
    airline_icao: Option<&str>,
    callsign: Option<&str>,
    aircraft_airline: Option<&str>,
    aircraft_title: Option<&str>,
    learned: &HashMap<String, String>,
) -> Option<CanonAirline> {
    // Nombre libre de respaldo (limpiado) si no reconocemos ICAO.
    let free_name = aircraft_airline
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(aircraft_title.map(str::trim).filter(|s| !s.is_empty()));

    let icao = direct_icao(airline_icao, callsign, aircraft_airline, aircraft_title)
        // Último recurso: el ICAO aprendido para este nombre de livery.
        .or_else(|| {
            free_name
                .map(|f| clean_livery_name(f).to_ascii_uppercase())
                .and_then(|k| learned.get(&k).cloned())
        });

    match (icao, free_name) {
        (Some(ic), free) => {
            // Nombre a mostrar: el canónico conocido manda (aerolínea
            // principal); si no es conocida, el nombre de livery limpiado.
            let display = KNOWN
                .get(ic.as_str())
                .map(|p| p.name.to_string())
                .or_else(|| free.map(clean_livery_name))
                .unwrap_or_else(|| ic.clone());
            Some(CanonAirline { key: ic.clone(), icao: Some(ic), name: display })
        }
        (None, Some(free)) => {
            // Sin ICAO reconocible: agrupamos por el nombre limpiado para no
            // partir "X (REG1)" y "X (REG2)" en dos.
            let clean = clean_livery_name(free);
            Some(CanonAirline {
                key: clean.to_ascii_uppercase(),
                icao: None,
                name: clean,
            })
        }
        (None, None) => None,
    }
}

/// Resuelve la aerolínea de un vuelo y le añade el perfil económico
/// (valor base + low-cost). Reusa `canonical_airline` para que la
/// agrupación sea idéntica a la del Hangar.
fn resolve_airline(e: &FlightLogEntry, learned: &HashMap<String, String>) -> Option<Resolved> {
    let canon = canonical_airline(
        e.airline_icao.as_deref(),
        e.callsign.as_deref(),
        e.aircraft_airline.as_deref(),
        e.aircraft_title.as_deref(),
        learned,
    )?;
    let known = canon.icao.as_deref().and_then(|ic| KNOWN.get(ic));
    let (valuation, lowcost) = known
        .map(|p| (p.valuation_usd, p.lowcost))
        .unwrap_or_else(|| {
            // Desconocida → estimación realista (miles de millones).
            let lc = detect_lowcost(&canon.name);
            (if lc { 2.0 * B } else { 4.0 * B }, lc)
        });
    // Carga: aerolínea de carga conocida, o vuelo con flete y sin pasajeros
    // (SimBrief no trae pax → es obvio que es carga).
    let cargo = known.map(|p| p.cargo).unwrap_or(false)
        || (e.passengers.unwrap_or(0) <= 0 && e.cargo_kg.unwrap_or(0) > 0);
    Some(Resolved {
        key: canon.key,
        icao: canon.icao,
        name: canon.name,
        valuation,
        lowcost,
        cargo,
    })
}

// ─────────────────────────── Clasificación de avión ──────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Size {
    Regional,
    Narrow,
    Wide,
}

/// Clasifica el avión por tamaño y asientos típicos (config. 2 clases).
/// Usado para el reparto de clases, capacidad estimada y costes por porte.
/// (v8.3) También lo usan los traslados (`ferry`).
pub(crate) fn classify(model: Option<&str>) -> (Size, u32) {
    let m = model.unwrap_or("").to_ascii_uppercase();
    let has = |t: &str| m.contains(t);
    // ── Widebody ── (fix) reconocer TAMBIÉN los designadores ICAO de tipo
    // (B789, B77W, A359, A339…), que NO contienen "787"/"777"/"A350"/"A330":
    // antes caían al default Narrow/150 y arruinaban el P&L (combustible,
    // tasas, asientos, valor de flota).
    if has("747") || has("B74") {
        return (Size::Wide, 410);
    }
    if has("777") || has("B77") {
        // 777-300ER (B77W) ~365; 777-200/LR (B772/B77L) ~310.
        return (Size::Wide, if has("300") || has("77W") { 365 } else { 310 });
    }
    if has("787") || has("B78") {
        return (
            Size::Wide,
            if has("10") || has("78X") {
                330
            } else if has("9") || has("789") {
                290
            } else {
                242
            },
        );
    }
    if has("767") || has("B76") {
        return (Size::Wide, 245);
    }
    if has("A380") || has("380") || has("A388") {
        return (Size::Wide, 525);
    }
    if has("A350") || has("A35") {
        return (Size::Wide, if has("1000") || has("35K") { 370 } else { 315 });
    }
    if has("A340") || has("A34") {
        return (Size::Wide, 300);
    }
    if has("A330") || has("A33") {
        return (Size::Wide, if has("200") || has("332") { 247 } else { 287 });
    }
    if has("A300") || has("A310") || has("MD-11") || has("MD11") || has("DC-10") || has("DC10") {
        return (Size::Wide, 250);
    }
    // ── Regional ──
    if has("CRJ") {
        return (Size::Regional, 80);
    }
    if has("ATR") {
        return (Size::Regional, 70);
    }
    if has("DASH") || has("Q400") || has("DHC") || has("DH8") {
        return (Size::Regional, 78);
    }
    if has("E190") || has("E195") || has("EMB-190") || has("EMB-195") || has("E2") {
        return (Size::Regional, 110);
    }
    if has("E170") || has("E175") || has("EMB-170") || has("EMB-175")
        || has("ERJ-170") || has("ERJ-175") || has("E-JET")
    {
        return (Size::Regional, 76);
    }
    if has("SAAB") || has("EMB-120") || has("BEECH") {
        return (Size::Regional, 50);
    }
    // ── Narrowbody / por defecto ──
    if has("A318") {
        return (Size::Narrow, 107);
    }
    if has("A319") {
        return (Size::Narrow, 124);
    }
    if has("A321") {
        return (Size::Narrow, 200);
    }
    if has("A320") || has("A20N") || has("A32") {
        return (Size::Narrow, 165);
    }
    if has("A220") || has("CS100") || has("CS300") || has("BCS") {
        return (Size::Narrow, 130);
    }
    if has("757") {
        return (Size::Narrow, 200);
    }
    if has("737") || has("B73") || has("MAX") {
        if has("700") || has("MAX 7") || has("MAX7") || has("73G") {
            return (Size::Narrow, 140);
        }
        if has("900") || has("MAX 9") || has("MAX9") {
            return (Size::Narrow, 178);
        }
        return (Size::Narrow, 165);
    }
    if has("717") || has("MD-8") || has("MD8") || has("MD-9") || has("MD9") || has("DC-9") {
        return (Size::Narrow, 130);
    }
    (Size::Narrow, 150)
}

/// Precio de adquisición de UN avión por tamaño (USD, aprox. precio real de
/// mercado). Cada matrícula/livery distinta que vuela la aerolínea cuenta como
/// la compra de ese avión y se descuenta de su saldo.
fn purchase_price(size: Size) -> f64 {
    match size {
        Size::Wide => 200_000_000.0,
        Size::Narrow => 55_000_000.0,
        Size::Regional => 30_000_000.0,
    }
}

/// Quema de combustible típica en crucero (kg por NM) por tamaño — sólo se usa
/// cuando el vuelo no capturó `fuel_used_kg` real.
pub(crate) fn burn_per_nm(size: Size) -> f64 {
    match size {
        Size::Wide => 16.0,
        Size::Narrow => 7.5,
        Size::Regional => 4.0,
    }
}

// ──────────────────────────────── Recibos GSX ────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum GsxKind {
    Fuel,
    Catering,
    Handling,
}

struct GsxReceipt {
    kind: GsxKind,
    ts: DateTime<Utc>,
    airport: String, // ICAO en mayúsculas
    reg: String,     // matrícula normalizada (alfanumérica, mayúsculas)
    total: f64,
}

/// Costes GSX casados para un vuelo (None = no hubo recibo de ese tipo).
#[derive(Default)]
struct GsxCosts {
    fuel: Option<f64>,
    catering: Option<f64>,
    handling: Option<f64>,
}

impl GsxCosts {
    fn matched(&self) -> bool {
        self.fuel.is_some() || self.catering.is_some() || self.handling.is_some()
    }
}

/// Carpeta de recibos de GSX, resuelta vía `%APPDATA%` del usuario actual
/// (NO hardcodeada — funciona igual para otra cuenta/equipo). Sólo Windows.
fn receipts_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let path = PathBuf::from(appdata)
        .join("Virtuali")
        .join("GSX")
        .join("Receipts");
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

static FILE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{8}T\d{6}Z)_([A-Za-z0-9]+)_(.+)$").unwrap());
/// (v8.3) Equivalencia en USD que GSX añade al total en moneda local
/// ("C$22.05 ~$ 15.54").
static USD_APPROX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"~\s*\$\s*([0-9][0-9,]*(?:\.[0-9]+)?)").unwrap());
/// Total ya en USD ("$1,234.56" / "US$ 1,234.56"), sólo al principio: "C$22"
/// es de otra moneda.
static USD_PLAIN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(?:US)?\$\s*([0-9][0-9,]*(?:\.[0-9]+)?)").unwrap());
/// Línea "Total …" del recibo HTML (no "Subtotal": `\b`).
static HTML_TOTAL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\btotal\b\s*([^\n<]{0,80})").unwrap());
static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<style.*?</style>|<[^>]+>").unwrap());

/// Aeropuerto que GSX pone cuando no lo conoce: el recibo se casa por
/// matrícula + hora.
const UNKNOWN_AIRPORT: &str = "UNKWN";

/// Normaliza una matrícula para comparar ("B-222V" / "b222v" → "B222V").
pub fn normalize_reg(reg: &str) -> String {
    reg.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// (v8.3) Importe en USD del total de un recibo de GSX. GSX factura en la
/// moneda del aeropuerto y añade la equivalencia en dólares ("C$22.05 ~$
/// 15.54", "CN¥1,234 ~$ 170.10"); en EE. UU. el total ya es "$1,234.56".
fn usd_from_total(total: &str) -> Option<f64> {
    let caps = USD_APPROX_RE
        .captures(total)
        .or_else(|| USD_PLAIN_RE.captures(total))?;
    caps.get(1)?.as_str().replace(',', "").parse::<f64>().ok()
}

/// Total del recibo `.json` (formato actual de GSX, campo `total`).
fn parse_total_json(json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    usd_from_total(v.get("total")?.as_str()?)
}

/// Total del recibo `.html` (recibos viejos sin `.json`). Antes se buscaba
/// "total $" sin más: tomaba el SUBTOTAL (sin tasas) y no entendía los
/// totales en moneda local.
fn parse_total_html(html: &str) -> Option<f64> {
    let text = TAG_RE.replace_all(html, "\n").replace("&nbsp;", " ");
    HTML_TOTAL_RE
        .captures_iter(&text)
        .filter_map(|c| usd_from_total(c.get(1)?.as_str()))
        .last()
}

/// Lee y parsea todos los recibos de GSX (Fuel/Catering/Handling y, v8.3,
/// PassengerBus → handling). Cada recibo viene como `.json` + `.html` con el
/// mismo nombre: manda el `.json`; el `.html` sólo si no hay `.json`.
/// Tolerante: si la carpeta no existe o un archivo no parsea, se omite.
fn scan_receipts() -> Vec<GsxReceipt> {
    let Some(base) = receipts_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (sub, kind) in [
        ("Fuel", GsxKind::Fuel),
        ("Catering", GsxKind::Catering),
        ("Handling", GsxKind::Handling),
        // El bus de pasajeros es un servicio de tierra: va con el handling.
        ("PassengerBus", GsxKind::Handling),
    ] {
        let dir = base.join(sub);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut stems: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let ext = p.extension()?.to_str()?.to_ascii_lowercase();
                if ext != "json" && ext != "html" {
                    return None;
                }
                p.file_stem()?.to_str().map(String::from)
            })
            .collect();
        stems.sort();
        stems.dedup();
        for stem in stems {
            let Some(caps) = FILE_RE.captures(&stem) else {
                continue;
            };
            let ts_raw = caps.get(1).unwrap().as_str();
            let airport = caps.get(2).unwrap().as_str().to_ascii_uppercase();
            let reg = normalize_reg(caps.get(3).unwrap().as_str());
            let Ok(naive) = NaiveDateTime::parse_from_str(ts_raw, "%Y%m%dT%H%M%SZ") else {
                continue;
            };
            let ts = naive.and_utc();
            let total = std::fs::read_to_string(dir.join(format!("{stem}.json")))
                .ok()
                .and_then(|j| parse_total_json(&j))
                .or_else(|| {
                    std::fs::read_to_string(dir.join(format!("{stem}.html")))
                        .ok()
                        .and_then(|h| parse_total_html(&h))
                });
            let Some(total) = total else {
                tracing::debug!(target: "economy", "recibo GSX sin total legible: {sub}/{stem}");
                continue;
            };
            out.push(GsxReceipt { kind, ts, airport, reg, total });
        }
    }
    out
}

fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Casa los recibos GSX con un vuelo por matrícula + aeropuerto (origen o
/// destino) + ventana de ±6 h alrededor del bloque. Suma por tipo de servicio.
fn gsx_for_flight(receipts: &[GsxReceipt], e: &FlightLogEntry) -> GsxCosts {
    let mut costs = GsxCosts::default();
    if receipts.is_empty() {
        return costs;
    }
    let reg = match e.aircraft_registration.as_deref() {
        Some(r) => normalize_reg(r),
        None => return costs,
    };
    if reg.is_empty() {
        return costs;
    }
    let airports: Vec<String> = [e.origin_icao.as_deref(), e.destination_icao.as_deref()]
        .into_iter()
        .flatten()
        .map(|s| s.to_ascii_uppercase())
        .collect();
    let origin_up = e.origin_icao.as_deref().map(|s| s.to_ascii_uppercase());

    let start = parse_dt(&e.started_at);
    let end = e
        .ended_at
        .as_deref()
        .and_then(parse_dt)
        .or(start);
    // (fix) Ventana más estrecha (±4h vs ±6h) para reducir que un mismo recibo
    // caiga en dos vuelos de la misma aeronave en el día (turnaround).
    let window = match (start, end) {
        (Some(s), Some(en)) => Some((s - Duration::hours(4), en + Duration::hours(4))),
        _ => None, // sin tiempos fiables: casa sólo por matrícula + aeropuerto
    };

    for r in receipts {
        if r.reg != reg {
            continue;
        }
        // (fix) El COMBUSTIBLE es un servicio de SALIDA → sólo cuenta en el
        // aeropuerto de ORIGEN del tramo. Antes, un recibo de fuel en el DESTINO
        // (que es la salida del tramo SIGUIENTE de la misma aeronave) se cargaba
        // por duplicado a este vuelo y al siguiente. Catering/handling siguen
        // casando por origen o destino.
        // (v8.3) Aeropuerto desconocido para GSX ("UNKWN"): se casa por
        // matrícula + ventana de tiempo, que entonces es obligatoria.
        let airport_ok = if r.airport == UNKNOWN_AIRPORT {
            window.is_some()
        } else {
            match r.kind {
                GsxKind::Fuel => origin_up.as_deref() == Some(r.airport.as_str()),
                _ => airports.is_empty() || airports.contains(&r.airport),
            }
        };
        if !airport_ok {
            continue;
        }
        if let Some((lo, hi)) = window {
            if r.ts < lo || r.ts > hi {
                continue;
            }
        }
        let slot = match r.kind {
            GsxKind::Fuel => &mut costs.fuel,
            GsxKind::Catering => &mut costs.catering,
            GsxKind::Handling => &mut costs.handling,
        };
        *slot = Some(slot.unwrap_or(0.0) + r.total);
    }
    costs
}

// ─────────────────────── Política de gestión (ecosistema) ────────────────────

/// Decisiones del jugador para gestionar la aerolínea. Alimentan el P&L:
/// el nivel de mantenimiento mueve el coste y la satisfacción; cada servicio
/// a bordo añade ingreso ancillary (y algo de satisfacción).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlinePolicy {
    /// 0 = básico, 1 = estándar, 2 = premium.
    pub maintenance_level: i64,
    pub snacks: bool,
    pub meals: bool,
    pub wifi: bool,
    pub seat_upgrades: bool,
    pub priority_boarding: bool,
    pub extra_baggage: bool,
}

impl Default for AirlinePolicy {
    fn default() -> Self {
        Self {
            maintenance_level: 1,
            snacks: false,
            meals: false,
            wifi: false,
            seat_upgrades: false,
            priority_boarding: false,
            extra_baggage: false,
        }
    }
}

impl AirlinePolicy {
    /// Multiplicador del coste de mantenimiento según el nivel elegido.
    fn maintenance_mult(&self) -> f64 {
        match self.maintenance_level {
            0 => 0.75,
            2 => 1.45,
            _ => 1.0,
        }
    }
    /// Ajuste de satisfacción (puntos) por las decisiones de gestión.
    fn satisfaction_delta(&self) -> f64 {
        let mut d = match self.maintenance_level {
            0 => -6.0,
            2 => 6.0,
            _ => 0.0,
        };
        if self.wifi {
            d += 3.0;
        }
        if self.meals {
            d += 3.0;
        }
        if self.seat_upgrades {
            d += 2.0;
        }
        if self.snacks {
            d += 1.0;
        }
        if self.priority_boarding {
            d += 1.0;
        }
        d
    }
}

/// (v6.1) Servicios a bordo AUTOMÁTICOS por tipo de aerolínea. Ya no se gestionan
/// a mano: cada aerolínea ofrece lo que tiene sentido para su modelo de negocio,
/// y la economía (ingresos ancillary, satisfacción, nickel-and-diming) lo refleja.
/// Las 3 categorías difieren de verdad:
///   · Carga      → sin servicios de pasaje (vuela flete).
///   · Low-cost   → TODO à la carte de pago: snacks, asiento, prioridad, equipaje,
///     wifi. Ancillary fuerte pero penaliza la percepción ("nickel-and-diming").
///   · Legacy     → confort incluido: snacks, comida (largo radio), wifi, selección
///     de asiento. Sin cobrar prioridad/equipaje (van en la tarifa).
/// El `maintenance_level` NO se toca aquí — ese lo sigue eligiendo el usuario en
/// el panel de mantenimiento (premium/estándar/básico).
fn auto_services(lowcost: bool, cargo: bool) -> AirlinePolicy {
    if cargo {
        return AirlinePolicy {
            maintenance_level: 1,
            snacks: false,
            meals: false,
            wifi: false,
            seat_upgrades: false,
            priority_boarding: false,
            extra_baggage: false,
        };
    }
    if lowcost {
        return AirlinePolicy {
            maintenance_level: 1,
            snacks: true,
            meals: false,
            wifi: true,
            seat_upgrades: true,
            priority_boarding: true,
            extra_baggage: true,
        };
    }
    // Legacy / tradicional.
    AirlinePolicy {
        maintenance_level: 1,
        snacks: true,
        meals: true,
        wifi: true,
        seat_upgrades: true,
        priority_boarding: false,
        extra_baggage: false,
    }
}

// ─────────────────────────────── P&L por vuelo ───────────────────────────────

struct Pnl {
    pax: i64,
    seats: i64,
    revenue_tickets: f64,
    revenue_ancillary: f64,
    cost_fuel: f64,
    cost_catering: f64,
    cost_handling: f64,
    cost_maintenance: f64,
    cost_fees: f64,
    net: f64,
    gsx_matched: bool,
}

/// Carga útil típica de un carguero por tamaño (kg) — cuando el vuelo no
/// registró `cargo_kg`.
fn freighter_payload(size: Size) -> f64 {
    match size {
        Size::Wide => 100_000.0,
        Size::Narrow => 22_000.0,
        Size::Regional => 9_000.0,
    }
}

/// Economía de los servicios a bordo para UN vuelo. Devuelve (ingreso, coste).
/// Modelo REAL: cada servicio tiene una tasa de ADOPCIÓN (depende del público
/// LCC/legacy y de la distancia), un precio, un coste unitario y a veces un
/// coste FIJO por vuelo. Por eso activar un servicio NO siempre da beneficio:
/// el wifi o el menú de pago pierden dinero en vuelos cortos o con pocos pax.
fn service_economics(p: &AirlinePolicy, lowcost: bool, d: f64, pax: f64) -> (f64, f64) {
    let long = d > 1500.0;
    let mut rev = 0.0;
    let mut cost = 0.0;
    // (activo, adopción, precio, coste_unitario, coste_fijo_por_vuelo)
    let items: [(bool, f64, f64, f64, f64); 6] = [
        (p.snacks, if lowcost { 0.40 } else { 0.25 }, if lowcost { 4.0 } else { 5.0 }, 1.5, 0.0),
        (p.meals, if long { if lowcost { 0.15 } else { 0.30 } } else { 0.10 }, 12.0, 7.0, 200.0),
        (p.wifi, if long { 0.30 } else { 0.12 }, 8.0, 0.5, 400.0),
        (p.seat_upgrades, if lowcost { 0.25 } else { 0.12 }, 18.0, 0.0, 0.0),
        (p.priority_boarding, if lowcost { 0.20 } else { 0.08 }, 9.0, 0.0, 0.0),
        (p.extra_baggage, if lowcost { 0.35 } else { 0.10 }, if lowcost { 40.0 } else { 60.0 }, 5.0, 0.0),
    ];
    for (on, adoption, price, unit_cost, fixed) in items {
        if !on {
            continue;
        }
        let sold = pax * adoption;
        rev += sold * price;
        cost += sold * unit_cost + fixed;
    }
    (rev, cost)
}

/// Distancia del vuelo (NM): la real si está, o estimada por tiempo × crucero.
fn distance_nm(e: &FlightLogEntry, size: Size) -> f64 {
    if let Some(d) = e.distance_nm {
        if d > 1.0 {
            return d;
        }
    }
    if let Some(s) = e.flight_time_s {
        if s > 0 {
            let hours = s as f64 / 3600.0;
            let cruise = match size {
                Size::Wide => 470.0,
                Size::Narrow => 440.0,
                Size::Regional => 300.0,
            };
            return hours * cruise * 0.9; // descuenta ascenso/descenso
        }
    }
    300.0
}

/// Pasajeros: el real si se registró, o estimado por capacidad × load factor.
/// El load factor se siembra con el `id` del vuelo para variar de forma
/// determinista (mismo vuelo → mismo estimado) sin ser idéntico en todos.
fn estimate_pax(e: &FlightLogEntry, seats: u32, lowcost: bool) -> i64 {
    if let Some(p) = e.passengers {
        if p > 0 {
            return p;
        }
    }
    let base = if lowcost { 0.90 } else { 0.82 };
    let jitter = ((e.id.rem_euclid(13)) as f64 - 6.0) / 100.0; // ±0.06
    let lf = (base + jitter).clamp(0.60, 0.98);
    ((seats as f64) * lf).round() as i64
}

/// Combustible del vuelo (USD): GSX real si casó, o estimado por kg quemados.
fn fuel_cost(e: &FlightLogEntry, gsx: &GsxCosts, size: Size, d: f64) -> f64 {
    let fuel_kg = e
        .fuel_used_kg
        .map(|k| k as f64)
        .filter(|k| *k > 0.0)
        .unwrap_or_else(|| d * burn_per_nm(size));
    gsx.fuel.unwrap_or(fuel_kg * FUEL_USD_PER_KG)
}

/// Precio del Jet A-1 cuando no hay recibo GSX (~0.80 USD/kg).
pub(crate) const FUEL_USD_PER_KG: f64 = 0.80;

/// Horas de bloque (reales o estimadas por distancia).
fn block_hours(e: &FlightLogEntry, d: f64) -> f64 {
    e.flight_time_s
        .map(|s| s as f64 / 3600.0)
        .filter(|h| *h > 0.0)
        .unwrap_or(d / 440.0)
}

pub(crate) fn maint_rate(size: Size) -> f64 {
    match size {
        Size::Wide => 1500.0,
        Size::Narrow => 800.0,
        Size::Regional => 450.0,
    }
}

pub(crate) fn landing_fee(size: Size) -> f64 {
    match size {
        Size::Wide => 1800.0,
        Size::Narrow => 650.0,
        Size::Regional => 350.0,
    }
}

/// P&L de un vuelo de CARGA: ingreso por flete (USD/kg × distancia), sin
/// pasajeros ni servicios a bordo. Handling más caro (carga/descarga).
fn compute_cargo_pnl(
    e: &FlightLogEntry,
    gsx: &GsxCosts,
    policy: &AirlinePolicy,
    size: Size,
    d: f64,
) -> Pnl {
    let payload = e
        .cargo_kg
        .map(|k| k as f64)
        .filter(|k| *k > 0.0)
        .unwrap_or_else(|| {
            let lf = (0.78 + ((e.id.rem_euclid(11)) as f64 - 5.0) / 100.0).clamp(0.55, 0.95);
            freighter_payload(size) * lf
        });
    // Tarifa de flete por kg para todo el trayecto (sube con la distancia).
    let revenue_tickets = payload * (0.80 + 0.0006 * d);
    let revenue_ancillary = payload * 0.05; // express/prioridad/seguro

    let cost_fuel = fuel_cost(e, gsx, size, d);
    let cost_catering = 0.0; // no hay catering en carga
    let handling_est = match size {
        Size::Wide => 2500.0,
        Size::Narrow => 900.0,
        Size::Regional => 500.0,
    };
    let cost_handling = gsx.handling.unwrap_or(handling_est);
    let cost_maintenance = block_hours(e, d) * maint_rate(size) * policy.maintenance_mult();
    let cost_fees = landing_fee(size) + d * 1.2 + payload * 0.012; // landing+nav+aduana

    let revenue = revenue_tickets + revenue_ancillary;
    let costs = cost_fuel + cost_catering + cost_handling + cost_maintenance + cost_fees;
    Pnl {
        pax: 0,
        seats: 0,
        revenue_tickets,
        revenue_ancillary,
        cost_fuel,
        cost_catering,
        cost_handling,
        cost_maintenance,
        cost_fees,
        net: revenue - costs,
        gsx_matched: gsx.matched(),
    }
}

/// Calcula el P&L de un vuelo. Ruta de carga o de pasajeros según la aerolínea.
fn compute_pnl(e: &FlightLogEntry, res: &Resolved, gsx: &GsxCosts, policy: &AirlinePolicy) -> Pnl {
    let (size, seats) = classify(e.aircraft_model.as_deref().or(e.aircraft_atc_type.as_deref()));
    let d = distance_nm(e, size);

    if res.cargo {
        return compute_cargo_pnl(e, gsx, policy, size, d);
    }

    let pax = estimate_pax(e, seats, res.lowcost);
    let paxf = pax as f64;

    // ── Ingresos de billetes ──
    let (revenue_tickets, base_ancillary) = if res.lowcost {
        let fare = 22.0 + 0.095 * d;
        let tickets = paxf * fare;
        let ancillary = paxf * (10.0 + 0.008 * d); // base ancillary LCC (sin servicios add-on)
        (tickets, ancillary)
    } else {
        let econ = 38.0 + 0.135 * d;
        let (se, sb, sf) = match size {
            Size::Wide => (0.80, 0.15, 0.05),
            Size::Narrow => (0.90, 0.10, 0.00),
            Size::Regional => (0.96, 0.04, 0.00),
        };
        let tickets = paxf * se * econ + paxf * sb * (econ * 3.0) + paxf * sf * (econ * 5.5);
        let ancillary = paxf * (5.0 + 0.003 * d);
        (tickets, ancillary)
    };

    // (v6.1) Servicios a bordo: ingreso por adopción − coste (puede dar pérdida).
    let (svc_rev, svc_cost) = service_economics(policy, res.lowcost, d, paxf);
    let revenue_ancillary = base_ancillary + svc_rev;

    // ── Costes ──
    let cost_fuel = fuel_cost(e, gsx, size, d);
    let cat_est = if res.lowcost { paxf * 2.0 } else { paxf * (4.0 + 0.0015 * d) };
    let cost_catering = gsx.catering.unwrap_or(cat_est) + svc_cost;
    let handling_est = match size {
        Size::Wide => 1100.0,
        Size::Narrow => 480.0,
        Size::Regional => 280.0,
    };
    let cost_handling = gsx.handling.unwrap_or(handling_est);
    let cost_maintenance = block_hours(e, d) * maint_rate(size) * policy.maintenance_mult();
    let cost_fees = landing_fee(size) + d * 1.2 + paxf * 22.0; // landing + nav + tasas pax

    let revenue = revenue_tickets + revenue_ancillary;
    let costs = cost_fuel + cost_catering + cost_handling + cost_maintenance + cost_fees;

    Pnl {
        pax,
        seats: seats as i64,
        revenue_tickets,
        revenue_ancillary,
        cost_fuel,
        cost_catering,
        cost_handling,
        cost_maintenance,
        cost_fees,
        net: revenue - costs,
        gsx_matched: gsx.matched(),
    }
}

// ───────────────────────────────── Persistencia ──────────────────────────────

struct LedgerAcc {
    icao: Option<String>,
    name: String,
    lowcost: bool,
    cargo: bool,
    valuation: f64,
    revenue: f64,
    costs: f64,
    flights: i64,
    passengers: i64,
    revenue_tickets: f64,
    revenue_ancillary: f64,
    cost_fuel: f64,
    cost_catering: f64,
    cost_handling: f64,
    cost_maintenance: f64,
    cost_fees: f64,
    gsx_flights: i64,
    /// Acumuladores para la media de FPM al aterrizar (sólo vuelos con dato).
    fpm_sum: f64,
    fpm_count: i64,
    /// Asientos ofertados acumulados (para el factor de ocupación).
    seat_sum: i64,
    /// Aterrizajes duros (FPM < -600) y rebotes — penalizan la satisfacción.
    hard: i64,
    bounced: i64,
    /// Aviones distintos (matrículas/liveries) → flota comprada.
    fleet_regs: HashSet<String>,
    /// Suma del precio de compra de cada avión distinto (inversión en flota).
    fleet_value: f64,
    /// (v8.3) Traslados (vuelos de posicionamiento) y su costo.
    cost_ferry: f64,
    ferries: i64,
    /// (v8.3 · 6.2c) Operación autónoma: días, vuelos reales, ingresos y
    /// costos (incluidos ya en los totales), costos fijos y flota.
    auto_days: i64,
    auto_flights: i64,
    auto_revenue: f64,
    auto_costs: f64,
    cost_fixed: f64,
    auto_fleet: i64,
}

/// Satisfacción del pasajero (0-100) derivada de la media de FPM al tocar
/// pista. Aterrizaje suave (≈ -150 FPM) → ~100; cuanto más duro (más negativo),
/// más baja. Es el factor PRINCIPAL pero no el único (ver `holistic_satisfaction`).
fn satisfaction_from_fpm(avg_fpm: f64) -> f64 {
    let harsh = (avg_fpm.abs() - 150.0).max(0.0);
    (100.0 - harsh * 0.09).clamp(25.0, 100.0)
}

/// Satisfacción HOLÍSTICA (0-100): no es solo el aterrizaje. Combina la
/// suavidad media de aterrizaje (factor principal), la fiabilidad (aterrizajes
/// duros/rebotes), la gestión (mantenimiento + servicios), la ocupación
/// (aviones reventados de gente incomodan) y el "nickel-and-diming" (demasiados
/// servicios de pago en una LCC molestan). Para carga, mide fiabilidad operativa.
fn holistic_satisfaction(acc: &LedgerAcc, policy: &AirlinePolicy) -> f64 {
    let base = if acc.fpm_count > 0 {
        satisfaction_from_fpm(acc.fpm_sum / acc.fpm_count as f64)
    } else {
        72.0 // neutro si aún no hay aterrizajes medidos
    };
    if acc.cargo {
        // Carga: solo cuenta la fiabilidad (mantenimiento + aterrizajes suaves).
        let maint = match policy.maintenance_level {
            0 => -6.0,
            2 => 6.0,
            _ => 0.0,
        };
        let hard_ratio = if acc.flights > 0 { acc.hard as f64 / acc.flights as f64 } else { 0.0 };
        return (base + maint - hard_ratio * 18.0).clamp(15.0, 100.0);
    }
    let mut s = base + policy.satisfaction_delta();
    // Fiabilidad: aterrizajes duros y rebotes molestan a los pasajeros.
    if acc.flights > 0 {
        let hard_ratio = acc.hard as f64 / acc.flights as f64;
        let bounce_ratio = acc.bounced as f64 / acc.flights as f64;
        s -= hard_ratio * 20.0 + bounce_ratio * 15.0;
    }
    // Ocupación: aviones siempre reventados (>92%) restan algo de confort.
    if acc.seat_sum > 0 {
        let lf = acc.passengers as f64 / acc.seat_sum as f64;
        if lf > 0.92 {
            s -= 3.0;
        }
    }
    // "Nickel-and-diming": muchas tarifas de pago en una LCC fastidian.
    if acc.lowcost {
        let paid = [policy.snacks, policy.meals, policy.wifi, policy.seat_upgrades,
                    policy.priority_boarding, policy.extra_baggage]
            .iter().filter(|x| **x).count();
        if paid >= 4 {
            s -= 3.0;
        }
    }
    s.clamp(15.0, 100.0)
}

// ─────────────────── Operación autónoma (v8.3 · Etapa 6.2c) ───────────────────

/// Costo fijo diario por avión de la flota (tripulaciones, seguro,
/// estacionamiento y estructura). Decisión del usuario: nivel "realista".
fn fixed_daily(size: Size) -> f64 {
    match size {
        Size::Wide => 22_000.0,
        Size::Narrow => 8_000.0,
        Size::Regional => 4_000.0,
    }
}

/// Totales de la operación autónoma de una aerolínea (todos sus días).
#[derive(Debug, Default, Clone)]
struct AutoTotals {
    days: i64,
    flights: i64,
    revenue_tickets: f64,
    revenue_ancillary: f64,
    cost_fuel: f64,
    cost_catering: f64,
    cost_handling: f64,
    cost_maintenance: f64,
    cost_fees: f64,
    cost_fixed: f64,
    /// Flota del día más reciente.
    last_fleet: i64,
}

/// Semilla estable de un vuelo real (para el factor de ocupación).
fn real_flight_seed(icao24: &str, first_seen: i64) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in icao24.bytes().chain(first_seen.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    (h >> 1) as i64
}

/// Tipo más común (designador) entre los aviones de una aerolínea.
fn most_common_type<'a>(types: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for t in types {
        *count.entry(t).or_default() += 1;
    }
    count.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(t, _)| t.to_string())
}

/// Recalcula los días UTC completos (12 ventanas) que aún tienen vuelos
/// crudos: P&L de cada vuelo real con el mismo modelo que los del usuario
/// y costos fijos de la flota (vistos volando en 30 días + los del usuario).
async fn refresh_auto_days(
    pool: &SqlitePool,
    flights: &[FlightLogEntry],
    learned: &HashMap<String, String>,
    policies: &HashMap<String, AirlinePolicy>,
) -> Result<usize> {
    let days = crate::opensky::complete_days(pool).await?;
    if days.is_empty() {
        return Ok(0);
    }
    // Perfil económico por ICAO y aviones del usuario por aerolínea.
    let mut profiles: HashMap<String, Resolved> = HashMap::new();
    let mut user_regs: HashMap<String, HashMap<String, Option<String>>> = HashMap::new();
    for f in flights {
        let Some(res) = resolve_airline(f, learned) else { continue };
        if let Some(reg) = f.aircraft_registration.as_deref().map(normalize_reg).filter(|r| !r.is_empty()) {
            user_regs
                .entry(res.key.clone())
                .or_default()
                .entry(reg)
                .or_insert_with(|| f.aircraft_model.clone().or_else(|| f.aircraft_atc_type.clone()));
        }
        if let Some(icao) = res.icao.clone() {
            profiles.entry(icao).or_insert(res);
        }
    }
    if profiles.is_empty() {
        return Ok(0);
    }
    let coords: HashMap<String, (f64, f64)> = sqlx::query("SELECT icao, latitude, longitude FROM airports")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.get::<String, _>("icao"), (r.get("latitude"), r.get("longitude"))))
        .collect();
    let now = Utc::now().to_rfc3339();
    let mut rows_out: Vec<(String, String, AutoTotals, i64)> = Vec::new();
    for day_start in &days {
        let day_end = day_start + 86_400;
        let day = DateTime::from_timestamp(*day_start, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
        // Flota de cada aerolínea: aviones vistos en los 30 días hasta este día.
        let mut fleet: HashMap<String, HashMap<String, (Option<String>, Option<String>)>> = HashMap::new();
        for r in sqlx::query(
            "SELECT DISTINCT rf.airline_icao AS al, rf.icao24 AS hex, a.typecode AS tc, a.registration AS reg
               FROM real_flight rf LEFT JOIN aircraft_db a ON a.icao24 = rf.icao24
              WHERE rf.first_seen >= ? AND rf.first_seen < ?",
        )
        .bind(day_end - 30 * 86_400)
        .bind(day_end)
        .fetch_all(pool)
        .await?
        {
            fleet
                .entry(r.get("al"))
                .or_default()
                .insert(r.get("hex"), (r.get("tc"), r.get("reg")));
        }
        let common: HashMap<&String, Option<String>> = fleet
            .iter()
            .map(|(al, ac)| (al, most_common_type(ac.values().filter_map(|(tc, _)| tc.as_deref()))))
            .collect();
        // Vuelos reales del día.
        let mut acc: HashMap<String, AutoTotals> = HashMap::new();
        for r in sqlx::query(
            "SELECT rf.icao24 AS hex, rf.first_seen AS fs, rf.last_seen AS ls, rf.airline_icao AS al,
                    rf.dep_icao AS dep, rf.arr_icao AS arr, rf.callsign AS cs,
                    a.typecode AS tc, a.registration AS reg
               FROM real_flight rf LEFT JOIN aircraft_db a ON a.icao24 = rf.icao24
              WHERE rf.window_begin >= ? AND rf.window_begin < ?",
        )
        .bind(day_start)
        .bind(day_end)
        .fetch_all(pool)
        .await?
        {
            let al: String = r.get("al");
            let Some(res) = profiles.get(&al) else { continue };
            let hex: String = r.get("hex");
            let fs: i64 = r.get("fs");
            let ls: i64 = r.get("ls");
            let dep: Option<String> = r.get("dep");
            let arr: Option<String> = r.get("arr");
            let model = r
                .get::<Option<String>, _>("tc")
                .or_else(|| common.get(&al).cloned().flatten());
            let distance = match (dep.as_deref().and_then(|d| coords.get(d)), arr.as_deref().and_then(|a| coords.get(a))) {
                (Some(a), Some(b)) if dep != arr => Some(flight_log::haversine_nm(a.0, a.1, b.0, b.1)),
                _ => None,
            };
            let entry = FlightLogEntry {
                id: real_flight_seed(&hex, fs),
                started_at: DateTime::from_timestamp(fs, 0).map(|d| d.to_rfc3339()).unwrap_or_default(),
                ended_at: DateTime::from_timestamp(ls, 0).map(|d| d.to_rfc3339()),
                origin_icao: dep,
                destination_icao: arr,
                aircraft_model: model,
                aircraft_registration: r.get("reg"),
                airline_icao: Some(al.clone()),
                callsign: Some(r.get("cs")),
                distance_nm: distance,
                flight_time_s: Some((ls - fs).max(600)),
                source: "opensky".into(),
                ..Default::default()
            };
            let mut policy = auto_services(res.lowcost, res.cargo);
            policy.maintenance_level = policies.get(&res.key).map(|p| p.maintenance_level).unwrap_or(1);
            let pnl = compute_pnl(&entry, res, &GsxCosts::default(), &policy);
            let t = acc.entry(res.key.clone()).or_default();
            t.flights += 1;
            t.revenue_tickets += pnl.revenue_tickets;
            t.revenue_ancillary += pnl.revenue_ancillary;
            t.cost_fuel += pnl.cost_fuel;
            t.cost_catering += pnl.cost_catering;
            t.cost_handling += pnl.cost_handling;
            t.cost_maintenance += pnl.cost_maintenance;
            t.cost_fees += pnl.cost_fees;
        }
        // Costos fijos: flota vista + aviones del usuario que no se vieron.
        for (al, res) in &profiles {
            let seen = fleet.get(al);
            let common_size = common.get(al).cloned().flatten();
            let mut cost = 0.0;
            let mut n = 0i64;
            let mut seen_regs: HashSet<String> = HashSet::new();
            for (tc, reg) in seen.map(|m| m.values().collect::<Vec<_>>()).unwrap_or_default() {
                let model = tc.clone().or_else(|| common_size.clone());
                cost += fixed_daily(classify(model.as_deref()).0);
                n += 1;
                seen_regs.extend(reg.as_deref().map(normalize_reg));
            }
            for (reg, model) in user_regs.get(&res.key).into_iter().flatten() {
                if !seen_regs.contains(reg) {
                    cost += fixed_daily(classify(model.as_deref()).0);
                    n += 1;
                }
            }
            let t = acc.entry(res.key.clone()).or_default();
            if t.flights == 0 && seen.is_none() {
                // Sin vuelos reales vistos aún: no opera (ni costos fijos).
                continue;
            }
            t.cost_fixed = cost;
            rows_out.push((res.key.clone(), day.clone(), t.clone(), n));
        }
    }
    let mut tx = pool.begin().await?;
    for (key, day, t, fleet_n) in &rows_out {
        sqlx::query(
            "INSERT OR REPLACE INTO auto_daily
                (airline_key, day, flights, pax, revenue_tickets, revenue_ancillary, cost_fuel,
                 cost_catering, cost_handling, cost_maintenance, cost_fees, cost_fixed, fleet, computed_at)
             VALUES (?,?,?,0,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(key)
        .bind(day)
        .bind(t.flights)
        .bind(t.revenue_tickets)
        .bind(t.revenue_ancillary)
        .bind(t.cost_fuel)
        .bind(t.cost_catering)
        .bind(t.cost_handling)
        .bind(t.cost_maintenance)
        .bind(t.cost_fees)
        .bind(t.cost_fixed)
        .bind(fleet_n)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows_out.len())
}

/// Totales por aerolínea de todos los días de operación guardados.
async fn load_auto_totals(pool: &SqlitePool) -> Result<HashMap<String, AutoTotals>> {
    let rows = sqlx::query(
        "SELECT airline_key, COUNT(*) AS days, SUM(flights) AS flights,
                SUM(revenue_tickets) AS rt, SUM(revenue_ancillary) AS ra,
                SUM(cost_fuel) AS cf, SUM(cost_catering) AS cc, SUM(cost_handling) AS ch,
                SUM(cost_maintenance) AS cm, SUM(cost_fees) AS cfe, SUM(cost_fixed) AS cfx,
                (SELECT fleet FROM auto_daily d2 WHERE d2.airline_key = d.airline_key
                  ORDER BY day DESC LIMIT 1) AS last_fleet
           FROM auto_daily d GROUP BY airline_key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("airline_key"),
                AutoTotals {
                    days: r.get("days"),
                    flights: r.get("flights"),
                    revenue_tickets: r.get("rt"),
                    revenue_ancillary: r.get("ra"),
                    cost_fuel: r.get("cf"),
                    cost_catering: r.get("cc"),
                    cost_handling: r.get("ch"),
                    cost_maintenance: r.get("cm"),
                    cost_fees: r.get("cfe"),
                    cost_fixed: r.get("cfx"),
                    last_fleet: r.get::<Option<i64>, _>("last_fleet").unwrap_or(0),
                },
            )
        })
        .collect())
}

/// Recalcula TODO el ledger desde `flight_log` (idempotente). Borra y
/// reconstruye `flight_pnl` y `airline_economy`, de modo que cada llamada
/// refleja exactamente los vuelos actuales — "consume la data actual".
pub async fn recompute(pool: &SqlitePool) -> Result<()> {
    // (v6.2.19) Auto-mantenimiento ANTES de leer costes: los componentes al
    // 100% se sirven solos (como en la vida real) y su coste entra en esta
    // misma pasada. Hace la economía autosostenible sin intervención manual.
    if let Err(e) = crate::aircraft_maintenance::auto_service_worn(pool).await {
        tracing::warn!(target: "economy", "auto-mantenimiento falló: {e:#}");
    }
    // (v8.3) Traslados automáticos (vuelos que salieron de otro aeropuerto)
    // antes de leer los traslados que se cargan a cada aerolínea.
    if let Err(e) = crate::ferry::sync_auto_ferries(pool).await {
        tracing::warn!(target: "economy", "traslados automáticos fallaron: {e:#}");
    }
    let ferries = crate::ferry::active(pool).await.unwrap_or_default();
    let receipts = scan_receipts();
    let flights = flight_log::list_entries(pool).await?;
    let policies = load_policies(pool).await?;
    let learned = learn_airline_icaos(&flights);
    // Coste de servicios de mantenimiento por matrícula → se imputa a su
    // aerolínea (todo relacionado: lo que gastas en el panel de mantenimiento
    // sale en las cuentas de Finanzas).
    let maint_costs = crate::aircraft_maintenance::maintenance_cost_by_reg(pool)
        .await
        .unwrap_or_default();
    let mut reg_to_airline: HashMap<String, String> = HashMap::new();
    // (v8.3 · Etapa 6.2c) Operación autónoma (vuelos reales de OpenSky +
    // costos fijos), antes de abrir la transacción del ledger.
    if let Err(e) = refresh_auto_days(pool, &flights, &learned, &policies).await {
        tracing::warn!(target: "economy", "operación autónoma: {e:#}");
    }
    let auto = load_auto_totals(pool).await.unwrap_or_default();

    let mut ledger: HashMap<String, LedgerAcc> = HashMap::new();
    let now = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM flight_pnl").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM airline_economy")
        .execute(&mut *tx)
        .await?;

    for f in &flights {
        let Some(res) = resolve_airline(f, &learned) else {
            continue;
        };
        // (v6.1) Servicios AUTOMÁTICOS por tipo; solo el nivel de mantenimiento
        // se conserva de la política manual del usuario.
        let stored_level = policies
            .get(&res.key)
            .map(|p| p.maintenance_level)
            .unwrap_or(1);
        let mut policy = auto_services(res.lowcost, res.cargo);
        policy.maintenance_level = stored_level;
        if let Some(reg) = f.aircraft_registration.as_deref() {
            let reg = normalize_reg(reg);
            if !reg.is_empty() {
                reg_to_airline.entry(reg).or_insert_with(|| res.key.clone());
            }
        }
        let gsx = gsx_for_flight(&receipts, f);
        let pnl = compute_pnl(f, &res, &gsx, &policy);

        sqlx::query(
            r#"INSERT OR REPLACE INTO flight_pnl
                (flight_id, airline_key, pax, revenue_tickets, revenue_ancillary,
                 cost_fuel, cost_catering, cost_handling, cost_maintenance,
                 cost_fees, net, gsx_matched, created_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(f.id)
        .bind(&res.key)
        .bind(pnl.pax)
        .bind(pnl.revenue_tickets)
        .bind(pnl.revenue_ancillary)
        .bind(pnl.cost_fuel)
        .bind(pnl.cost_catering)
        .bind(pnl.cost_handling)
        .bind(pnl.cost_maintenance)
        .bind(pnl.cost_fees)
        .bind(pnl.net)
        .bind(pnl.gsx_matched as i64)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let acc = ledger.entry(res.key.clone()).or_insert_with(|| LedgerAcc {
            icao: res.icao.clone(),
            name: res.name.clone(),
            lowcost: res.lowcost,
            cargo: res.cargo,
            valuation: res.valuation,
            revenue: 0.0,
            costs: 0.0,
            flights: 0,
            passengers: 0,
            revenue_tickets: 0.0,
            revenue_ancillary: 0.0,
            cost_fuel: 0.0,
            cost_catering: 0.0,
            cost_handling: 0.0,
            cost_maintenance: 0.0,
            cost_fees: 0.0,
            gsx_flights: 0,
            fpm_sum: 0.0,
            fpm_count: 0,
            seat_sum: 0,
            hard: 0,
            bounced: 0,
            fleet_regs: HashSet::new(),
            fleet_value: 0.0,
            cost_ferry: 0.0,
            ferries: 0,
            auto_days: 0,
            auto_flights: 0,
            auto_revenue: 0.0,
            auto_costs: 0.0,
            cost_fixed: 0.0,
            auto_fleet: 0,
        });
        // Cada avión distinto (matrícula) = compra de ese avión → inversión.
        if let Some(reg) = f.aircraft_registration.as_deref() {
            let reg = normalize_reg(reg);
            if !reg.is_empty() && acc.fleet_regs.insert(reg) {
                let (size, _) =
                    classify(f.aircraft_model.as_deref().or(f.aircraft_atc_type.as_deref()));
                acc.fleet_value += purchase_price(size);
            }
        }
        acc.revenue += pnl.revenue_tickets + pnl.revenue_ancillary;
        acc.costs += pnl.cost_fuel
            + pnl.cost_catering
            + pnl.cost_handling
            + pnl.cost_maintenance
            + pnl.cost_fees;
        acc.flights += 1;
        acc.passengers += pnl.pax;
        acc.seat_sum += pnl.seats;
        acc.revenue_tickets += pnl.revenue_tickets;
        acc.revenue_ancillary += pnl.revenue_ancillary;
        acc.cost_fuel += pnl.cost_fuel;
        acc.cost_catering += pnl.cost_catering;
        acc.cost_handling += pnl.cost_handling;
        acc.cost_maintenance += pnl.cost_maintenance;
        acc.cost_fees += pnl.cost_fees;
        if pnl.gsx_matched {
            acc.gsx_flights += 1;
        }
        if f.bounced {
            acc.bounced += 1;
        }
        // Satisfacción: sólo cuentan los vuelos que registraron FPM al tocar.
        if let Some(fpm) = f.landing_fpm {
            acc.fpm_sum += fpm as f64;
            acc.fpm_count += 1;
            if fpm < -600 {
                acc.hard += 1;
            }
        }
    }

    // Imputa el gasto de mantenimiento (servicios hechos en el panel) a la
    // aerolínea dueña de cada matrícula.
    for (reg, cost) in &maint_costs {
        if let Some(key) = reg_to_airline.get(reg) {
            if let Some(acc) = ledger.get_mut(key) {
                acc.cost_maintenance += *cost;
                acc.costs += *cost;
            }
        }
    }

    // (v8.3) Traslados: a la aerolínea del vuelo más reciente de la matrícula.
    // Se guarda en `airline_key` para que la curva de saldo use la misma.
    for f in &ferries {
        let key = reg_to_airline.get(&f.registration);
        sqlx::query("UPDATE aircraft_ferry SET airline_key = ? WHERE id = ?")
            .bind(key)
            .bind(f.id)
            .execute(&mut *tx)
            .await?;
        if let Some(acc) = key.and_then(|k| ledger.get_mut(k)) {
            acc.cost_ferry += f.cost_total;
            acc.costs += f.cost_total;
            acc.ferries += 1;
        }
    }

    // (v8.3 · 6.2c) Operación autónoma: entra en los totales y en cada
    // categoría del desglose; los costos fijos van aparte.
    for (key, a) in &auto {
        let Some(acc) = ledger.get_mut(key) else { continue };
        let revenue = a.revenue_tickets + a.revenue_ancillary;
        let variable = a.cost_fuel + a.cost_catering + a.cost_handling + a.cost_maintenance + a.cost_fees;
        acc.revenue += revenue;
        acc.costs += variable + a.cost_fixed;
        acc.revenue_tickets += a.revenue_tickets;
        acc.revenue_ancillary += a.revenue_ancillary;
        acc.cost_fuel += a.cost_fuel;
        acc.cost_catering += a.cost_catering;
        acc.cost_handling += a.cost_handling;
        acc.cost_maintenance += a.cost_maintenance;
        acc.cost_fees += a.cost_fees;
        acc.cost_fixed += a.cost_fixed;
        acc.auto_days = a.days;
        acc.auto_flights = a.flights;
        acc.auto_revenue = revenue;
        acc.auto_costs = variable + a.cost_fixed;
        acc.auto_fleet = a.last_fleet;
    }

    for (key, acc) in &ledger {
        // (v6.1) Misma política automática usada en el P&L (servicios por tipo +
        // nivel de mantenimiento manual) para que la satisfacción cuadre.
        let stored_level = policies.get(key).map(|p| p.maintenance_level).unwrap_or(1);
        let mut policy = auto_services(acc.lowcost, acc.cargo);
        policy.maintenance_level = stored_level;
        let avg_fpm = if acc.fpm_count > 0 {
            Some(acc.fpm_sum / acc.fpm_count as f64)
        } else {
            None
        };
        let satisfaction = Some(holistic_satisfaction(acc, &policy));
        sqlx::query(
            r#"INSERT INTO airline_economy
                (airline_key, airline_icao, airline_name, is_lowcost, is_cargo,
                 start_valuation, revenue, costs, flights,
                 passengers, revenue_tickets, revenue_ancillary,
                 cost_fuel, cost_catering, cost_handling, cost_maintenance,
                 cost_fees, avg_landing_fpm, satisfaction, gsx_flights,
                 fleet_size, fleet_value, cost_ferry, ferries,
                 auto_flights, auto_days, auto_revenue, auto_costs, cost_fixed, auto_fleet,
                 updated_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(key)
        .bind(&acc.icao)
        .bind(&acc.name)
        .bind(acc.lowcost as i64)
        .bind(acc.cargo as i64)
        .bind(acc.valuation)
        .bind(acc.revenue)
        .bind(acc.costs)
        .bind(acc.flights)
        .bind(acc.passengers)
        .bind(acc.revenue_tickets)
        .bind(acc.revenue_ancillary)
        .bind(acc.cost_fuel)
        .bind(acc.cost_catering)
        .bind(acc.cost_handling)
        .bind(acc.cost_maintenance)
        .bind(acc.cost_fees)
        .bind(avg_fpm)
        .bind(satisfaction)
        .bind(acc.gsx_flights)
        .bind(acc.fleet_regs.len() as i64)
        .bind(acc.fleet_value)
        .bind(acc.cost_ferry)
        .bind(acc.ferries)
        .bind(acc.auto_flights)
        .bind(acc.auto_days)
        .bind(acc.auto_revenue)
        .bind(acc.auto_costs)
        .bind(acc.cost_fixed)
        .bind(acc.auto_fleet)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Fila del ledger por aerolínea, lista para el frontend (la pantalla aún no
/// existe; este es el contrato que consumirá cuando se construya).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlineLedger {
    pub key: String,
    pub icao: Option<String>,
    pub name: String,
    pub lowcost: bool,
    /// Operador de carga (sin pasajeros; ingreso por flete).
    pub cargo: bool,
    /// Valor base USD ("banco").
    pub valuation: f64,
    pub revenue: f64,
    pub costs: f64,
    /// `revenue - costs` acumulado.
    pub net: f64,
    /// `valuation + net` — saldo actual.
    pub balance: f64,
    pub flights: i64,
    pub passengers: i64,
    pub revenue_tickets: f64,
    pub revenue_ancillary: f64,
    pub cost_fuel: f64,
    pub cost_catering: f64,
    pub cost_handling: f64,
    pub cost_maintenance: f64,
    pub cost_fees: f64,
    /// Media de FPM al tocar pista (negativo). `None` si ningún vuelo lo midió.
    pub avg_landing_fpm: Option<f64>,
    /// Satisfacción del pasajero 0-100 (derivada del FPM). `None` si no hay dato.
    pub satisfaction: Option<f64>,
    /// Nº de vuelos cuyos costes salieron de facturas GSX reales.
    pub gsx_flights: i64,
    /// Aviones distintos comprados (matrículas/liveries voladas).
    pub fleet_size: i64,
    /// Inversión total en flota (suma de precios de compra) — descontada del saldo.
    pub fleet_value: f64,
    /// (v8.3) Costo de los traslados (ya incluido en `costs`) y cuántos hubo.
    pub cost_ferry: f64,
    pub ferries: i64,
    /// (v8.3 · 6.2c) Operación autónoma (ya incluida en revenue / costs):
    /// días completos operados, vuelos reales, ingresos, costos (con fijos),
    /// costos fijos y flota del último día.
    pub auto_days: i64,
    pub auto_flights: i64,
    pub auto_revenue: f64,
    pub auto_costs: f64,
    pub cost_fixed: f64,
    pub auto_fleet: i64,
}

/// (v8.3 · Etapa 6.2b) Matrículas que el usuario voló con una aerolínea
/// (clave canónica, = ICAO cuando se conoce): normalizada → (matrícula como
/// la trae su vuelo más reciente, modelo). Son "sus aviones" en la flota.
pub async fn user_registrations(
    pool: &SqlitePool,
    key: &str,
) -> Result<HashMap<String, (String, Option<String>)>> {
    let flights = flight_log::list_entries(pool).await?;
    let learned = learn_airline_icaos(&flights);
    let mut out = HashMap::new();
    for f in &flights {
        let Some(reg) = f.aircraft_registration.as_deref().map(str::trim).filter(|r| !r.is_empty()) else {
            continue;
        };
        if resolve_airline(f, &learned).map(|r| r.key).as_deref() != Some(key) {
            continue;
        }
        let n = normalize_reg(reg);
        if !n.is_empty() {
            out.entry(n).or_insert_with(|| {
                (reg.to_ascii_uppercase(), f.aircraft_model.clone().or_else(|| f.aircraft_atc_type.clone()))
            });
        }
    }
    Ok(out)
}

/// Recalcula y devuelve el ledger por aerolínea ordenado por resultado neto.
pub async fn list_economy(pool: &SqlitePool) -> Result<Vec<AirlineLedger>> {
    recompute(pool).await?;

    let rows = sqlx::query(
        r#"SELECT airline_key, airline_icao, airline_name, is_lowcost, is_cargo,
                  start_valuation, revenue, costs, flights, passengers,
                  revenue_tickets, revenue_ancillary, cost_fuel, cost_catering,
                  cost_handling, cost_maintenance, cost_fees, avg_landing_fpm,
                  satisfaction, gsx_flights, fleet_size, fleet_value,
                  cost_ferry, ferries, auto_flights, auto_days, auto_revenue,
                  auto_costs, cost_fixed, auto_fleet
             FROM airline_economy
            ORDER BY (revenue - costs) DESC"#,
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let valuation: f64 = r.get("start_valuation");
        let revenue: f64 = r.get("revenue");
        let costs: f64 = r.get("costs");
        let fleet_value: f64 = r.get("fleet_value");
        let net = revenue - costs;
        out.push(AirlineLedger {
            key: r.get("airline_key"),
            icao: r.get("airline_icao"),
            name: r.get("airline_name"),
            lowcost: r.get::<i64, _>("is_lowcost") != 0,
            cargo: r.get::<i64, _>("is_cargo") != 0,
            valuation,
            revenue,
            costs,
            net,
            // El saldo descuenta la inversión en flota (los aviones comprados).
            balance: valuation + net - fleet_value,
            flights: r.get("flights"),
            passengers: r.get("passengers"),
            revenue_tickets: r.get("revenue_tickets"),
            revenue_ancillary: r.get("revenue_ancillary"),
            cost_fuel: r.get("cost_fuel"),
            cost_catering: r.get("cost_catering"),
            cost_handling: r.get("cost_handling"),
            cost_maintenance: r.get("cost_maintenance"),
            cost_fees: r.get("cost_fees"),
            avg_landing_fpm: r.get("avg_landing_fpm"),
            satisfaction: r.get("satisfaction"),
            gsx_flights: r.get("gsx_flights"),
            fleet_size: r.get("fleet_size"),
            fleet_value,
            cost_ferry: r.get("cost_ferry"),
            ferries: r.get("ferries"),
            auto_days: r.get("auto_days"),
            auto_flights: r.get("auto_flights"),
            auto_revenue: r.get("auto_revenue"),
            auto_costs: r.get("auto_costs"),
            cost_fixed: r.get("cost_fixed"),
            auto_fleet: r.get("auto_fleet"),
        });
    }
    Ok(out)
}

/// Punto de la curva de saldo en el tiempo.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancePoint {
    /// Timestamp ISO-8601 del evento.
    pub t: String,
    /// Saldo ("banco") acumulado tras este evento.
    pub balance: f64,
    /// Variación del saldo en este evento.
    pub delta: f64,
    /// Etiqueta legible (ruta o servicio de mantenimiento).
    pub label: String,
    /// "start" | "flight" | "maint".
    pub kind: String,
}

/// Serie temporal del saldo de una aerolínea, reconstruida vuelo a vuelo desde
/// `flight_pnl` + `flight_log`: parte del valor base, suma el neto de cada vuelo
/// en orden cronológico, descuenta la compra de cada avión la primera vez que
/// aparece y los servicios de mantenimiento en su fecha real. El último punto
/// coincide con el `balance` de `list_economy`.
///
/// Asume que `flight_pnl` está al día (lo deja `list_economy`, que corre al
/// abrir la pantalla); no recalcula para que abrir el detalle sea instantáneo.
pub async fn balance_history(pool: &SqlitePool, key: &str) -> Result<Vec<BalancePoint>> {
    let valuation: f64 =
        sqlx::query_scalar("SELECT start_valuation FROM airline_economy WHERE airline_key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?
            .unwrap_or(0.0);

    let rows = sqlx::query(
        r#"SELECT fp.net AS net, fl.started_at AS started_at,
                  fl.aircraft_registration AS reg, fl.aircraft_model AS model,
                  fl.aircraft_atc_type AS atc,
                  fl.origin_icao AS origin, fl.destination_icao AS dest
             FROM flight_pnl fp
             JOIN flight_log fl ON fl.id = fp.flight_id
            WHERE fp.airline_key = ? AND fl.started_at IS NOT NULL
            ORDER BY fl.started_at ASC"#,
    )
    .bind(key)
    .fetch_all(pool)
    .await?;

    struct Ev {
        t: String,
        delta: f64,
        label: String,
        kind: &'static str,
    }
    let mut events: Vec<Ev> = Vec::with_capacity(rows.len());
    let mut seen_regs: HashSet<String> = HashSet::new();
    let mut airline_regs: HashSet<String> = HashSet::new();

    for r in &rows {
        let net: f64 = r.get("net");
        let t: String = r.get("started_at");
        let reg_raw: Option<String> = r.get("reg");
        let model: Option<String> = r.get("model");
        let atc: Option<String> = r.get("atc");
        let origin: Option<String> = r.get("origin");
        let dest: Option<String> = r.get("dest");

        let mut delta = net;
        let mut bought = false;
        if let Some(rr) = reg_raw.as_deref() {
            let nreg = normalize_reg(rr);
            if !nreg.is_empty() {
                airline_regs.insert(nreg.clone());
                if seen_regs.insert(nreg) {
                    let (size, _) = classify(model.as_deref().or(atc.as_deref()));
                    delta -= purchase_price(size);
                    bought = true;
                }
            }
        }
        let route = match (origin.as_deref(), dest.as_deref()) {
            (Some(o), Some(d)) => format!("{o} → {d}"),
            (Some(o), None) => o.to_string(),
            (None, Some(d)) => d.to_string(),
            _ => crate::tr!("economy.flight"),
        };
        let label = if bought {
            crate::tr!("economy.aircraftPurchase", route = route)
        } else {
            route
        };
        events.push(Ev {
            t,
            delta,
            label,
            kind: "flight",
        });
    }

    // Servicios de mantenimiento de las matrículas de esta aerolínea, en su fecha.
    let maint = sqlx::query("SELECT registration, component, cost, serviced_at FROM maintenance_log")
        .fetch_all(pool)
        .await?;
    for m in &maint {
        let reg: String = m.get("registration");
        if !airline_regs.contains(&reg) {
            continue;
        }
        let cost: f64 = m.get("cost");
        let comp: String = m.get("component");
        let t: String = m.get("serviced_at");
        events.push(Ev {
            t,
            delta: -cost,
            label: crate::tr!("economy.maintenance", comp = comp),
            kind: "maint",
        });
    }

    // (v8.3 · 6.2c) Operación autónoma: un punto por día (neto con fijos).
    for r in sqlx::query(
        "SELECT day, flights, revenue_tickets + revenue_ancillary AS rev,
                cost_fuel + cost_catering + cost_handling + cost_maintenance + cost_fees + cost_fixed AS cost
           FROM auto_daily WHERE airline_key = ? ORDER BY day",
    )
    .bind(key)
    .fetch_all(pool)
    .await?
    {
        let day: String = r.get("day");
        let rev: f64 = r.get("rev");
        let cost: f64 = r.get("cost");
        let label_day = day.get(5..).map(|md| format!("{}/{}", &md[3..], &md[..2])).unwrap_or_else(|| day.clone());
        events.push(Ev {
            t: format!("{day}T23:59:59Z"),
            delta: rev - cost,
            label: crate::tr!("economy.autoDay", date = label_day, n = r.get::<i64, _>("flights")),
            kind: "auto",
        });
    }

    // (v8.3) Traslados cargados a esta aerolínea, en su fecha de salida.
    for f in crate::ferry::for_airline(pool, key).await.unwrap_or_default() {
        let label = if f.source == "auto" {
            crate::tr!("economy.ferryAuto", from = f.from_icao, to = f.to_icao)
        } else {
            crate::tr!("economy.ferry", from = f.from_icao, to = f.to_icao)
        };
        events.push(Ev {
            t: f.requested_at,
            delta: -f.cost_total,
            label,
            kind: "ferry",
        });
    }

    events.sort_by(|a, b| a.t.cmp(&b.t));

    let mut out: Vec<BalancePoint> = Vec::with_capacity(events.len() + 1);
    let mut bal = valuation;
    if let Some(first) = events.first() {
        out.push(BalancePoint {
            t: first.t.clone(),
            balance: bal,
            delta: 0.0,
            label: crate::tr!("economy.start"),
            kind: "start".to_string(),
        });
    }
    for ev in events {
        bal += ev.delta;
        out.push(BalancePoint {
            t: ev.t,
            balance: bal,
            delta: ev.delta,
            label: ev.label,
            kind: ev.kind.to_string(),
        });
    }
    Ok(out)
}

// ──────────────────────── Política de gestión: persistencia ───────────────────

/// Carga todas las políticas configuradas, indexadas por `airline_key`.
async fn load_policies(pool: &SqlitePool) -> Result<HashMap<String, AirlinePolicy>> {
    let rows = sqlx::query(
        r#"SELECT airline_key, maintenance_level, snacks, meals, wifi,
                  seat_upgrades, priority_boarding, extra_baggage
             FROM airline_policy"#,
    )
    .fetch_all(pool)
    .await?;
    let mut map = HashMap::with_capacity(rows.len());
    for r in rows {
        let key: String = r.get("airline_key");
        map.insert(
            key,
            AirlinePolicy {
                maintenance_level: r.get("maintenance_level"),
                snacks: r.get::<i64, _>("snacks") != 0,
                meals: r.get::<i64, _>("meals") != 0,
                wifi: r.get::<i64, _>("wifi") != 0,
                seat_upgrades: r.get::<i64, _>("seat_upgrades") != 0,
                priority_boarding: r.get::<i64, _>("priority_boarding") != 0,
                extra_baggage: r.get::<i64, _>("extra_baggage") != 0,
            },
        );
    }
    Ok(map)
}

/// Política de una aerolínea (default si nunca se configuró).
pub async fn get_policy(pool: &SqlitePool, key: &str) -> Result<AirlinePolicy> {
    Ok(load_policies(pool).await?.remove(key).unwrap_or_default())
}

/// Guarda la política de una aerolínea y recalcula el ledger para que los
/// números reflejen la decisión al instante.
pub async fn set_policy(pool: &SqlitePool, key: &str, p: &AirlinePolicy) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO airline_policy
            (airline_key, maintenance_level, snacks, meals, wifi,
             seat_upgrades, priority_boarding, extra_baggage, updated_at)
           VALUES (?,?,?,?,?,?,?,?,?)
           ON CONFLICT(airline_key) DO UPDATE SET
             maintenance_level = excluded.maintenance_level,
             snacks            = excluded.snacks,
             meals             = excluded.meals,
             wifi              = excluded.wifi,
             seat_upgrades     = excluded.seat_upgrades,
             priority_boarding = excluded.priority_boarding,
             extra_baggage     = excluded.extra_baggage,
             updated_at        = excluded.updated_at"#,
    )
    .bind(key)
    .bind(p.maintenance_level)
    .bind(p.snacks as i64)
    .bind(p.meals as i64)
    .bind(p.wifi as i64)
    .bind(p.seat_upgrades as i64)
    .bind(p.priority_boarding as i64)
    .bind(p.extra_baggage as i64)
    .bind(&now)
    .execute(pool)
    .await?;
    recompute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod auto_ops_tests {
    use super::*;

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn exec(pool: &SqlitePool, sql: &str) {
        sqlx::query(sql).execute(pool).await.unwrap();
    }

    /// Un día con sus 12 ventanas completas cuenta; uno con 11 no. El avión
    /// sin tipo usa el tipo más común de la flota y el avión del usuario que
    /// no se vio volar también paga costo fijo.
    #[tokio::test]
    async fn complete_days_become_daily_pnl_with_fixed_costs() {
        let pool = db().await;
        exec(&pool, "INSERT INTO airports (icao, name, latitude, longitude) VALUES
            ('KBOS','Boston',42.3656,-71.0096), ('KJFK','JFK',40.6413,-73.7781)").await;
        // Vuelo del usuario con JetBlue → JBU está en Finanzas.
        exec(&pool, "INSERT INTO flight_log (started_at, ended_at, origin_lat, origin_lon, origin_icao,
                destination_icao, aircraft_registration, aircraft_model, airline_icao)
             VALUES ('2026-10-10T10:00:00Z','2026-10-10T11:00:00Z',0,0,'KBOS','KJFK','N657JB','A320','JBU')").await;
        let d21 = 1_792_540_800i64; // 2026-10-21 00:00 UTC
        let d22 = d21 + 86_400;
        // 21/10 completo (12 ventanas, bajadas tras cerrar el día); 22/10 con 11.
        for i in 0..12 {
            sqlx::query("INSERT INTO opensky_window (begin_ts, end_ts, fetched_at, status) VALUES (?, ?, '2026-10-22T07:00:00Z', 'ok')")
                .bind(d21 + i * 7_200)
                .bind(d21 + (i + 1) * 7_200)
                .execute(&pool)
                .await
                .unwrap();
        }
        for i in 0..11 {
            sqlx::query("INSERT INTO opensky_window (begin_ts, end_ts, fetched_at, status) VALUES (?, ?, '2026-10-23T07:00:00Z', 'ok')")
                .bind(d22 + i * 7_200)
                .bind(d22 + (i + 1) * 7_200)
                .execute(&pool)
                .await
                .unwrap();
        }
        exec(&pool, "INSERT INTO aircraft_db (icao24, registration, typecode, source) VALUES
            ('aaa001','N100JB','A320','opensky'), ('aaa003','N657JB','A320','opensky')").await;
        for (hex, fs, win) in [("aaa001", d21 + 3_600, d21), ("aaa001", d21 + 20_000, d21 + 14_400), ("aaa002", d21 + 30_000, d21 + 28_800), ("aaa001", d22 + 3_600, d22)] {
            sqlx::query("INSERT INTO real_flight (icao24, first_seen, last_seen, callsign, airline_icao, dep_icao, arr_icao, window_begin)
                         VALUES (?, ?, ?, 'JBU123', 'JBU', 'KBOS', 'KJFK', ?)")
                .bind(hex)
                .bind(fs)
                .bind(fs + 3_600)
                .bind(win)
                .execute(&pool)
                .await
                .unwrap();
        }
        let flights = flight_log::list_entries(&pool).await.unwrap();
        let learned = learn_airline_icaos(&flights);
        let policies = load_policies(&pool).await.unwrap();
        assert_eq!(refresh_auto_days(&pool, &flights, &learned, &policies).await.unwrap(), 1);
        let totals = load_auto_totals(&pool).await.unwrap();
        let jbu = &totals["JBU"];
        assert_eq!((jbu.days, jbu.flights), (1, 3), "sólo el 21/10 (el 22/10 está incompleto)");
        // Flota: aaa001 (A320) + aaa002 (sin tipo → A320) + N657JB del usuario (no visto).
        assert_eq!(jbu.last_fleet, 3);
        assert!((jbu.cost_fixed - 3.0 * 8_000.0).abs() < 1e-6, "{}", jbu.cost_fixed);
        assert!(jbu.revenue_tickets > 0.0 && jbu.cost_fuel > 0.0);
        // Idempotente: recalcular no duplica.
        refresh_auto_days(&pool, &flights, &learned, &policies).await.unwrap();
        assert_eq!(load_auto_totals(&pool).await.unwrap()["JBU"].flights, 3);
    }
}

#[cfg(test)]
mod gsx_receipt_tests {
    use super::*;

    #[test]
    fn usd_comes_from_the_gsx_dollar_equivalent() {
        assert_eq!(usd_from_total("C$22.05 ~$ 15.54"), Some(15.54));
        assert_eq!(usd_from_total("CN¥43,210.00 ~$ 5,987.20"), Some(5987.20));
        assert_eq!(usd_from_total("$6,000.00"), Some(6000.0));
        assert_eq!(usd_from_total("US$ 99.9"), Some(99.9));
        // Otra moneda sin equivalencia: no se inventa (C$ no son dólares).
        assert_eq!(usd_from_total("C$22.05"), None);
    }

    #[test]
    fn json_receipt_total() {
        let j = r#"{"title":"FUEL DELIVERY RECEIPT","subtotal":"C$21.00 ~$ 14.80","total":"C$22.05 ~$ 15.54"}"#;
        assert_eq!(parse_total_json(j), Some(15.54));
        assert_eq!(parse_total_json("{}"), None);
    }

    #[test]
    fn html_receipt_takes_the_total_not_the_subtotal() {
        let html = "<tr><td>Subtotal</td><td>$1,000.00</td></tr>\
                    <tr><td>Tax 8%</td><td>$80.00</td></tr>\
                    <tr><td><b>Total</b></td><td>$1,080.00</td></tr>";
        assert_eq!(parse_total_html(html), Some(1080.0));
        let local = "<td>Subtotal</td><td>C$21.00 ~$ 14.80</td><td>Total</td><td>C$22.05 ~$ 15.54</td>";
        assert_eq!(parse_total_html(local), Some(15.54));
    }
}
