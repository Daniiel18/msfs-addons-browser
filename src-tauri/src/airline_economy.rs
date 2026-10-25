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
    // (v8.3) Aquí sólo la aerolínea de carga CONOCIDA; si sus vuelos son todos
    // de carga lo decide `airline_cargo_flags`, y cada vuelo `is_cargo_flight`.
    let cargo = known.map(|p| p.cargo).unwrap_or(false);
    Some(Resolved {
        key: canon.key,
        icao: canon.icao,
        name: canon.name,
        valuation,
        lowcost,
        cargo,
    })
}

/// (v8.3) Asientos auxiliares (jumpseats): lo máximo que lleva un carguero.
const JUMPSEATS: i64 = 3;

/// (v8.3) Modelo/título de un carguero: 777F, 747-8F, 767-300F, MD-11F,
/// A330-200F, 737-800BCF/SF, P2F, "Freighter", "Cargo"… y los códigos de
/// cliente de la base de aviones (777-FB5, 747-8HVF, 767-3S2F).
static FREIGHTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(freighter|cargo|\bbcf\b|\bbdsf\b|p2f\b|\d{3}sf\b|\bsf\b|md-?11f\b|777-?f|(?:^|[^0-9])\d{3}(?:-[0-9a-z]{1,5})?f\b)")
        .unwrap()
});

pub(crate) fn is_freighter(text: Option<&str>) -> bool {
    text.is_some_and(|t| FREIGHTER_RE.is_match(t))
}

/// (v8.3) ¿Es un vuelo de carga? Aerolínea de carga, avión carguero, o con
/// carga y como mucho los jumpseats a bordo (OFP de SimBrief).
fn is_cargo_flight(e: &FlightLogEntry, airline_cargo: bool) -> bool {
    airline_cargo
        || is_freighter(e.aircraft_model.as_deref())
        || is_freighter(e.aircraft_title.as_deref())
        || (e.cargo_kg.unwrap_or(0) > 0 && e.passengers.unwrap_or(0) <= JUMPSEATS)
}

/// (v8.3) Aerolíneas "de carga": las conocidas como tales y las que tienen
/// TODOS sus vuelos de carga. Una aerolínea de pasaje con algún carguero
/// (p. ej. Korean Air con un 777F) queda mixta: cada vuelo se valora por sí.
fn airline_cargo_flags(flights: &[FlightLogEntry], learned: &HashMap<String, String>) -> HashMap<String, bool> {
    let mut count: HashMap<String, (bool, i64, i64)> = HashMap::new();
    for f in flights {
        let Some(res) = resolve_airline(f, learned) else { continue };
        let c = count.entry(res.key).or_insert((res.cargo, 0, 0));
        c.1 += 1;
        if is_cargo_flight(f, c.0) {
            c.2 += 1;
        }
    }
    count
        .into_iter()
        .map(|(k, (known, n, cargo))| (k, known || (n > 0 && cargo == n)))
        .collect()
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

// (24/10) Contabilidad sólida: cada LÍNEA de cada recibo va, como mucho, a UN
// vuelo. Antes un recibo de handling con el desembarque de un vuelo y el
// embarque del siguiente se cobraba entero a los dos (ventana de ±4 h), y la
// hora del recibo era la del SIMULADOR (con hora del sim distinta a la real no
// casaba con ningún vuelo).

#[derive(Clone, Copy, PartialEq, Debug)]
enum GsxKind {
    Fuel,
    Catering,
    Handling,
    Bus,
}

impl GsxKind {
    fn id(self) -> &'static str {
        match self {
            GsxKind::Fuel => "fuel",
            GsxKind::Catering => "catering",
            GsxKind::Handling => "handling",
            GsxKind::Bus => "bus",
        }
    }
}

/// Lado del servicio: el vuelo que SALE del aeropuerto o el que LLEGA.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Dep,
    Arr,
}

/// Una línea de un recibo de GSX, en USD con su parte de impuestos.
#[derive(Clone, Debug)]
struct GsxLine {
    receipt_id: String,
    line: usize,
    kind: GsxKind,
    service: String,
    side: Side,
    airport: String, // ICAO en mayúsculas
    reg: String,     // matrícula normalizada (alfanumérica, mayúsculas)
    at: DateTime<Utc>,
    usd: f64,
    /// Carga de combustible: NO es gasto del vuelo (se paga lo quemado), da el
    /// precio por kg del aeropuerto.
    uplift_kg: Option<f64>,
}

impl GsxLine {
    fn is_cost(&self) -> bool {
        self.uplift_kg.is_none()
    }
}

/// Lo que GSX facturó a un vuelo (None = sin línea de ese tipo → estimado).
#[derive(Default, Debug)]
struct GsxCosts {
    /// USD/kg del combustible cargado con GSX (este vuelo o su aeropuerto).
    fuel_price_kg: Option<f64>,
    catering: Option<f64>,
    handling_dep: Option<f64>,
    handling_arr: Option<f64>,
    /// Líneas de GSX cobradas a este vuelo.
    lines: usize,
}

impl GsxCosts {
    fn matched(&self) -> bool {
        self.lines > 0
    }
}

/// Carpeta de recibos de GSX, resuelta vía `%APPDATA%` del usuario actual
/// (NO hardcodeada — funciona igual para otra cuenta/equipo). Sólo Windows.
fn receipts_dir() -> Option<PathBuf> {
    // Las pruebas no leen los recibos reales de la máquina (salvo la de la
    // copia real, `ECON_DB`).
    if cfg!(test) && std::env::var_os("ECON_DB").is_none() {
        return None;
    }
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
#[cfg(test)]
fn parse_total_json(json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    usd_from_total(v.get("total")?.as_str()?)
}

static KG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"([0-9][0-9,]*(?:\.[0-9]+)?)\s*kg").unwrap());

/// Una línea ya leída de un recibo (sin aeropuerto/matrícula/hora).
#[derive(Debug, Clone, PartialEq)]
struct ReceiptLine {
    service: String,
    side: Side,
    usd: f64,
    uplift_kg: Option<f64>,
}

fn has_any(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

/// (24/10) Líneas de un recibo `.json` de GSX, en USD y con su parte de los
/// impuestos (total ÷ subtotal). Lado de cada línea:
/// - desembarque / descarga → el vuelo que LLEGA;
/// - embarque, pushback, carga, catering, desabastecimiento → el que SALE;
/// - jetway/escaleras/GPU (no dicen de qué lado son) → el lado del resto del
///   recibo (salida si tiene embarque/pushback).
///
/// Las cargas de combustible salen con `uplift_kg` (precio, no gasto). Las
/// líneas de 0 USD no cuentan. Si no hay líneas legibles, una sola con el total.
fn parse_receipt_lines(json: &str, kind: GsxKind) -> Vec<ReceiptLine> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let usd_of = |k: &str| v.get(k).and_then(|x| x.as_str()).and_then(usd_from_total);
    let total = usd_of("total");
    let factor = match (total, usd_of("subtotal")) {
        (Some(t), Some(s)) if s > 0.0 && t > 0.0 => t / s,
        _ => 1.0,
    };
    let items: Vec<serde_json::Value> = match v.get("items") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(o @ serde_json::Value::Object(_)) => vec![o.clone()],
        _ => Vec::new(),
    };
    const ARR: &[&str] = &["deboard", "disembark", "unload", "arrival"];
    const DEP: &[&str] = &["boarding", "pushback", "push back", "loading", "departure", "catering", "meal", "snack", "beverage", "defuel"];
    let mut raw: Vec<(String, Option<Side>, f64, Option<f64>)> = Vec::new();
    for it in &items {
        let desc = it
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let Some(usd) = it.get("amount").and_then(|x| x.as_str()).and_then(usd_from_total) else {
            continue;
        };
        let usd = (usd * factor * 100.0).round() / 100.0;
        if usd <= 0.0 {
            continue;
        }
        let low = desc.to_lowercase();
        if kind == GsxKind::Fuel && !low.contains("defuel") {
            let kg = it
                .get("qty")
                .and_then(|x| x.as_str())
                .and_then(|q| KG_RE.captures(q))
                .and_then(|c| c[1].replace(',', "").parse::<f64>().ok());
            // Sin kilos no hay precio que sacar: la línea no aporta nada.
            if let Some(kg) = kg.filter(|k| *k > 0.0) {
                raw.push((desc, Some(Side::Dep), usd, Some(kg)));
            }
            continue;
        }
        // "deboarding" contiene "boarding": se mira primero la llegada.
        let side = if has_any(&low, ARR) {
            Some(Side::Arr)
        } else if kind == GsxKind::Catering || has_any(&low, DEP) {
            Some(Side::Dep)
        } else {
            None
        };
        raw.push((desc, side, usd, None));
    }
    if raw.is_empty() {
        if let Some(t) = total.filter(|t| *t > 0.0 && kind != GsxKind::Fuel) {
            raw.push(("Total".into(), None, t, None));
        }
    }
    // Lo ambiguo va del lado del resto del recibo; salida si tiene salida.
    let has_dep = raw.iter().any(|r| r.1 == Some(Side::Dep) && r.3.is_none());
    let has_arr = raw.iter().any(|r| r.1 == Some(Side::Arr));
    let fallback = if has_dep || !has_arr { Side::Dep } else { Side::Arr };
    raw.into_iter()
        .map(|(service, side, usd, uplift_kg)| ReceiptLine { service, side: side.unwrap_or(fallback), usd, uplift_kg })
        .collect()
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

/// Lee todos los recibos de GSX (Fuel/Catering/Handling/PassengerBus) como
/// líneas. Cada recibo viene como `.json` + `.html` con el mismo nombre: manda
/// el `.json` (líneas); el `.html` sólo si no hay `.json` (una línea con el
/// total). Hora = la REAL en que GSX escribió el archivo; la del nombre es la
/// del simulador y sólo se usa si no se puede leer la del archivo.
/// Tolerante: si la carpeta no existe o un archivo no parsea, se omite.
fn scan_receipts() -> Vec<GsxLine> {
    let Some(base) = receipts_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (sub, kind) in [
        ("Fuel", GsxKind::Fuel),
        ("Catering", GsxKind::Catering),
        ("Handling", GsxKind::Handling),
        ("PassengerBus", GsxKind::Bus),
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
            let json_path = dir.join(format!("{stem}.json"));
            let html_path = dir.join(format!("{stem}.html"));
            let real_at = std::fs::metadata(&json_path)
                .or_else(|_| std::fs::metadata(&html_path))
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(|| naive.and_utc());
            let (receipt_id, lines) = match std::fs::read_to_string(&json_path) {
                Ok(j) => (
                    serde_json::from_str::<serde_json::Value>(&j)
                        .ok()
                        .and_then(|v| v.get("receiptId").and_then(|x| x.as_str()).map(String::from))
                        .unwrap_or_else(|| format!("{sub}/{stem}")),
                    parse_receipt_lines(&j, kind),
                ),
                Err(_) => (
                    format!("{sub}/{stem}"),
                    std::fs::read_to_string(&html_path)
                        .ok()
                        .and_then(|h| parse_total_html(&h))
                        .filter(|t| *t > 0.0 && kind != GsxKind::Fuel)
                        .map(|t| vec![ReceiptLine { service: "Total".into(), side: Side::Dep, usd: t, uplift_kg: None }])
                        .unwrap_or_default(),
                ),
            };
            if lines.is_empty() {
                tracing::debug!(target: "economy", "recibo GSX sin líneas cobrables: {sub}/{stem}");
            }
            for (i, l) in lines.into_iter().enumerate() {
                out.push(GsxLine {
                    receipt_id: receipt_id.clone(),
                    line: i,
                    kind,
                    service: l.service,
                    side: l.side,
                    airport: airport.clone(),
                    reg: reg.clone(),
                    at: real_at,
                    usd: l.usd,
                    uplift_kg: l.uplift_kg,
                });
            }
        }
    }
    out
}

fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// (24/10) Las líneas de GSX son registros contables: además de las de los
/// recibos de esta PC, cuentan las ya guardadas (de la nube o de recibos que
/// ya no están en disco). Un recibo presente en disco manda sobre lo guardado.
async fn all_gsx_lines(pool: &SqlitePool) -> Vec<GsxLine> {
    let mut lines = scan_receipts();
    let on_disk: std::collections::HashSet<String> = lines.iter().map(|l| l.receipt_id.clone()).collect();
    let stored = sqlx::query(
        "SELECT receipt_id, line, kind, service, side, airport, registration, at, amount_usd, uplift_kg FROM gsx_charge",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for r in stored {
        let receipt_id: String = r.get("receipt_id");
        if on_disk.contains(&receipt_id) {
            continue;
        }
        let kind = match r.get::<String, _>("kind").as_str() {
            "fuel" => GsxKind::Fuel,
            "catering" => GsxKind::Catering,
            "bus" => GsxKind::Bus,
            _ => GsxKind::Handling,
        };
        let Some(at) = parse_dt(&r.get::<String, _>("at")) else { continue };
        lines.push(GsxLine {
            receipt_id,
            line: r.get::<i64, _>("line").max(0) as usize,
            kind,
            service: r.get("service"),
            side: if r.get::<String, _>("side") == "arr" { Side::Arr } else { Side::Dep },
            airport: r.get("airport"),
            reg: r.get("registration"),
            at,
            usd: r.get("amount_usd"),
            uplift_kg: r.get("uplift_kg"),
        });
    }
    lines
}

/// Antes de la salida (embarque, catering, combustible…) el servicio puede ir
/// hasta 8 h antes del bloque; el pushback llega poco después del OUT.
const DEP_BEFORE: i64 = 8 * 60;
const DEP_AFTER: i64 = 60;
/// El desembarque llega tras el IN (hasta 8 h si el recibo se cierra tarde).
const ARR_AFTER: i64 = 8 * 60;
const ARR_BEFORE: i64 = 60;

/// (24/10) Vuelo de cada línea (índice paralelo a `lines`; `None` = ninguno).
/// Una línea va como mucho a UN vuelo: el de esa matrícula que SALE de ese
/// aeropuerto más cerca después de la línea (salida) o el que LLEGA más cerca
/// antes (llegada). Aeropuerto "UNKWN": sólo matrícula + hora.
fn assign_lines(lines: &[GsxLine], flights: &[FlightLogEntry]) -> Vec<Option<i64>> {
    struct F {
        id: i64,
        reg: String,
        origin: Option<String>,
        dest: Option<String>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    }
    let fs: Vec<F> = flights
        .iter()
        .filter_map(|e| {
            let reg = normalize_reg(e.aircraft_registration.as_deref()?);
            let start = parse_dt(&e.started_at)?;
            let end = e.ended_at.as_deref().and_then(parse_dt).unwrap_or(start);
            (!reg.is_empty()).then(|| F {
                id: e.id,
                reg,
                origin: e.origin_icao.as_deref().map(|s| s.to_ascii_uppercase()),
                dest: e.destination_icao.as_deref().map(|s| s.to_ascii_uppercase()),
                start,
                end,
            })
        })
        .collect();
    lines
        .iter()
        .map(|l| {
            let here = |a: &Option<String>| l.airport == UNKNOWN_AIRPORT || a.as_deref() == Some(l.airport.as_str());
            fs.iter()
                .filter(|f| f.reg == l.reg)
                .filter_map(|f| match l.side {
                    Side::Dep if here(&f.origin) => {
                        let mins = (f.start - l.at).num_minutes();
                        (-DEP_AFTER..=DEP_BEFORE).contains(&mins).then_some((mins.abs(), f.id))
                    }
                    Side::Arr if here(&f.dest) => {
                        let mins = (l.at - f.end).num_minutes();
                        (-ARR_BEFORE..=ARR_AFTER).contains(&mins).then_some((mins.abs(), f.id))
                    }
                    _ => None,
                })
                .min()
                .map(|(_, id)| id)
        })
        .collect()
}

/// Servicio "igual" a efectos de repetición: todo el catering es uno; el
/// resto por su nombre sin lo que va entre paréntesis ("Pushback service (82
/// t MTOW)" = "Pushback service (77 t MTOW)").
fn service_key(l: &GsxLine) -> String {
    if l.kind == GsxKind::Catering {
        return "catering".into();
    }
    let mut out = String::new();
    let mut depth = 0;
    for ch in l.service.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            c if depth == 0 => out.push(c.to_ascii_lowercase()),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// (24/10) Servicios repetidos para un mismo vuelo (reinicios del sim,
/// vuelo recargado): de salida cuenta sólo el ÚLTIMO antes de despegar; de
/// llegada, el PRIMERO tras aterrizar. Los demás eran de intentos abortados:
/// quedan sin vuelo y no se cobran (decisión del usuario).
fn drop_repeats(lines: &[GsxLine], assigned: &mut [Option<i64>]) {
    let mut keep: HashMap<(i64, bool, String), (DateTime<Utc>, usize)> = HashMap::new();
    for (i, (l, fid)) in lines.iter().zip(assigned.iter()).enumerate() {
        let Some(fid) = *fid else { continue };
        if !l.is_cost() {
            continue;
        }
        let dep = l.side == Side::Dep;
        let k = (fid, dep, service_key(l));
        let better = |cur: DateTime<Utc>| if dep { l.at > cur } else { l.at < cur };
        match keep.get(&k) {
            Some((at, _)) if !better(*at) => {}
            _ => {
                keep.insert(k, (l.at, i));
            }
        }
    }
    let winners: std::collections::HashSet<usize> = keep.values().map(|(_, i)| *i).collect();
    for (i, (l, fid)) in lines.iter().zip(assigned.iter_mut()).enumerate() {
        if fid.is_some() && l.is_cost() && !winners.contains(&i) {
            // Mismo recibo que el ganador (p. ej. dos líneas iguales en uno)
            // no es un reintento: se queda.
            let same_receipt = keep.values().any(|(_, w)| lines[*w].receipt_id == l.receipt_id && service_key(&lines[*w]) == service_key(l));
            if !same_receipt {
                *fid = None;
            }
        }
    }
}

/// Precio por kg de las cargas de GSX (ponderado por kilos).
fn price_per_kg<'a>(uplifts: impl Iterator<Item = &'a GsxLine>) -> Option<f64> {
    let (usd, kg) = uplifts.fold((0.0, 0.0), |(u, k), l| (u + l.usd, k + l.uplift_kg.unwrap_or(0.0)));
    (kg > 0.0).then(|| usd / kg)
}

/// Lo que GSX facturó a cada vuelo, a partir de las líneas asignadas. El
/// precio del combustible: el de las cargas de ESE vuelo; si no hubo, el de
/// la última carga en su aeropuerto de salida (hasta 120 días antes).
fn gsx_costs_by_flight(
    lines: &[GsxLine],
    assigned: &[Option<i64>],
    flights: &[FlightLogEntry],
) -> HashMap<i64, GsxCosts> {
    let mut out: HashMap<i64, GsxCosts> = HashMap::new();
    for (l, fid) in lines.iter().zip(assigned) {
        let Some(fid) = fid else { continue };
        let c = out.entry(*fid).or_default();
        if !l.is_cost() {
            continue;
        }
        c.lines += 1;
        let slot = match (l.kind, l.side) {
            (GsxKind::Catering, _) => &mut c.catering,
            (_, Side::Dep) => &mut c.handling_dep,
            (_, Side::Arr) => &mut c.handling_arr,
        };
        *slot = Some(slot.unwrap_or(0.0) + l.usd);
    }
    for f in flights {
        let own = price_per_kg(
            lines
                .iter()
                .zip(assigned)
                .filter(|(l, a)| **a == Some(f.id) && !l.is_cost())
                .map(|(l, _)| l),
        );
        let price = own.or_else(|| {
            let origin = f.origin_icao.as_deref()?.to_ascii_uppercase();
            let start = parse_dt(&f.started_at)?;
            lines
                .iter()
                .filter(|l| !l.is_cost() && l.airport == origin && l.at <= start && start - l.at <= Duration::days(120))
                .max_by_key(|l| l.at)
                .and_then(|l| price_per_kg(std::iter::once(l)))
        });
        if let Some(p) = price {
            out.entry(f.id).or_default().fuel_price_kg = Some(p);
        }
    }
    out
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
    /// (v8.3) Carga transportada (kg) en vuelos de carga; 0 en los de pasaje.
    cargo_kg: f64,
    /// (24/10) Combustible quemado (kg) y precio por kg con que se pagó.
    fuel_kg: f64,
    fuel_price_kg: f64,
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

/// (24/10) Semilla de los estimados de un vuelo: su hora de salida, igual en
/// todas las PCs (antes era el `id`, que es local: el mismo vuelo bajado de la
/// nube a otra PC daba otros pasajeros y otro dinero).
fn flight_seed(e: &FlightLogEntry) -> i64 {
    e.started_at.bytes().fold(0i64, |h, b| h.wrapping_mul(31).wrapping_add(b as i64))
}

/// Pasajeros: el real si se registró, o estimado por capacidad × load factor.
/// El load factor se siembra con el vuelo (`flight_seed`) para variar de forma
/// determinista (mismo vuelo → mismo estimado) sin ser idéntico en todos.
fn estimate_pax(e: &FlightLogEntry, seats: u32, lowcost: bool) -> i64 {
    if let Some(p) = e.passengers {
        if p > 0 {
            return p;
        }
    }
    let base = if lowcost { 0.90 } else { 0.82 };
    let jitter = ((flight_seed(e).rem_euclid(13)) as f64 - 6.0) / 100.0; // ±0.06
    let lf = (base + jitter).clamp(0.60, 0.98);
    ((seats as f64) * lf).round() as i64
}

/// (24/10) Combustible del vuelo: lo QUEMADO (real o estimado por distancia)
/// × el precio por kg de GSX en su aeropuerto de salida (o la referencia).
/// Las cargas de GSX no son gasto del vuelo: antes, si había recibo, el vuelo
/// pagaba sólo lo cargado (a veces 15 USD con 3 t quemadas).
/// Devuelve (USD, kg, USD/kg).
fn fuel_cost(e: &FlightLogEntry, gsx: &GsxCosts, size: Size, d: f64) -> (f64, f64, f64) {
    let fuel_kg = e
        .fuel_used_kg
        .map(|k| k as f64)
        .filter(|k| *k > 0.0)
        .unwrap_or_else(|| d * burn_per_nm(size));
    let price = gsx.fuel_price_kg.unwrap_or(FUEL_USD_PER_KG);
    (fuel_kg * price, fuel_kg, price)
}

/// (24/10) Handling: lo que GSX facturó en cada lado (salida/llegada) o la
/// mitad del estimado en el lado que no tuvo recibo.
fn handling_cost(gsx: &GsxCosts, estimate: f64) -> f64 {
    gsx.handling_dep.unwrap_or(estimate / 2.0) + gsx.handling_arr.unwrap_or(estimate / 2.0)
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
            let lf = (0.78 + ((flight_seed(e).rem_euclid(11)) as f64 - 5.0) / 100.0).clamp(0.55, 0.95);
            freighter_payload(size) * lf
        });
    // Tarifa de flete por kg para todo el trayecto (sube con la distancia).
    let revenue_tickets = payload * (0.80 + 0.0006 * d);
    let revenue_ancillary = payload * 0.05; // express/prioridad/seguro

    let (cost_fuel, fuel_kg, fuel_price_kg) = fuel_cost(e, gsx, size, d);
    // No hay catering en carga (salvo que GSX lo haya facturado).
    let cost_catering = gsx.catering.unwrap_or(0.0);
    let handling_est = match size {
        Size::Wide => 2500.0,
        Size::Narrow => 900.0,
        Size::Regional => 500.0,
    };
    let cost_handling = handling_cost(gsx, handling_est);
    let cost_maintenance = block_hours(e, d) * maint_rate(size) * policy.maintenance_mult();
    let cost_fees = landing_fee(size) + d * 1.2 + payload * 0.012; // landing+nav+aduana

    let revenue = revenue_tickets + revenue_ancillary;
    let costs = cost_fuel + cost_catering + cost_handling + cost_maintenance + cost_fees;
    Pnl {
        // (v8.3) A bordo de un carguero sólo van los jumpseats (máx. 3) que
        // traiga el OFP; no pagan billete.
        pax: e.passengers.unwrap_or(0).clamp(0, JUMPSEATS),
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
        cargo_kg: payload,
        fuel_kg,
        fuel_price_kg,
    }
}

/// Calcula el P&L de un vuelo. `cargo` = vuelo de carga (`is_cargo_flight`).
fn compute_pnl(e: &FlightLogEntry, res: &Resolved, gsx: &GsxCosts, policy: &AirlinePolicy, cargo: bool) -> Pnl {
    let (size, seats) = classify(e.aircraft_model.as_deref().or(e.aircraft_atc_type.as_deref()));
    let d = distance_nm(e, size);

    if cargo {
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
    let (cost_fuel, fuel_kg, fuel_price_kg) = fuel_cost(e, gsx, size, d);
    let cat_est = if res.lowcost { paxf * 2.0 } else { paxf * (4.0 + 0.0015 * d) };
    let cost_catering = gsx.catering.unwrap_or(cat_est) + svc_cost;
    let handling_est = match size {
        Size::Wide => 1100.0,
        Size::Narrow => 480.0,
        Size::Regional => 280.0,
    };
    let cost_handling = handling_cost(gsx, handling_est);
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
        cargo_kg: 0.0,
        fuel_kg,
        fuel_price_kg,
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
    /// (v8.3) Carga de tus vuelos (kg) y pasajeros/carga estimados de la
    /// operación autónoma.
    cargo_kg: f64,
    auto_pax: i64,
    auto_cargo_kg: f64,
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
    /// (v8.3) Pasajeros y carga (kg) estimados de los vuelos reales.
    pax: i64,
    cargo_kg: f64,
    /// (v8.3) Por hora UTC del día: (vuelos, neto sin costos fijos). Para
    /// que la curva de saldo se mueva con los vuelos, no una vez al día.
    hourly: Vec<(i64, f64)>,
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
    let cargo_flags = airline_cargo_flags(flights, learned);
    let mut profiles: HashMap<String, Resolved> = HashMap::new();
    let mut user_regs: HashMap<String, HashMap<String, Option<String>>> = HashMap::new();
    for f in flights {
        let Some(mut res) = resolve_airline(f, learned) else { continue };
        res.cargo = cargo_flags.get(&res.key).copied().unwrap_or(res.cargo);
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
    // (6.2d) Aviones del usuario de TechLog/pinturas cuyo operador (base de
    // aviones) es una aerolínea de Finanzas: también pagan costo fijo en ella.
    for (icao, aircraft) in crate::my_fleet::mine_by_operator(pool).await.unwrap_or_default() {
        if let Some(res) = profiles.get(&icao) {
            let regs = user_regs.entry(res.key.clone()).or_default();
            for (reg, a) in aircraft {
                regs.entry(reg).or_insert(a.model);
            }
        }
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
        // aerolínea → icao24 → (tipo, matrícula)
        type Fleet = HashMap<String, HashMap<String, (Option<String>, Option<String>)>>;
        let mut fleet: Fleet = HashMap::new();
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
                    a.typecode AS tc, a.registration AS reg, a.model AS mdl
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
                // (v8.3) Modelo de la base de aviones ("777-FB5", "747-8HVF"):
                // decide si es un carguero.
                aircraft_title: r.get("mdl"),
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
            let cargo = is_cargo_flight(&entry, res.cargo);
            let pnl = compute_pnl(&entry, res, &GsxCosts::default(), &policy, cargo);
            let t = acc.entry(res.key.clone()).or_default();
            t.flights += 1;
            t.revenue_tickets += pnl.revenue_tickets;
            t.revenue_ancillary += pnl.revenue_ancillary;
            t.cost_fuel += pnl.cost_fuel;
            t.cost_catering += pnl.cost_catering;
            t.cost_handling += pnl.cost_handling;
            t.cost_maintenance += pnl.cost_maintenance;
            t.cost_fees += pnl.cost_fees;
            // Pasajeros estimados (los cargueros reales no llevan) y carga.
            t.pax += if cargo { 0 } else { pnl.pax };
            t.cargo_kg += pnl.cargo_kg;
            if t.hourly.is_empty() {
                t.hourly = vec![(0, 0.0); 24];
            }
            let h = ((fs - day_start).div_euclid(3_600)).clamp(0, 23) as usize;
            t.hourly[h].0 += 1;
            t.hourly[h].1 += pnl.net;
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
                 cost_catering, cost_handling, cost_maintenance, cost_fees, cost_fixed, fleet, computed_at,
                 cargo_kg, hourly)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(key)
        .bind(day)
        .bind(t.flights)
        .bind(t.pax)
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
        .bind(t.cargo_kg)
        .bind(serde_json::to_string(&t.hourly).unwrap_or_else(|_| "[]".into()))
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
                SUM(pax) AS pax, SUM(cargo_kg) AS kg,
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
                    pax: r.get("pax"),
                    cargo_kg: r.get("kg"),
                    hourly: Vec::new(),
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
    let gsx_lines = all_gsx_lines(pool).await;
    let flights = flight_log::list_entries(pool).await?;
    // (24/10) Cada línea de GSX → como mucho UN vuelo (completado), sin los
    // servicios repetidos de intentos abortados.
    let mut gsx_assigned = assign_lines(&gsx_lines, &flights);
    drop_repeats(&gsx_lines, &mut gsx_assigned);
    let mut gsx_costs = gsx_costs_by_flight(&gsx_lines, &gsx_assigned, &flights);
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

    // (v8.3) Aerolínea de carga sólo si es conocida como tal o si TODOS sus
    // vuelos son de carga; cada vuelo se valora como carga o pasaje por sí.
    let cargo_flags = airline_cargo_flags(&flights, &learned);

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM flight_pnl").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM airline_economy")
        .execute(&mut *tx)
        .await?;
    // (24/10) Líneas de GSX con su vuelo (o sin vuelo): trazabilidad de cada
    // cobro y base de la tarjeta del aeropuerto.
    sqlx::query("DELETE FROM gsx_charge").execute(&mut *tx).await?;
    for (l, fid) in gsx_lines.iter().zip(&gsx_assigned) {
        sqlx::query(
            "INSERT OR REPLACE INTO gsx_charge
                (receipt_id, line, kind, service, side, airport, registration, at, amount_usd, uplift_kg, is_cost, flight_id)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&l.receipt_id)
        .bind(l.line as i64)
        .bind(l.kind.id())
        .bind(&l.service)
        .bind(if l.side == Side::Dep { "dep" } else { "arr" })
        .bind(&l.airport)
        .bind(&l.reg)
        .bind(l.at.to_rfc3339())
        .bind(l.usd)
        .bind(l.uplift_kg)
        .bind(l.is_cost() as i64)
        .bind(fid)
        .execute(&mut *tx)
        .await?;
    }

    for f in &flights {
        let Some(mut res) = resolve_airline(f, &learned) else {
            continue;
        };
        res.cargo = cargo_flags.get(&res.key).copied().unwrap_or(res.cargo);
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
        let gsx = gsx_costs.remove(&f.id).unwrap_or_default();
        // Pasajeros y carga del OFP de SimBrief (registrados en el vuelo);
        // sólo se estiman si faltan.
        let pnl = compute_pnl(f, &res, &gsx, &policy, is_cargo_flight(f, res.cargo));

        sqlx::query(
            r#"INSERT OR REPLACE INTO flight_pnl
                (flight_id, airline_key, pax, revenue_tickets, revenue_ancillary,
                 cost_fuel, cost_catering, cost_handling, cost_maintenance,
                 cost_fees, net, gsx_matched, created_at, fuel_kg, fuel_price_kg)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
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
        .bind(pnl.fuel_kg)
        .bind(pnl.fuel_price_kg)
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
            cargo_kg: 0.0,
            auto_pax: 0,
            auto_cargo_kg: 0.0,
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
        acc.cargo_kg += pnl.cargo_kg;
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
        acc.auto_pax = a.pax;
        acc.auto_cargo_kg = a.cargo_kg;
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
                 cargo_kg, auto_pax, auto_cargo_kg, updated_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
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
        .bind(acc.cargo_kg)
        .bind(acc.auto_pax)
        .bind(acc.auto_cargo_kg)
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
    /// (v8.3) Carga de tus vuelos (kg, del OFP o estimada) y pasajeros y carga
    /// estimados de la operación autónoma.
    pub cargo_kg: f64,
    pub auto_pax: i64,
    pub auto_cargo_kg: f64,
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
                  auto_costs, cost_fixed, auto_fleet, cargo_kg, auto_pax, auto_cargo_kg
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
            cargo_kg: r.get("cargo_kg"),
            auto_pax: r.get("auto_pax"),
            auto_cargo_kg: r.get("auto_cargo_kg"),
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
    /// (v8.3) Saldo sin las compras de aviones: la curva se dibuja con este
    /// valor para que cada vuelo se note (las compras van como marcas).
    pub operating: f64,
    /// Variación del saldo en este evento.
    pub delta: f64,
    /// Etiqueta legible (ruta, hora de operación, compra o mantenimiento).
    pub label: String,
    /// "start" | "flight" | "purchase" | "maint" | "ferry" | "auto".
    pub kind: String,
}

/// (v8.3) La operación real se dibuja por hora los últimos días; antes, por día.
const HOURLY_DAYS: i64 = 30;

/// Serie temporal del saldo de una aerolínea: parte del valor base y suma, en
/// orden, el neto de cada vuelo tuyo, la operación real por hora (con su parte
/// de los costos fijos), traslados, mantenimiento y la compra de cada avión la
/// primera vez que aparece (como evento propio). El último punto coincide con
/// el `balance` de `list_economy`.
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
    let mut events: Vec<Ev> = Vec::with_capacity(rows.len() * 2);
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

        // Compra del avión la primera vez que vuela con la aerolínea: evento
        // propio (antes del vuelo; el orden es estable a igual hora).
        if let Some(rr) = reg_raw.as_deref() {
            let nreg = normalize_reg(rr);
            if !nreg.is_empty() {
                airline_regs.insert(nreg.clone());
                if seen_regs.insert(nreg) {
                    let (size, _) = classify(model.as_deref().or(atc.as_deref()));
                    events.push(Ev {
                        t: t.clone(),
                        delta: -purchase_price(size),
                        label: crate::tr!("economy.purchase", reg = rr.trim().to_ascii_uppercase()),
                        kind: "purchase",
                    });
                }
            }
        }
        let route = match (origin.as_deref(), dest.as_deref()) {
            (Some(o), Some(d)) => format!("{o} → {d}"),
            (Some(o), None) => o.to_string(),
            (None, Some(d)) => d.to_string(),
            _ => crate::tr!("economy.flight"),
        };
        events.push(Ev {
            t,
            delta: net,
            label: route,
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

    // (v8.3) Operación real: por hora (vuelos de esa hora menos 1/24 de los
    // costos fijos del día) los últimos 30 días; antes, un punto por día.
    let hourly_since = (Utc::now() - Duration::days(HOURLY_DAYS)).format("%Y-%m-%d").to_string();
    for r in sqlx::query(
        "SELECT day, flights, hourly, cost_fixed,
                revenue_tickets + revenue_ancillary AS rev,
                cost_fuel + cost_catering + cost_handling + cost_maintenance + cost_fees AS cost
           FROM auto_daily WHERE airline_key = ? ORDER BY day",
    )
    .bind(key)
    .fetch_all(pool)
    .await?
    {
        let day: String = r.get("day");
        let rev: f64 = r.get("rev");
        let cost: f64 = r.get("cost");
        let fixed: f64 = r.get("cost_fixed");
        let label_day = day.get(5..).map(|md| format!("{}/{}", &md[3..], &md[..2])).unwrap_or_else(|| day.clone());
        let hourly: Vec<(i64, f64)> = r
            .get::<Option<String>, _>("hourly")
            .and_then(|h| serde_json::from_str(&h).ok())
            .unwrap_or_default();
        if day >= hourly_since && hourly.len() == 24 {
            for (h, (n, net)) in hourly.iter().enumerate() {
                events.push(Ev {
                    t: format!("{day}T{h:02}:59:59Z"),
                    delta: net - fixed / 24.0,
                    label: crate::tr!("economy.autoHour", date = label_day, hour = format!("{h:02}"), n = n),
                    kind: "auto",
                });
            }
        } else {
            events.push(Ev {
                t: format!("{day}T23:59:59Z"),
                delta: rev - cost - fixed,
                label: crate::tr!("economy.autoDay", date = label_day, n = r.get::<i64, _>("flights")),
                kind: "auto",
            });
        }
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
    let mut op = valuation;
    if let Some(first) = events.first() {
        out.push(BalancePoint {
            t: first.t.clone(),
            balance: bal,
            operating: op,
            delta: 0.0,
            label: crate::tr!("economy.start"),
            kind: "start".to_string(),
        });
    }
    for ev in events {
        bal += ev.delta;
        if ev.kind != "purchase" {
            op += ev.delta;
        }
        out.push(BalancePoint {
            t: ev.t,
            balance: bal,
            operating: op,
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

        // (v8.3) Pasajeros estimados y neto por hora UTC (24 horas).
        assert!(jbu.pax > 300, "3 vuelos de A320: {}", jbu.pax);
        assert_eq!(jbu.cargo_kg, 0.0);
        let hourly: String = sqlx::query_scalar("SELECT hourly FROM auto_daily WHERE airline_key = 'JBU'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let hourly: Vec<(i64, f64)> = serde_json::from_str(&hourly).unwrap();
        assert_eq!(hourly.len(), 24);
        assert_eq!(hourly.iter().map(|h| h.0).sum::<i64>(), 3);
        assert_eq!((hourly[1].0, hourly[5].0, hourly[8].0), (1, 1, 1), "01:00, 05:33 y 08:20 UTC");
    }

    fn flight(model: &str, title: &str, pax: Option<i64>, cargo: Option<i64>) -> FlightLogEntry {
        FlightLogEntry {
            aircraft_model: Some(model.into()),
            aircraft_title: Some(title.into()),
            passengers: pax,
            cargo_kg: cargo,
            ..Default::default()
        }
    }

    #[test]
    fn freighters_are_detected_by_model() {
        for f in ["777F", "PMDG 777F China Cargo", "747-8F", "767-300ERF", "A330-200F", "MD-11F",
                  "737-800BCF", "737-800SF", "A321P2F", "777-FB5", "747-8HVF", "767-3S2F", "Boeing 757 Freighter"] {
            assert!(is_freighter(Some(f)), "{f}");
        }
        for p in ["FenixA320 IAE WF", "A20N", "737-8 MAX", "B38M", "737-7CT", "Boeing 737-8K5", "A321neo",
                  "777-3F2ER", "737-8F2", "iFly 737-MAX8", "B789"] {
            assert!(!is_freighter(Some(p)), "{p}");
        }
    }

    /// Un carguero con 4 personas en el OFP cuenta 3 (jumpseats) y su carga;
    /// un vuelo de pasaje con carga en bodega sigue siendo de pasaje.
    #[test]
    fn cargo_flights_carry_at_most_three_jumpseats() {
        let res = Resolved {
            key: "DHL".into(),
            icao: Some("DHL".into()),
            name: "DHL".into(),
            valuation: 1e9,
            lowcost: false,
            cargo: false,
        };
        let policy = AirlinePolicy::default();
        let dhl = flight("PMDG 777F DHL UK G", "", Some(4), Some(26_200));
        assert!(is_cargo_flight(&dhl, false));
        let p = compute_pnl(&dhl, &res, &GsxCosts::default(), &policy, true);
        assert_eq!((p.pax, p.cargo_kg), (3, 26_200.0));
        let kal = flight("Airbus A321Neo", "Korean Air", Some(160), Some(4_000));
        assert!(!is_cargo_flight(&kal, false), "pasaje con carga en bodega");
        let p = compute_pnl(&kal, &res, &GsxCosts::default(), &policy, false);
        assert_eq!((p.pax, p.cargo_kg), (160, 0.0));
        // Carga sin pasajeros en el OFP → carga aunque el modelo no lo diga.
        assert!(is_cargo_flight(&flight("777", "", Some(0), Some(50_000)), false));
    }

    /// Korean Air con un 777F y un A321neo queda MIXTA (no "de carga");
    /// China Cargo, con todos sus vuelos de carga, sí lo es.
    #[test]
    fn mixed_airlines_are_not_flagged_as_cargo() {
        let mk = |icao: &str, model: &str, pax: i64, cargo: i64| FlightLogEntry {
            airline_icao: Some(icao.into()),
            aircraft_model: Some(model.into()),
            passengers: Some(pax),
            cargo_kg: Some(cargo),
            ..Default::default()
        };
        let flights = vec![
            mk("KAL", "777F", 0, 46_873),
            mk("KAL", "Airbus A321Neo", 160, 4_000),
            mk("CKK", "PMDG 777F China Cargo", 0, 72_548),
        ];
        let flags = airline_cargo_flags(&flights, &HashMap::new());
        assert_eq!(flags.get("KAL"), Some(&false));
        assert_eq!(flags.get("CKK"), Some(&true));
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

    /// Recibo real de CYWG (13/10): desembarque del vuelo que llega + embarque
    /// y pushback del que sale + jetway, con impuestos.
    const TURNAROUND: &str = r#"{"receiptId":"GSX-20261013T034021Z-CYWG","items":[
        {"description":"Passenger deboarding","qty":"x","unitPrice":"x","amount":"C$879.70 ~$ 617.55"},
        {"description":"Passenger boarding","qty":"x","unitPrice":"x","amount":"C$2,282.82 ~$ 1,602.54"},
        {"description":"Pushback service\n(82 t MTOW)","qty":"x","unitPrice":"x","amount":"C$1,170.81 ~$ 821.91"},
        {"description":"Jetway connection fee","qty":"x","unitPrice":"x","amount":"C$142.45 ~$ 100.00"},
        {"description":"Jetway operation time","qty":"x","unitPrice":"x","amount":"C$84.16 ~$ 59.08"}],
        "subtotal":"C$4,559.94 ~$ 3,201.08","total":"C$4,787.94 ~$ 3,361.14"}"#;

    #[test]
    fn receipt_lines_split_by_side_with_their_taxes() {
        let lines = parse_receipt_lines(TURNAROUND, GsxKind::Handling);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0].side, Side::Arr, "desembarque = vuelo que llega");
        assert!(lines[1..].iter().all(|l| l.side == Side::Dep), "embarque/pushback/jetway = el que sale");
        assert_eq!(lines[2].service, "Pushback service (82 t MTOW)");
        // Con impuestos repartidos, las líneas suman el total del recibo.
        let sum: f64 = lines.iter().map(|l| l.usd).sum();
        assert!((sum - 3361.14).abs() < 0.06, "suma {sum}");
        // Sólo llegada: el jetway va con la llegada.
        let arr = r#"{"items":[{"description":"Passenger deboarding","amount":"$ 1,066.50"},
            {"description":"Jetway connection fee","amount":"$ 100.00"}],"subtotal":"$ 1,166.50","total":"$ 1,166.50"}"#;
        assert!(parse_receipt_lines(arr, GsxKind::Handling).iter().all(|l| l.side == Side::Arr));
        // Líneas de 0 USD no cuentan.
        let zero = r#"{"items":[{"description":"Jetway connection fee","amount":"C$0.00 ~$ 0.00"}],"total":"C$0.00 ~$ 0.00"}"#;
        assert!(parse_receipt_lines(zero, GsxKind::Handling).is_empty());
    }

    #[test]
    fn fuel_uplift_gives_a_price_and_defuel_is_a_service() {
        let up = r#"{"items":[{"description":"Jet-A1 fuel","qty":"3 gal 20 lb 9 kg","amount":"C$21.00 ~$ 14.80"}],
            "subtotal":"C$21.00 ~$ 14.80","total":"C$22.05 ~$ 15.54"}"#;
        let l = parse_receipt_lines(up, GsxKind::Fuel);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].uplift_kg, Some(9.0));
        assert!((l[0].usd - 15.54).abs() < 0.01);
        let de = r#"{"items":[{"description":"Jet-A1 fuel (defueling)","qty":"603 gal 4,043 lb 1,834 kg","amount":"€970.53 ~$ 1,090.69"}],
            "subtotal":"€970.53 ~$ 1,090.69","total":"€1,218.01 ~$ 1,368.82"}"#;
        let l = parse_receipt_lines(de, GsxKind::Fuel);
        assert_eq!(l[0].uplift_kg, None, "desabastecer es un servicio cobrado");
        assert_eq!(l[0].side, Side::Dep);
        assert!((l[0].usd - 1368.82).abs() < 0.01);
    }

    fn flight(id: i64, from: &str, to: &str, start: &str, end: &str) -> FlightLogEntry {
        FlightLogEntry {
            id,
            aircraft_registration: Some("C-GORP".into()),
            origin_icao: Some(from.into()),
            destination_icao: Some(to.into()),
            started_at: start.into(),
            ended_at: Some(end.into()),
            ..Default::default()
        }
    }

    fn line(i: usize, kind: GsxKind, side: Side, airport: &str, at: &str, usd: f64, kg: Option<f64>) -> GsxLine {
        GsxLine {
            receipt_id: "R".into(),
            line: i,
            kind,
            service: format!("s{i}"),
            side,
            airport: airport.into(),
            reg: "CGORP".into(),
            at: parse_dt(at).unwrap(),
            usd,
            uplift_kg: kg,
        }
    }

    #[test]
    fn each_line_goes_to_one_flight_only() {
        // Llega a CYWG 02:38, sale de CYWG 03:29; el recibo se cierra 03:40.
        let flights = [
            flight(45, "CYYC", "CYWG", "2026-10-13T00:41:27Z", "2026-10-13T02:38:36Z"),
            flight(46, "CYWG", "CYYC", "2026-10-13T03:29:05Z", "2026-10-13T05:32:19Z"),
        ];
        let at = "2026-10-13T03:40:21Z";
        let lines = [
            line(0, GsxKind::Handling, Side::Arr, "CYWG", at, 617.55, None),
            line(1, GsxKind::Handling, Side::Dep, "CYWG", at, 1602.54, None),
            line(2, GsxKind::Handling, Side::Dep, "CYWG", at, 821.91, None),
        ];
        let a = assign_lines(&lines, &flights);
        assert_eq!(a, vec![Some(45), Some(46), Some(46)]);
        let costs = gsx_costs_by_flight(&lines, &a, &flights);
        assert_eq!(costs[&45].handling_arr, Some(617.55));
        assert_eq!(costs[&45].handling_dep, None);
        assert!((costs[&46].handling_dep.unwrap() - 2424.45).abs() < 0.01);
        // Ningún dinero se cuenta dos veces.
        let total_charged: f64 = costs.values().map(|c| c.handling_arr.unwrap_or(0.0) + c.handling_dep.unwrap_or(0.0)).sum();
        assert!((total_charged - (617.55 + 1602.54 + 821.91)).abs() < 0.01);
    }

    #[test]
    fn repeated_services_count_once() {
        // #32 real: catering 15:05, 15:46, 16:59 y embarque 15:11, 17:36 antes
        // de salir a las 17:23 → cuentan los últimos.
        let flights = [flight(32, "CYYC", "KLAS", "2026-10-09T17:23:33Z", "2026-10-09T20:14:39Z")];
        let mut lines = vec![
            line(0, GsxKind::Catering, Side::Dep, "CYYC", "2026-10-09T15:05:39Z", 328.86, None),
            line(1, GsxKind::Handling, Side::Dep, "CYYC", "2026-10-09T15:11:54Z", 1459.78, None),
            line(2, GsxKind::Catering, Side::Dep, "CYYC", "2026-10-09T15:46:46Z", 328.86, None),
            line(3, GsxKind::Catering, Side::Dep, "CYYC", "2026-10-09T16:59:02Z", 328.86, None),
            line(4, GsxKind::Handling, Side::Dep, "CYYC", "2026-10-09T17:36:05Z", 1732.0, None),
        ];
        for (i, l) in lines.iter_mut().enumerate() {
            l.receipt_id = format!("R{i}");
            if l.kind == GsxKind::Handling {
                l.service = "Passenger boarding".into();
            }
        }
        let mut a = assign_lines(&lines, &flights);
        assert!(a.iter().all(|x| *x == Some(32)));
        drop_repeats(&lines, &mut a);
        assert_eq!(a, vec![None, None, None, Some(32), Some(32)]);
        let c = &gsx_costs_by_flight(&lines, &a, &flights)[&32];
        assert_eq!(c.catering, Some(328.86));
        assert_eq!(c.handling_dep, Some(1732.0));
    }

    #[test]
    fn lines_without_a_flight_are_not_charged() {
        // Vuelo incompleto/borrado: sus servicios no caen en otro vuelo.
        let flights = [flight(1, "CYYC", "CYEG", "2026-10-23T03:42:52Z", "2026-10-23T04:33:30Z")];
        let lines = [
            line(0, GsxKind::Catering, Side::Dep, "EFHK", "2026-10-23T19:08:03Z", 761.85, None),
            line(1, GsxKind::Handling, Side::Dep, "CYYC", "2026-10-23T16:33:04Z", 50.0, None),
        ];
        assert_eq!(assign_lines(&lines, &flights), vec![None, None]);
    }

    #[test]
    fn fuel_is_burned_kg_times_the_local_gsx_price() {
        let mut f = flight(7, "CYYZ", "CYYC", "2026-10-17T03:00:00Z", "2026-10-17T07:00:00Z");
        f.fuel_used_kg = Some(9000);
        // Carga de GSX en CYYZ ese día: 9 kg por 15.54 USD → 1.7267 USD/kg.
        let lines = [line(0, GsxKind::Fuel, Side::Dep, "CYYZ", "2026-10-17T02:19:33Z", 15.54, Some(9.0))];
        let a = assign_lines(&lines, std::slice::from_ref(&f));
        assert_eq!(a, vec![Some(7)]);
        let costs = gsx_costs_by_flight(&lines, &a, std::slice::from_ref(&f));
        let c = &costs[&7];
        assert!(!c.matched(), "una carga de combustible no es un cobro");
        let (usd, kg, price) = fuel_cost(&f, c, Size::Narrow, 1500.0);
        assert_eq!(kg, 9000.0);
        assert!((price - 15.54 / 9.0).abs() < 1e-9);
        assert!((usd - 9000.0 * 15.54 / 9.0).abs() < 0.01, "paga lo quemado, no los 15 USD cargados");
        // Sin recibo: precio de referencia.
        let (usd, _, price) = fuel_cost(&f, &GsxCosts::default(), Size::Narrow, 1500.0);
        assert_eq!(price, FUEL_USD_PER_KG);
        assert!((usd - 9000.0 * FUEL_USD_PER_KG).abs() < 0.01);
    }

    /// Contra una COPIA de la base real y los recibos reales de GSX:
    /// `ECON_DB=<copia> cargo test --lib gsx_real_copy -- --ignored --nocapture`.
    /// Migra la copia, recalcula y compara el P&L de cada vuelo antes/después.
    #[tokio::test]
    #[ignore]
    async fn gsx_real_copy() {
        let path = std::env::var("ECON_DB").expect("ECON_DB");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{path}"))
            .await
            .unwrap();
        let old: HashMap<i64, (f64, f64, f64, f64)> =
            sqlx::query_as::<_, (i64, f64, f64, f64, f64)>("SELECT flight_id, cost_fuel, cost_catering, cost_handling, net FROM flight_pnl")
                .fetch_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .map(|(id, a, b, c, d)| (id, (a, b, c, d)))
                .collect();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        recompute(&pool).await.unwrap();
        type Row = (i64, String, f64, f64, f64, f64, Option<f64>, Option<f64>);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT p.flight_id, COALESCE(f.origin_icao,'?') || '→' || COALESCE(f.destination_icao,'?') || ' ' || COALESCE(f.aircraft_registration,''),
                    p.cost_fuel, p.cost_catering, p.cost_handling, p.net, p.fuel_kg, p.fuel_price_kg
             FROM flight_pnl p JOIN flight_log f ON f.id = p.flight_id ORDER BY f.started_at DESC LIMIT 14",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for (id, route, fuel, cat, hand, net, kg, price) in rows {
            let o = old.get(&id).copied().unwrap_or_default();
            let lines: Vec<(String, String, f64)> =
                sqlx::query_as("SELECT side, service, amount_usd FROM gsx_charge WHERE flight_id = ?1 AND is_cost = 1 ORDER BY at")
                    .bind(id)
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            eprintln!(
                "#{id} {route}\n   antes: fuel {:.0} catering {:.0} handling {:.0} neto {:.0}\n   ahora: fuel {fuel:.0} ({:.0} kg × {:.3}) catering {cat:.0} handling {hand:.0} neto {net:.0}",
                o.0, o.1, o.2, o.3, kg.unwrap_or(0.0), price.unwrap_or(0.0)
            );
            for (side, svc, usd) in lines {
                eprintln!("     GSX {side} {svc}: {usd:.2}");
            }
        }
        let (n, charged, free): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), SUM(flight_id IS NOT NULL AND is_cost = 1), SUM(flight_id IS NULL AND is_cost = 1) FROM gsx_charge",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        eprintln!("líneas GSX: {n} · cobradas a un vuelo: {charged} · sin vuelo (no se cobran): {free}");
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
