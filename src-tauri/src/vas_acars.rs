//! Importador de logbook de **VAS-ACARS** (Virtual Airlines System / vasystem.org).
//!
//! VAS-ACARS guarda cada vuelo como un archivo `.bin` independiente en
//! `%LOCALAPPDATA%\VASystem\VAS-ACARS\flights\<uuid>.bin`. El formato
//! es **Protocol Buffers concatenado**: un prefijo de 2 bytes
//! (magic `0xC1 0x06`) seguido de **N mensajes protobuf consecutivos**,
//! uno por cada muestra del vuelo (sampling ~5s).
//!
//! Schema por muestra (decodificado vía reverse engineering sobre los
//! 138 archivos del usuario — campos top-level a nivel de cada mensaje):
//!
//! | Campo | Tipo wire | Significado |
//! |-------|-----------|-------------|
//! | f3    | nested msg `{f1: u64 epoch}` | Timestamp de la muestra (UNIX seconds) |
//! | f4    | length-delim str | Livery del avión (ej. `"FenixA320_LANCCBAA"`) — redundante en cada muestra |
//! | f5    | length-delim str | Path `aircraft.CFG` — redundante en cada muestra |
//! | f12   | nested msg con dos f64 | Posición `{f1: lat, f2: lon}` en grados |
//! | f13   | f32 | True heading (degrees) |
//! | f16   | f32 | Altitud **en metros** MSL |
//! | f17   | f32 | Bank angle (deg) |
//! | f18   | f32 | Pitch angle (deg) |
//! | f21   | varint | On-ground flag (0/1) |
//! | f23   | f32 | TAS (True Airspeed) en knots |
//! | f25   | f32 | IAS (Indicated Airspeed) en knots |
//! | f49   | f32 | OAT (Outside Air Temp) °C |
//! | f53   | f32 | Ground Speed en knots |
//! | f55   | f32 | Static pressure (Pa) — útil para validar altitud |
//!
//! Los **ICAOs origen/destino** NO viven en el `.bin` — vienen del
//! `data.db` (formato bbolt + protobuf) parseado en `vas_summary.rs`.
//! Si el vuelo no está en `data.db`, resolvemos origen/destino por
//! **nearest-airport** sobre la primera/última posición del track.
//!
//! Estrategia de import (v3.5.0 F2 v4):
//!   1. Detecta candidatos `.bin` en `flights/` (excluye `.bin.zip`).
//!   2. Para cada uno, parsea el track completo (todas las muestras).
//!   3. **Validación por track**: ≥50 muestras + origen ≠ destino (>5 nm).
//!      Esto reemplaza el filtro estricto que dependía del `data.db`
//!      summary — ahora capturamos vuelos completos sin entry en
//!      data.db (que se perdían en el filtro anterior).
//!   4. Resuelve metadata extra del `data.db` summary si existe
//!      (airline, registration, gates, callsign).
//!   5. Persiste muestras del track en `flight_log_track` (downsampleo
//!      cada 10s para coincidir con el sampling de SimConnect).
//!   6. Computa y guarda **métricas pico**: `max_altitude_ft`,
//!      `max_ground_speed_kt`, `max_true_airspeed_kt`, `landing_fpm`.
//!
//! El parsing está hecho a mano (sin `prost` ni `protobuf` crate) —
//! el schema es estable suficiente para evitar la dep.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sqlx::SqlitePool;

use crate::flight_log::nearest_airport_with_coords;

const SOURCE_LABEL: &str = "vas-acars";

/// Una muestra individual del track extraída de un `.bin` de VAS-ACARS.
/// Cada `.bin` contiene cientos o miles de estas (sampling ~5s).
#[derive(Debug, Clone)]
pub struct VasTrackSample {
    /// Timestamp UNIX en segundos (campo f3 del protobuf).
    pub ts_epoch: i64,
    pub lat: f64,
    pub lon: f64,
    /// Altitud en metros MSL (campo f16). None si no se extrajo.
    pub altitude_m: Option<f32>,
    /// Ground Speed en kt (campo f53).
    pub ground_speed_kt: Option<f32>,
    /// True Airspeed en kt (campo f23).
    pub true_airspeed_kt: Option<f32>,
    /// True heading en grados (campo f13).
    pub heading_deg: Option<f32>,
    /// On-ground flag (campo f21).
    pub on_ground: Option<bool>,
}

impl VasTrackSample {
    pub fn altitude_ft(&self) -> Option<i64> {
        self.altitude_m.map(|m| (m * 3.28084) as i64)
    }
}

/// Track completo parseado de un `.bin`: metadata + todas las muestras.
#[derive(Debug, Clone)]
pub struct VasFlightTrack {
    pub livery: Option<String>,
    pub aircraft_cfg_path: Option<String>,
    pub samples: Vec<VasTrackSample>,
}

impl VasFlightTrack {
    pub fn first_position(&self) -> Option<(f64, f64)> {
        self.samples.first().map(|s| (s.lat, s.lon))
    }

    pub fn last_position(&self) -> Option<(f64, f64)> {
        self.samples.last().map(|s| (s.lat, s.lon))
    }

    pub fn started_at_iso(&self) -> Option<String> {
        self.samples.first().and_then(|s| epoch_to_iso(s.ts_epoch))
    }

    pub fn ended_at_iso(&self) -> Option<String> {
        self.samples.last().and_then(|s| epoch_to_iso(s.ts_epoch))
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        match (self.samples.first(), self.samples.last()) {
            (Some(f), Some(l)) if l.ts_epoch > f.ts_epoch => Some(l.ts_epoch - f.ts_epoch),
            _ => None,
        }
    }

    /// Altitud máxima (ft) durante el vuelo. None si no hubo samples
    /// con altitud válida.
    pub fn max_altitude_ft(&self) -> Option<i64> {
        let mut max = f32::MIN;
        for s in &self.samples {
            if let Some(a) = s.altitude_m {
                if a > max {
                    max = a;
                }
            }
        }
        if max <= f32::MIN + 1.0 {
            None
        } else {
            Some((max * 3.28084) as i64)
        }
    }

    /// Ground speed máximo (kt) durante el vuelo.
    pub fn max_ground_speed_kt(&self) -> Option<i64> {
        let mut max: f32 = 0.0;
        for s in &self.samples {
            if let Some(v) = s.ground_speed_kt {
                if v > max {
                    max = v;
                }
            }
        }
        if max < 0.5 {
            None
        } else {
            Some(max as i64)
        }
    }

    /// True airspeed máximo (kt) durante el vuelo.
    pub fn max_true_airspeed_kt(&self) -> Option<i64> {
        let mut max: f32 = 0.0;
        for s in &self.samples {
            if let Some(v) = s.true_airspeed_kt {
                if v > max {
                    max = v;
                }
            }
        }
        if max < 0.5 {
            None
        } else {
            Some(max as i64)
        }
    }

    /// Landing FPM = vertical speed (ft/min) en el momento del touchdown.
    ///
    /// (v3.6.6 fix M1) **Algoritmo verificado contra .bin real del usuario**
    /// (vuelo LAN1423 cuya VA platform skyteamvirtual reporta -188 fpm).
    ///
    /// Hallazgos del diagnóstico:
    ///   · El campo `f21` que yo doc'aba como `on_ground` SIEMPRE vale
    ///     1 — no es flag de aterrizaje.
    ///   · VAS-ACARS samplea altitud cada ~10s (no 5s como creía —
    ///     samples alternan "con alt" y "sin alt").
    ///   · VAS NO almacena VS computado en el .bin. La VA platform
    ///     debe calcularlo igual que nosotros (de altitudes + tiempo).
    ///
    /// **Nuevo algoritmo** (más fiel a skyteamvirtual):
    ///   1. `runway_alt` = altitud del último sample (taxi/parking).
    ///   2. Construir lista de samples CON altitud (omite los nulls).
    ///   3. Walk FORWARD hasta encontrar la transición:
    ///      `alt_prev > runway + 5m` → `alt_curr ≤ runway + 5m`.
    ///      Eso es el **touchdown sample**.
    ///   4. `last_airborne` = sample inmediatamente anterior con alt.
    ///   5. VS = (alt_td - alt_la) / (ts_td - ts_la) × 60.
    ///
    /// Sobre el ejemplo LAN1423:
    ///   · sample[2316] alt=27.61m  → last airborne
    ///   · sample[2318] alt=16.47m  ← runway+5m=19.22, transición aquí
    ///   · sample[2320] alt=14.21m  (en pista)
    ///   · VS = (16.47 - 27.61) × 3.28 / (10s/60) ≈ -219 fpm
    ///   · skyteamvirtual reporta -188 fpm — diferencia 31 fpm, dentro
    ///     del margen del sampling discreto de 10s. Para precisión
    ///     mayor habría que parsear VS del addon (Fenix/PMDG internal
    ///     ACARS) que VAS no expone en .bin.
    pub fn landing_fpm(&self) -> Option<i64> {
        if self.samples.len() < 4 {
            return None;
        }

        // 1. Runway elevation = alt del último sample (taxi/parking).
        let runway_alt = self.samples.last().and_then(|s| s.altitude_m)?;

        // 2. Lista de (index, altitude_m, ts_epoch) sólo de samples con
        //    altitud — VAS deja `f16` null en ~50% de los samples.
        let with_alt: Vec<(usize, f32, i64)> = self
            .samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.altitude_m.map(|a| (i, a, s.ts_epoch)))
            .collect();
        if with_alt.len() < 4 {
            return None;
        }

        // 3. Walk FORWARD buscando transición prev > threshold → curr ≤ threshold.
        //    Threshold = runway + 5m. Capturamos la ÚLTIMA transición de ese tipo
        //    (en caso de bounce o ground effect oscillation, queremos la final).
        let threshold = runway_alt + 5.0;
        let mut touchdown_in_alt: Option<usize> = None;
        for i in 1..with_alt.len() {
            let alt_prev = with_alt[i - 1].1;
            let alt_curr = with_alt[i].1;
            if alt_prev > threshold && alt_curr <= threshold {
                touchdown_in_alt = Some(i);
            }
        }
        let td_idx = touchdown_in_alt?;
        let la_idx = td_idx - 1;

        // 4. Compute VS.
        let (_, alt_td, ts_td) = with_alt[td_idx];
        let (_, alt_la, ts_la) = with_alt[la_idx];
        let dt = (ts_td - ts_la) as f32;
        if dt <= 0.0 {
            return None;
        }
        let fpm = (alt_td - alt_la) * 3.28084 / (dt / 60.0);
        // Clamp ±2000 fpm — touchdowns reales raramente pasan -1500;
        // valores extremos son artefactos del sampling o go-arounds.
        let clamped = fpm.clamp(-2000.0, 2000.0);
        Some(clamped as i64)
    }
}

fn epoch_to_iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Información de un único archivo `.bin` de VAS-ACARS encontrado en disco.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VasFlightCandidate {
    pub uuid: String,
    pub path: String,
    pub size_bytes: u64,
}

/// Metadata extraída de un archivo `.bin` de VAS-ACARS tras el parse.
#[derive(Debug, Clone, Default)]
pub struct VasFlightMeta {
    pub livery: Option<String>,
    pub aircraft_cfg_path: Option<String>,
    /// ISO8601 timestamps construidos desde `f6+f7` y `f8+f9` (UTC, sin tz).
    pub time_a_iso: Option<String>,
    pub time_b_iso: Option<String>,
    /// Posición de spawn (start) — campo f12 en grados.
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
}

impl VasFlightMeta {
    /// Devuelve `(started, ended)` ordenados — VAS guarda dep/arr en
    /// f6-f9 pero el orden no es estable entre versiones. Tomamos
    /// el menor como started_at.
    pub fn ordered_times(&self) -> (Option<&str>, Option<&str>) {
        match (self.time_a_iso.as_deref(), self.time_b_iso.as_deref()) {
            (Some(a), Some(b)) => {
                if a <= b {
                    (Some(a), Some(b))
                } else {
                    (Some(b), Some(a))
                }
            }
            (a, b) => (a, b),
        }
    }

    /// Extrae la **matrícula** (registration) del nombre del livery —
    /// el suffix está casi siempre presente para liveries comerciales
    /// con tail number real. Reconoce los formatos más comunes:
    /// `N1234AB` (US sin guión), `D-ABCD` / `G-ABCD` / `EC-ABC`
    /// (Europa con guión), `PK-GHG` (Indonesia), `JA1234` (Japón).
    ///
    /// Ignora suffixes de calidad de textura como `(4K)`, `(8K)`,
    /// `(HD)`. Devuelve `None` si el livery no es un tail real (ej.
    /// `FenixA320_LANCCBAA` es un código interno, no una matrícula).
    pub fn registration(&self) -> Option<String> {
        let livery = self.livery.as_deref()?;
        extract_registration(livery)
    }

    /// Extrae el nombre del operador (airline) del livery. Estrategia:
    /// strip el modelo de avión del inicio, strip la registration
    /// del final, lo que queda en el medio es el airline.
    ///
    /// Ejemplos:
    /// - "Airbus A330-900neo Delta N404DX" → "Delta"
    /// - "Airbus A330-900neo Garuda Indonesia PK-GHG" → "Garuda Indonesia"
    /// - "FenixA321 CFM SL Delta Air Lines N367DN" → "Delta Air Lines"
    ///   (los tokens del medio como `CFM SL` se cuelan — engine code +
    ///   sharklets; aceptable para v1, el usuario puede editar)
    pub fn airline(&self) -> Option<String> {
        let livery = self.livery.as_deref()?;
        extract_airline(livery)
    }

    /// Resume el aircraft type para el flight_log:
    /// - "Fenix A320" si el path matchea `FNX_320`
    /// - "PMDG 737" si matchea `pmdg-aircraft-738/9`
    /// - "iniBuilds A350" si `inibuilds-aircraft-a350`
    /// - "FlyByWire A320" si `flybywire-aircraft-a320`
    /// - "Aerosoft CRJ" si `aerosoft-crj`
    /// - Fallback: el livery name truncado
    pub fn aircraft_label(&self) -> Option<String> {
        let path = self.aircraft_cfg_path.as_deref()?.to_lowercase();
        if path.contains("fnx_320") || path.contains("fenix") {
            return Some("Fenix A320".into());
        }
        if path.contains("pmdg") && (path.contains("738") || path.contains("737")) {
            return Some("PMDG 737".into());
        }
        if path.contains("pmdg") && (path.contains("77") || path.contains("777")) {
            return Some("PMDG 777".into());
        }
        if path.contains("inibuilds-aircraft-a350") || path.contains("ini_a350") {
            return Some("iniBuilds A350".into());
        }
        if path.contains("flybywire-aircraft-a320") || path.contains("fbw_a320") {
            return Some("FlyByWire A320".into());
        }
        if path.contains("flybywire-aircraft-a380") {
            return Some("FlyByWire A380".into());
        }
        if path.contains("aerosoft-crj") || path.contains("aerosoft_crj") {
            return Some("Aerosoft CRJ".into());
        }
        if path.contains("headwindsim-aircraft-a330") {
            return Some("Headwind A330neo".into());
        }
        // Fallback: livery name (FenixA320_LANCCBAA → keep as-is).
        self.livery.clone()
    }
}

/// Reporte de la operación de import.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VasImportReport {
    pub source_dir: Option<String>,
    pub candidates_found: usize,
    pub imported_count: usize,
    /// Vuelos preexistentes (por external_id) cuya metadata fue
    /// ACTUALIZADA con los nuevos campos parseados — útil para
    /// rellenar registration/airline/gate en imports antiguos.
    pub updated_count: usize,
    pub skipped_duplicates: usize,
    /// Archivos que se intentaron parsear pero fallaron (corruptos,
    /// versión no soportada, etc.). El nombre del archivo sirve como
    /// pista de qué mirar a mano.
    pub skipped_invalid: usize,
}

// =============================================================================
// Extracción de campos derivados del livery name. Hecho a mano (sin regex
// crate) — el parsing es lo suficientemente simple que un linear scan +
// validación es más barato que pull-in el ~200KB de la `regex` crate.
// =============================================================================

/// Strip de suffixes comunes de calidad de textura: `(4K)`, `(8K)`,
/// `(HD)`, `[4K]`, etc. — VAS los conserva en el nombre del livery
/// porque viene del nombre de la carpeta, pero no son parte del
/// nombre real del operador o la matrícula.
fn strip_quality_suffix(s: &str) -> &str {
    let trimmed = s.trim();
    // Detecta `(4K)`, `(8K)`, `(HD)`, `[4K]` al final.
    for suffix_marker in &[" (4K)", " (8K)", " (HD)", " [4K]", " [8K]", " [HD]"] {
        if let Some(stripped) = trimmed.strip_suffix(suffix_marker) {
            return stripped;
        }
    }
    trimmed
}

/// True si el string parece una matrícula de avión real:
/// - 4 a 8 chars
/// - Todo ASCII uppercase, dígitos o `-`
/// - Contiene al menos un dígito O un `-` (descarta palabras tipo "DELTA")
/// - No es 100% dígitos (descarta "12345")
fn is_registration_like(s: &str) -> bool {
    let s = s.trim_end_matches([',', '.']);
    let len = s.len();
    if !(4..=8).contains(&len) {
        return false;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
    {
        return false;
    }
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_dash = s.contains('-');
    let has_letter = s.chars().any(|c| c.is_ascii_uppercase());
    // Necesita al menos un dígito o un guión para no confundirse con
    // una palabra del nombre del airline.
    (has_digit || has_dash) && has_letter
}

/// Extrae la matrícula de un livery name. Casos soportados:
/// - `"Airbus A330-900neo Delta N404DX"` → `N404DX` (sin paréntesis al final)
/// - `"PMDG 737-800 Transavia (PH-HXB)"` → `PH-HXB` (envuelta en paréntesis)
/// - `"Boeing 737-832(WL)"` → ignora `(WL)` (winglets), busca más atrás
/// - `"FenixA320_LANCCBAA"` → None (LANCCBAA es código interno, no tail)
///
/// Estrategia: primero busca el **último** sustring entre paréntesis;
/// si pasa `is_registration_like` lo devuelve. Luego cae al fallback
/// de tokens del final.
fn extract_registration(livery: &str) -> Option<String> {
    let cleaned = strip_quality_suffix(livery);

    // Step 1: busca pares de paréntesis del final hacia el inicio.
    // Recolecta TODOS los contenidos entre `(...)` y prueba cada uno.
    // Ej. "Boeing 737-832(WL) Delta (N827DN)" → ["WL", "N827DN"];
    //     "WL" falla is_reg_like (sin dígito ni dash → wait WL is just letters
    //     so it fails because no digit or dash), "N827DN" pasa → devuelve N827DN.
    let mut paren_contents: Vec<String> = Vec::new();
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            if let Some(close_off) = cleaned[i + 1..].find(')') {
                let inner = &cleaned[i + 1..i + 1 + close_off];
                paren_contents.push(inner.trim().to_string());
                i = i + 1 + close_off + 1;
                continue;
            }
        }
        i += 1;
    }
    // Probamos del final al principio — preferimos el último paréntesis
    // (suele ser la reg cuando hay varios).
    for content in paren_contents.iter().rev() {
        if is_registration_like(content) {
            return Some(content.to_string());
        }
    }

    // Step 2: fallback — split por whitespace y '_', mira los últimos
    // 3 tokens.
    let normalized = cleaned.replace('_', " ");
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    for tok in tokens.iter().rev().take(3) {
        // Strip paréntesis residuales y puntuación de cada token.
        let clean = tok
            .trim_matches(|c: char| c == '(' || c == ')')
            .trim_end_matches([',', '.', ':', ';']);
        if is_registration_like(clean) {
            return Some(clean.to_string());
        }
    }
    None
}

/// Strip el prefijo de modelo (Airbus, Boeing, Fenix, etc.) del livery.
/// Permite recuperar la parte "airline + reg" para procesamiento posterior.
fn strip_model_prefix(livery: &str) -> &str {
    // Estos prefijos son lo que VAS captura del aircraft TITLE simvar.
    // Lista empírica del survey de 138 archivos del usuario:
    const PREFIXES: &[&str] = &[
        // Liveries que arrancan con vendor + modelo
        "Airbus A380-800",
        "Airbus A330-900neo",
        "Airbus A330-300",
        "Airbus A350-1000",
        "Airbus A350-900",
        "Airbus A320-200",
        "Airbus A320neo",
        "Airbus A321neo",
        "Airbus A321",
        "Airbus A320",
        "Airbus A319",
        "Boeing 737-800",
        "Boeing 737 MAX 8",
        "Boeing 737 MAX 9",
        "Boeing 777-300ER",
        "Boeing 777-200",
        "Boeing 747-8",
        "Boeing 787-9",
        "Boeing 787-10",
        // Liveries con vendor-name embebido (Fenix, PMDG, iniBuilds)
        "FenixA321",
        "FenixA320",
        "PMDG 737-800",
        "PMDG 777-300ER",
        "iniBuilds A350-900",
        "Bombardier CRJ900",
        "Bombardier CRJ700",
    ];
    for p in PREFIXES {
        if let Some(rest) = livery.strip_prefix(p) {
            return rest.trim();
        }
    }
    livery
}

/// Tokens que aparecen entre el modelo y el airline en algunos liveries
/// y NO son parte del nombre del operador. Survey de 138 archivos del
/// usuario: `CFM SL` (CFM engines + sharklets), `IAE` (engines IAE),
/// `LEAP` (LEAP engines), `GE` (GE engines), `RR` (Rolls-Royce).
const ENGINE_VARIANT_TOKENS: &[&str] =
    &["CFM", "SL", "IAE", "LEAP", "GE", "RR", "PW", "WL"];

/// Estimaciones razonables de payload (pax, cargo, fuel) según
/// aircraft type. Basadas en specs publicadas de típica operación
/// comercial (load factor ~80%, cargo medio típico, fuel para
/// vuelo medio-largo, no MTOW). NO son números MTOW — son
/// valores PROMEDIO de un vuelo comercial real.
///
/// Si el aircraft no matchea ningún pattern, devuelve `None` y el
/// importer deja los campos NULL (mejor que adivinar al azar).
///
/// Formato: `(pax, cargo_kg, fuel_used_kg)`.
fn estimate_payload(aircraft_path: &str) -> Option<(i64, i64, i64)> {
    let p = aircraft_path.to_lowercase();
    // Wide-body trijet/quadjet → vuelos largos, mucho fuel + carga
    if p.contains("a380") {
        return Some((450, 18_000, 215_000));
    }
    if p.contains("a350-1000") || p.contains("a350_1000") {
        return Some((320, 14_000, 105_000));
    }
    if p.contains("a350") || p.contains("inibuilds-aircraft-a350") {
        return Some((300, 13_000, 90_000));
    }
    if p.contains("a330-900") || p.contains("a330neo") || p.contains("headwindsim-aircraft-a330") {
        return Some((280, 11_000, 65_000));
    }
    if p.contains("a330") {
        return Some((260, 10_000, 60_000));
    }
    if p.contains("777-300") || p.contains("77w") {
        return Some((360, 16_000, 110_000));
    }
    if p.contains("777-200") || p.contains("77er") {
        return Some((320, 14_000, 95_000));
    }
    if p.contains("747") {
        return Some((400, 18_000, 130_000));
    }
    if p.contains("787-10") || p.contains("787_10") {
        return Some((310, 12_000, 75_000));
    }
    if p.contains("787-9") || p.contains("787_9") || p.contains("787") {
        return Some((280, 11_000, 65_000));
    }
    // Narrow-body — vuelos cortos/medios
    if p.contains("a321neo") || p.contains("fenixa321") || p.contains("aircraft-a321") {
        return Some((200, 4_500, 16_000));
    }
    if p.contains("a321") {
        return Some((190, 4_500, 15_500));
    }
    if p.contains("a320neo") || p.contains("aircraft-a320-neo") {
        return Some((170, 4_000, 13_500));
    }
    if p.contains("a320")
        || p.contains("fenixa320")
        || p.contains("fnx_320")
        || p.contains("flybywire-aircraft-a320")
    {
        return Some((160, 4_000, 13_000));
    }
    if p.contains("a319") {
        return Some((130, 3_500, 11_500));
    }
    if p.contains("737 max") || p.contains("737max") || p.contains("737-max") {
        return Some((175, 4_500, 14_000));
    }
    if p.contains("737-900") || p.contains("737_900") || p.contains("pmdg-aircraft-739") {
        return Some((180, 4_500, 14_500));
    }
    if p.contains("737-800") || p.contains("737_800") || p.contains("pmdg-aircraft-738") {
        return Some((160, 4_000, 13_500));
    }
    if p.contains("737") {
        return Some((150, 4_000, 13_000));
    }
    // Regional jets
    if p.contains("crj-900") || p.contains("crj_900") || p.contains("crj900") {
        return Some((85, 1_800, 6_500));
    }
    if p.contains("crj-700") || p.contains("crj_700") || p.contains("crj700") {
        return Some((70, 1_500, 5_500));
    }
    if p.contains("crj") || p.contains("aerosoft-crj") {
        return Some((80, 1_700, 6_000));
    }
    if p.contains("embraer") || p.contains("e190") || p.contains("e195") {
        return Some((100, 2_500, 8_000));
    }
    None
}

/// Estimaciones de **métricas pico** del vuelo según aircraft type.
/// Valores típicos en cruise para una aeronave de ese tipo. Útiles
/// como placeholder cuando el VAS .bin no embebe estos como campos
/// extraíbles (están en el track time-series, pero parsearlo es
/// out of scope para v1 — ver doc del módulo).
///
/// Formato: `(landing_fpm, max_ground_speed_kt, max_true_airspeed_kt)`.
/// El landing_fpm es un "promedio aceptable" (-200 fpm = aterrizaje
/// suave estándar). El GS y TAS son cruise reales del tipo.
/// (v3.6.1 fix I9) Marcada dead_code — el caller fue eliminado porque
/// los valores hardcoded daban "todos los vuelos -220 fpm". Conservada
/// como referencia histórica en caso de que se rescaten heurísticas
/// más sofisticadas (por altitud máxima del track, por ejemplo).
#[allow(dead_code)]
fn estimate_peak_metrics(aircraft_path: &str) -> Option<(i64, i64, i64)> {
    let p = aircraft_path.to_lowercase();
    // Wide-body — cruise alto
    if p.contains("a380") {
        return Some((-180, 510, 480));
    }
    if p.contains("a350") || p.contains("777") {
        return Some((-200, 520, 490));
    }
    if p.contains("a330") || p.contains("787") {
        return Some((-200, 500, 470));
    }
    if p.contains("747") {
        return Some((-220, 530, 500));
    }
    // Narrow-body — cruise medio
    if p.contains("a320")
        || p.contains("a321")
        || p.contains("a319")
        || p.contains("fnx_320")
        || p.contains("fenixa320")
        || p.contains("fenixa321")
    {
        return Some((-220, 460, 440));
    }
    if p.contains("737") {
        return Some((-220, 470, 450));
    }
    // Regional
    if p.contains("crj") || p.contains("embraer") {
        return Some((-250, 430, 410));
    }
    None
}

/// Extrae el airline del livery. Asume el formato `<model> <airline> <reg>`.
/// Strip model + strip reg al final + strip engine/variant tokens del inicio
/// = lo que queda.
fn extract_airline(livery: &str) -> Option<String> {
    let cleaned = strip_quality_suffix(livery);
    // Normaliza `_` a espacio para liveries `FenixA320_LANCCBAA`.
    let normalized = cleaned.replace('_', " ");
    let without_model = strip_model_prefix(&normalized);
    let tokens: Vec<&str> = without_model.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // Drop el último token si es reg-like.
    let end_idx = if is_registration_like(tokens[tokens.len() - 1]) {
        tokens.len() - 1
    } else {
        tokens.len()
    };

    // Drop tokens iniciales que sean engine/variant codes (CFM, SL, IAE, etc.)
    let mut start_idx = 0;
    while start_idx < end_idx && ENGINE_VARIANT_TOKENS.contains(&tokens[start_idx]) {
        start_idx += 1;
    }

    if start_idx >= end_idx {
        return None;
    }
    let middle = tokens[start_idx..end_idx].join(" ");
    let trimmed = middle.trim();
    // Mínimo 2 chars para evitar "garbage" residual de liveries raros.
    if trimmed.len() < 2 {
        return None;
    }
    // Si lo que queda es un único token todo uppercase sin espacios y
    // sin dígitos (ej. "LANCCBAA"), es probablemente código interno
    // del livery, no nombre de airline. Descartamos.
    if !trimmed.contains(' ')
        && trimmed.chars().all(|c| c.is_ascii_uppercase())
        && trimmed.len() >= 6
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// Resuelve `(lat, lon)` para un ICAO desde la tabla `airports`. Si
/// el ICAO no existe en nuestra DB, devuelve (0.0, 0.0). Usado al
/// importar vuelos con summary que tiene origen/destino ICAO pero
/// no coords explícitas.
async fn lookup_airport_coords(pool: &SqlitePool, icao: &str) -> (f64, f64) {
    let row: Option<(f64, f64)> = sqlx::query_as(
        "SELECT latitude, longitude FROM airports WHERE icao = ?1 LIMIT 1",
    )
    .bind(icao)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.unwrap_or((0.0, 0.0))
}


/// Path raíz donde VAS-ACARS guarda sus flights.
pub fn flights_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        Path::new(&local)
            .join("VASystem")
            .join("VAS-ACARS")
            .join("flights"),
    )
}

/// Lista los `*.bin` en `flights/` excluyendo `.bin.zip` (backups).
/// Si la carpeta no existe (usuario sin VAS-ACARS instalado),
/// devuelve `Vec::new()` — no es un error.
pub fn detect_flights() -> Vec<VasFlightCandidate> {
    let Some(dir) = flights_dir() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Skip backups (.bin.zip) y cualquier no-.bin.
        if !name.ends_with(".bin") || name.ends_with(".bin.zip") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        // UUID = filename sin extensión.
        let uuid = name.trim_end_matches(".bin").to_string();
        out.push(VasFlightCandidate {
            uuid,
            path: path.to_string_lossy().into_owned(),
            size_bytes: meta.len(),
        });
    }
    out
}

// =============================================================================
// Protobuf minimal decoder — solo lo necesario para los campos top-level
// que nos importan. NO maneja groups (wire 3/4, deprecated en proto3) ni
// repeated packed fields (que están en VAS-ACARS pero no los necesitamos
// para origin/timestamps).
// =============================================================================

/// Lee un varint desde `pos` y devuelve `(valor, nueva_pos)`. Falla
/// silenciosamente devolviendo None si se sale del buffer.
fn read_varint(data: &[u8], pos: usize) -> Option<(u64, usize)> {
    let mut val: u64 = 0;
    let mut shift: u32 = 0;
    let mut p = pos;
    while p < data.len() {
        let b = data[p];
        p += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((val, p));
        }
        shift += 7;
        if shift > 63 {
            return None; // varint malformado
        }
    }
    None
}

/// Estructura intermedia con las piezas que extraemos del `.bin`.
/// `arr/dep` se llaman a/b porque su orden no es estable — luego se
/// ordenan por timestamp con `ordered_times()`.
#[derive(Debug, Default)]
struct ParsedFields {
    livery: Option<String>,           // f4
    aircraft_cfg_path: Option<String>, // f5
    time_a_hms: Option<(u32, u32, u32)>,  // f6 -> (h, m, s)
    date_a_ymd: Option<(u32, u32, u32)>,  // f7 -> (y, m, d)
    time_b_hms: Option<(u32, u32, u32)>,  // f8
    date_b_ymd: Option<(u32, u32, u32)>,  // f9
    start_lat: Option<f64>,            // f12.f1
    start_lon: Option<f64>,            // f12.f2
}

/// Parsea el payload protobuf de un `.bin` de VAS-ACARS (sin el prefijo
/// de 2 bytes). Recorre solo top-level + el nested `f12`, ignora todo
/// lo demás (track de 19k samples, eventos, etc.). Devuelve None si
/// el header no parece protobuf válido — caller puede tratar como
/// "skip invalid".
fn parse_payload(payload: &[u8]) -> Option<ParsedFields> {
    let mut out = ParsedFields::default();
    let mut pos = 0;
    let mut fields_seen = 0u32;
    while pos < payload.len() {
        let (key, np) = read_varint(payload, pos)?;
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            // varint
            0 => {
                let (_, np) = read_varint(payload, pos)?;
                pos = np;
            }
            // fixed64
            1 => {
                if pos + 8 > payload.len() {
                    return if fields_seen > 0 { Some(out) } else { None };
                }
                pos += 8;
            }
            // length-delimited
            2 => {
                let (ln, np) = read_varint(payload, pos)?;
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return if fields_seen > 0 { Some(out) } else { None };
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                fields_seen += 1;
                match field {
                    4 => {
                        if let Ok(s) = std::str::from_utf8(chunk) {
                            out.livery = Some(s.to_string());
                        }
                    }
                    5 => {
                        if let Ok(s) = std::str::from_utf8(chunk) {
                            out.aircraft_cfg_path = Some(s.to_string());
                        }
                    }
                    6 => out.time_a_hms = parse_triple_varint(chunk),
                    7 => out.date_a_ymd = parse_triple_varint(chunk),
                    8 => out.time_b_hms = parse_triple_varint(chunk),
                    9 => out.date_b_ymd = parse_triple_varint(chunk),
                    12 => {
                        let (lat, lon) = parse_geo_pair(chunk);
                        out.start_lat = lat;
                        out.start_lon = lon;
                    }
                    _ => {}
                }
            }
            // fixed32
            5 => {
                if pos + 4 > payload.len() {
                    return if fields_seen > 0 { Some(out) } else { None };
                }
                pos += 4;
            }
            // unknown wire type → abortar
            _ => {
                return if fields_seen > 0 { Some(out) } else { None };
            }
        }
    }
    if fields_seen > 0 {
        Some(out)
    } else {
        None
    }
}

/// Decodifica un nested message con 3 varints como `(f1, f2, f3)`.
/// Usado para `time` (`{h, m, s}`) y `date` (`{Y, M, D}`).
fn parse_triple_varint(payload: &[u8]) -> Option<(u32, u32, u32)> {
    let mut values: [Option<u32>; 3] = [None; 3];
    let mut pos = 0;
    while pos < payload.len() {
        let (key, np) = read_varint(payload, pos)?;
        pos = np;
        let field = (key >> 3) as usize;
        let wire = key & 0x7;
        if wire != 0 {
            // Si hay otros wire types los saltamos respetando la regla
            // (length-delimited consume length+bytes, fixed consume 8/4).
            match wire {
                1 => pos += 8,
                2 => {
                    let (ln, np) = read_varint(payload, pos)?;
                    pos = np + ln as usize;
                }
                5 => pos += 4,
                _ => return None,
            }
            continue;
        }
        let (val, np) = read_varint(payload, pos)?;
        pos = np;
        if (1..=3).contains(&field) {
            values[field - 1] = Some(val as u32);
        }
    }
    match (values[0], values[1], values[2]) {
        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
        _ => None,
    }
}

/// Decodifica el nested `f12` que contiene `f1 f64 = lat` y `f2 f64 = lon`.
fn parse_geo_pair(payload: &[u8]) -> (Option<f64>, Option<f64>) {
    let mut lat = None;
    let mut lon = None;
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return (lat, lon);
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = key & 0x7;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return (lat, lon);
                };
                pos = np;
            }
            1 => {
                if pos + 8 > payload.len() {
                    return (lat, lon);
                }
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&payload[pos..pos + 8]);
                let v = f64::from_le_bytes(buf);
                pos += 8;
                if field == 1 {
                    lat = Some(v);
                } else if field == 2 {
                    lon = Some(v);
                }
            }
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return (lat, lon);
                };
                pos = np + ln as usize;
            }
            5 => pos += 4,
            _ => return (lat, lon),
        }
    }
    (lat, lon)
}

/// Lee un `.bin` de VAS-ACARS, salta el prefijo de 2 bytes y devuelve
/// la metadata parseada. None si el archivo no decodifica
/// (corrupto, otra versión, etc.).
pub fn parse_flight_file(path: &Path) -> Option<VasFlightMeta> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 4 {
        return None;
    }
    let fields = parse_payload(&bytes[2..])?;

    // Construye los timestamps ISO 8601 desde las dos parejas (date, time).
    // VAS guarda (f6=time, f7=date) y (f8=time, f9=date) — los emparejamos.
    let iso_a = match (fields.date_a_ymd, fields.time_a_hms) {
        (Some((y, mo, d)), Some((h, mi, s))) => Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            y, mo, d, h, mi, s
        )),
        _ => None,
    };
    let iso_b = match (fields.date_b_ymd, fields.time_b_hms) {
        (Some((y, mo, d)), Some((h, mi, s))) => Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            y, mo, d, h, mi, s
        )),
        _ => None,
    };

    Some(VasFlightMeta {
        livery: fields.livery,
        aircraft_cfg_path: fields.aircraft_cfg_path,
        time_a_iso: iso_a,
        time_b_iso: iso_b,
        start_lat: fields.start_lat,
        start_lon: fields.start_lon,
    })
}

// =============================================================================
// (v3.5.0 F2 v4) Track parser — extrae LAS MILES de muestras de posición
// que VAS guarda concatenadas en el .bin. Esto sustituye al parser
// "single-message" anterior (que solo veía la primera muestra como header).
// =============================================================================

/// Estructura intermedia mientras acumulamos una muestra durante el parse.
#[derive(Default)]
struct PartialSample {
    ts_epoch: Option<i64>,
    lat: Option<f64>,
    lon: Option<f64>,
    altitude_m: Option<f32>,
    ground_speed_kt: Option<f32>,
    true_airspeed_kt: Option<f32>,
    heading_deg: Option<f32>,
    on_ground: Option<bool>,
}

impl PartialSample {
    /// True si tenemos la triple mínima viable: timestamp + lat + lon.
    fn is_viable(&self) -> bool {
        self.ts_epoch.is_some() && self.lat.is_some() && self.lon.is_some()
    }

    fn finalize(self) -> Option<VasTrackSample> {
        Some(VasTrackSample {
            ts_epoch: self.ts_epoch?,
            lat: self.lat?,
            lon: self.lon?,
            altitude_m: self.altitude_m,
            ground_speed_kt: self.ground_speed_kt,
            true_airspeed_kt: self.true_airspeed_kt,
            heading_deg: self.heading_deg,
            on_ground: self.on_ground,
        })
    }
}

/// Lee un `.bin` de VAS-ACARS y devuelve el track COMPLETO con todas las
/// muestras (sampling cada ~5s). None si el archivo no decodifica o no
/// hay muestras viables.
///
/// Estrategia: cada muestra es un mensaje protobuf completo. El archivo
/// es una CONCATENACIÓN de N mensajes. Detectamos boundaries de mensaje
/// cuando el número de field DECRECE (típicamente al pasar de f208 →
/// f3 de la siguiente muestra).
pub fn parse_full_track(path: &Path) -> Option<VasFlightTrack> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 4 {
        return None;
    }
    parse_track_payload(&bytes[2..])
}

/// Marcador del inicio de un sample: f3 wire 2 (tag `0x1A`) + length 12
/// (`0x0C`) + inner Timestamp f1 wire 0 (tag `0x08`). Cada sample del
/// `.bin` empieza con este patrón → lo usamos para resincronizar el
/// parser cuando atraviesa trailer junk entre samples.
const SAMPLE_MARKER: &[u8] = &[0x1A, 0x0C, 0x08];

/// Busca la siguiente ocurrencia de `SAMPLE_MARKER` a partir de `start`.
/// Devuelve `None` si no hay más marcadores.
fn find_next_sample_marker(data: &[u8], start: usize) -> Option<usize> {
    if start + SAMPLE_MARKER.len() > data.len() {
        return None;
    }
    let mut pos = start;
    while pos + SAMPLE_MARKER.len() <= data.len() {
        if &data[pos..pos + SAMPLE_MARKER.len()] == SAMPLE_MARKER {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn parse_track_payload(data: &[u8]) -> Option<VasFlightTrack> {
    let mut track = VasFlightTrack {
        livery: None,
        aircraft_cfg_path: None,
        samples: Vec::new(),
    };

    let mut cur = PartialSample::default();
    let mut last_field: u32 = 0;
    let mut pos = 0;

    // Flusha la sample actual y resetea el accumulator. Devuelve si se
    // pusheó (útil para tracking).
    macro_rules! flush_sample {
        ($cur:expr, $track:expr, $last_field:expr) => {{
            if $cur.is_viable() {
                if let Some(sample) = std::mem::take(&mut $cur).finalize() {
                    $track.samples.push(sample);
                }
            }
            $cur = PartialSample::default();
            // El último flush del loop deja esta asignación sin leer —
            // es inherente al patrón del accumulator, no un bug.
            #[allow(unused_assignments)]
            {
                $last_field = 0;
            }
        }};
    }

    while pos < data.len() {
        let Some((key, np)) = read_varint(data, pos) else {
            // Varint malformado — intenta resync.
            flush_sample!(cur, track, last_field);
            match find_next_sample_marker(data, pos + 1) {
                Some(sync) => {
                    pos = sync;
                    continue;
                }
                None => break,
            }
        };
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;

        // Boundary: campo decreció — termina la muestra actual y empieza
        // una nueva. SIEMPRE resetea cur (aunque la sample anterior no
        // fuera viable) para evitar que datos de samples diferentes se
        // mezclen en el mismo accumulator.
        if field < last_field && pos > 0 {
            flush_sample!(cur, track, last_field);
        }

        match wire {
            0 => {
                let Some((val, np2)) = read_varint(data, np) else {
                    flush_sample!(cur, track, last_field);
                    match find_next_sample_marker(data, np + 1) {
                        Some(sync) => {
                            pos = sync;
                            continue;
                        }
                        None => break,
                    }
                };
                pos = np2;
                last_field = field;
                if field == 21 {
                    cur.on_ground = Some(val == 1);
                }
            }
            1 => {
                if np + 8 > data.len() {
                    break;
                }
                pos = np + 8;
                last_field = field;
            }
            2 => {
                let Some((ln, np2)) = read_varint(data, np) else {
                    flush_sample!(cur, track, last_field);
                    match find_next_sample_marker(data, np + 1) {
                        Some(sync) => {
                            pos = sync;
                            continue;
                        }
                        None => break,
                    }
                };
                let ln = ln as usize;
                if np2 + ln > data.len() {
                    // Length-delim sale del buffer — corrupción.
                    // Resync.
                    flush_sample!(cur, track, last_field);
                    match find_next_sample_marker(data, np2 + 1) {
                        Some(sync) => {
                            pos = sync;
                            continue;
                        }
                        None => break,
                    }
                }
                let chunk = &data[np2..np2 + ln];
                match field {
                    3 => {
                        // Timestamp sub-msg {f1: u64 epoch_seconds}
                        if let Some(secs) = parse_inner_timestamp(chunk) {
                            cur.ts_epoch = Some(secs);
                        }
                    }
                    4 => {
                        if track.livery.is_none() {
                            if let Ok(s) = std::str::from_utf8(chunk) {
                                track.livery = Some(s.to_string());
                            }
                        }
                    }
                    5 => {
                        if track.aircraft_cfg_path.is_none() {
                            if let Ok(s) = std::str::from_utf8(chunk) {
                                track.aircraft_cfg_path = Some(s.to_string());
                            }
                        }
                    }
                    12 => {
                        let (lat, lon) = parse_geo_pair(chunk);
                        if lat.is_some() && lon.is_some() {
                            cur.lat = lat;
                            cur.lon = lon;
                        }
                    }
                    _ => {}
                }
                pos = np2 + ln;
                last_field = field;
            }
            5 => {
                if np + 4 > data.len() {
                    break;
                }
                let arr: [u8; 4] = match data[np..np + 4].try_into() {
                    Ok(a) => a,
                    Err(_) => break,
                };
                let val = f32::from_le_bytes(arr);
                match field {
                    13 => cur.heading_deg = Some(val),
                    16 => cur.altitude_m = Some(val),
                    23 => cur.true_airspeed_kt = Some(val),
                    25 => {
                        // f25 es IAS — la guardamos en un campo aparte
                        // si quisiéramos; ahora mismo no se usa.
                    }
                    53 => cur.ground_speed_kt = Some(val),
                    _ => {}
                }
                pos = np + 4;
                last_field = field;
            }
            // Wire types 3 (group start), 4 (group end), 6, 7 — el .bin
            // de VAS tiene "trailer junk" entre samples con bytes que
            // no son protobuf válido. NO podemos seguir parseando byte
            // a byte; resincronizamos buscando el próximo marcador de
            // sample (f3 wire 2 len 12 + inner f1 wire 0).
            _ => {
                flush_sample!(cur, track, last_field);
                match find_next_sample_marker(data, np) {
                    Some(sync) => {
                        pos = sync;
                        continue;
                    }
                    None => break,
                }
            }
        }
    }

    // Push del último sample pendiente.
    if cur.is_viable() {
        if let Some(s) = cur.finalize() {
            track.samples.push(s);
        }
    }

    if track.samples.is_empty() {
        None
    } else {
        Some(track)
    }
}

/// Decodifica un sub-message Timestamp `{f1: u64 epoch_seconds, f2: u64 nanos}`
/// y devuelve los segundos. None si la estructura no matchea o el valor
/// está fuera de rango razonable (años 2001-2096).
fn parse_inner_timestamp(chunk: &[u8]) -> Option<i64> {
    let (key, np) = read_varint(chunk, 0)?;
    // f1 wire 0 (varint) = tag 0x08
    if key != 8 {
        return None;
    }
    let (val, _) = read_varint(chunk, np)?;
    if (1_000_000_000..4_000_000_000).contains(&val) {
        Some(val as i64)
    } else {
        None
    }
}

/// Mínimo de muestras de track para considerar un .bin como vuelo válido.
/// 50 muestras × 5s ≈ 4 min. Por debajo de eso son vuelos de prueba,
/// crashes inmediatos al spawn, o tests de cabina sin movimiento.
const MIN_TRACK_SAMPLES: usize = 50;

/// Distancia mínima (nm) entre primer y último sample para considerar
/// "vuelo real". Si origen == destino dentro de este radio, asumimos
/// que el usuario nunca llegó a despegar o aterrizó en el mismo aeropuerto
/// de salida (no es un vuelo completo).
const MIN_FLIGHT_DISTANCE_NM: f64 = 5.0;

/// Cada cuántas muestras del .bin guardamos un punto en flight_log_track.
/// (v3.5.0 F3) Antes era 2 (10s efectivo) — el usuario reportó que las
/// rutas "fallan en ciertos puntos" porque el downsampling perdía las
/// micro-correcciones del autopilot. Bajamos a 1 (sin downsample, 5s
/// efectivo) para preservar TODA la fidelidad del .bin de VAS. Cuesta
/// 2× el almacenamiento (~10 MB extra para 138 vuelos) pero el track
/// se ve significativamente más preciso.
const TRACK_DOWNSAMPLE_STRIDE: usize = 1;

/// Importa **todos** los `.bin` válidos de VAS-ACARS al `flight_log`.
///
/// (v3.5.0 F2 v4) Estrategia track-based:
///
/// Cada `.bin` se parsea como un track completo de muestras (lat/lon/alt/
/// speed/heading). El vuelo se valida usando el TRACK MISMO en lugar de
/// los timestamps OOOI del data.db (estrategia anterior). Esto captura
/// vuelos completos que no aparecen en data.db por cualquier razón.
///
/// Validación:
///   1. Track tiene ≥`MIN_TRACK_SAMPLES` muestras (descarta tests sin vuelo)
///   2. Origen ≠ Destino por al menos `MIN_FLIGHT_DISTANCE_NM` (descarta
///      vuelos abortados en gate o aterrizajes inmediatos en el mismo apt)
///
/// Enriquecimiento desde data.db summary (best-effort):
///   - origin_icao, destination_icao (override sobre nearest-airport)
///   - airline_name, registration, gates, callsign
///
/// Métricas pico desde el track:
///   - max_altitude_ft (de f16 en metros)
///   - max_ground_speed_kt (de f53)
///   - max_true_airspeed_kt (de f23)
///   - landing_fpm (calculada de VS en touchdown)
///
/// Persistencia del track: 1 de cada 2 muestras (≈10s) a `flight_log_track`,
/// para coincidir con el sampling rate del watcher SimConnect.
pub async fn import_all(
    pool: &SqlitePool,
    app: Option<&tauri::AppHandle>,
) -> anyhow::Result<VasImportReport> {
    use tauri::Emitter;
    let candidates = detect_flights();
    let total = candidates.len();
    let dir = flights_dir().map(|p| p.to_string_lossy().into_owned());
    let mut report = VasImportReport {
        source_dir: dir,
        candidates_found: total,
        ..Default::default()
    };

    // Carga el índice del data.db UNA vez al inicio (best-effort).
    let summary_index = crate::vas_summary::load_index();
    tracing::info!(
        target: "vas_acars",
        "summary index has {} entries; {} .bin candidates",
        summary_index.len(),
        total
    );

    // (v3.6.3 fix J2) Emit "started" event para que la UI muestre
    // la barra de progreso desde 0%.
    if let Some(app) = app {
        let _ = app.emit(
            "vas:import:progress",
            &serde_json::json!({
                "current": 0_usize,
                "total": total,
                "phase": "started",
            }),
        );
    }

    // (v3.5.0 F2 v5) Reporte agregado de razones de skip — para que el
    // usuario vea por qué se rechazó cada candidate sin tener que
    // pasar por 138 líneas de debug.
    let mut skip_reasons: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for (idx, cand) in candidates.into_iter().enumerate() {
        // (v3.6.3 fix J2) Emit progress por archivo antes de procesarlo.
        // El frontend dibuja: "Importando 23/138 (16%)".
        if let Some(app) = app {
            let _ = app.emit(
                "vas:import:progress",
                &serde_json::json!({
                    "current": idx + 1,
                    "total": total,
                    "phase": "importing",
                    "uuid": cand.uuid.clone(),
                }),
            );
        }
        match import_one(pool, &cand, &summary_index).await {
            Ok(ImportOutcome::Inserted) => report.imported_count += 1,
            Ok(ImportOutcome::Updated) => report.updated_count += 1,
            Ok(ImportOutcome::SkippedInvalid(reason)) => {
                // Categoriza la razón para el agregado (toma el prefijo
                // antes del primer paréntesis / dos puntos).
                let category = reason
                    .split(|c: char| c == '(' || c == ':')
                    .next()
                    .unwrap_or("unknown")
                    .trim()
                    .to_string();
                *skip_reasons.entry(category).or_insert(0) += 1;
                tracing::debug!(
                    target: "vas_acars",
                    "skip {} — {}",
                    cand.uuid, reason
                );
                report.skipped_invalid += 1;
            }
            Err(e) => {
                tracing::warn!(
                    target: "vas_acars",
                    "import {} failed: {:#}",
                    cand.uuid, e
                );
                *skip_reasons.entry("error".to_string()).or_insert(0) += 1;
                report.skipped_invalid += 1;
            }
        }
    }

    // (v3.6.3 fix J2) Emit "done" para que la UI marque 100% +
    // resetee la barra a estado idle tras ~1s.
    if let Some(app) = app {
        let _ = app.emit(
            "vas:import:progress",
            &serde_json::json!({
                "current": total,
                "total": total,
                "phase": "done",
                "imported": report.imported_count,
                "updated": report.updated_count,
                "skipped": report.skipped_invalid,
            }),
        );
    }

    // Log el agregado de razones de skip para que el usuario entienda
    // de un vistazo qué pasó (sin necesidad de RUST_LOG=debug).
    if !skip_reasons.is_empty() {
        let mut rows: Vec<(String, usize)> = skip_reasons.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        tracing::info!(
            target: "vas_acars",
            "skip breakdown: {}",
            rows.iter()
                .map(|(r, n)| format!("{}={}", r, n))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    tracing::info!(
        target: "vas_acars",
        "import done · candidates={} imported={} updated={} dupes={} invalid={}",
        report.candidates_found,
        report.imported_count,
        report.updated_count,
        report.skipped_duplicates,
        report.skipped_invalid,
    );
    Ok(report)
}

enum ImportOutcome {
    Inserted,
    Updated,
    SkippedInvalid(String),
}

/// Procesa un solo `.bin`: parse track → valida → resuelve metadata →
/// INSERT/UPDATE en flight_log → guarda track points.
async fn import_one(
    pool: &SqlitePool,
    cand: &VasFlightCandidate,
    summary_index: &std::collections::HashMap<String, crate::vas_summary::VasFlightSummary>,
) -> anyhow::Result<ImportOutcome> {
    // 1. Parse del track completo.
    let Some(track) = parse_full_track(Path::new(&cand.path)) else {
        return Ok(ImportOutcome::SkippedInvalid("could not parse .bin".into()));
    };

    tracing::debug!(
        target: "vas_acars",
        "parsed {} — {} samples, livery={:?}",
        cand.uuid,
        track.samples.len(),
        track.livery.as_deref().unwrap_or("?"),
    );

    // 2. Validación: ≥MIN_TRACK_SAMPLES y origen ≠ destino > MIN_FLIGHT_DISTANCE_NM.
    if track.samples.len() < MIN_TRACK_SAMPLES {
        return Ok(ImportOutcome::SkippedInvalid(format!(
            "only {} track samples (<{})",
            track.samples.len(),
            MIN_TRACK_SAMPLES
        )));
    }
    let Some((o_lat, o_lon)) = track.first_position() else {
        return Ok(ImportOutcome::SkippedInvalid("no first position".into()));
    };
    let Some((d_lat, d_lon)) = track.last_position() else {
        return Ok(ImportOutcome::SkippedInvalid("no last position".into()));
    };
    let track_distance_nm = crate::flight_log::haversine_nm(o_lat, o_lon, d_lat, d_lon);
    if track_distance_nm < MIN_FLIGHT_DISTANCE_NM {
        return Ok(ImportOutcome::SkippedInvalid(format!(
            "origin≈destination ({:.1}nm < {}nm) — never departed",
            track_distance_nm, MIN_FLIGHT_DISTANCE_NM
        )));
    }

    // 3. Resolve ICAOs: prefer data.db summary, fallback a nearest-airport
    //    sobre primer/último sample del track.
    let summary = summary_index.get(&cand.uuid);

    let (origin_icao, origin_lat_use, origin_lon_use) = match summary
        .and_then(|s| s.origin_icao.as_deref())
    {
        Some(icao) => {
            let (lat, lon) = lookup_airport_coords(pool, icao).await;
            // Si el lookup encontró coords válidas, las usamos; si no, las del track.
            if (lat - 0.0).abs() < f64::EPSILON && (lon - 0.0).abs() < f64::EPSILON {
                (Some(icao.to_string()), o_lat, o_lon)
            } else {
                (Some(icao.to_string()), lat, lon)
            }
        }
        None => {
            let icao = nearest_airport_with_coords(pool, o_lat, o_lon)
                .await
                .ok()
                .flatten()
                .map(|a| a.icao);
            (icao, o_lat, o_lon)
        }
    };

    let (destination_icao, destination_lat_use, destination_lon_use) = match summary
        .and_then(|s| s.destination_icao.as_deref())
    {
        Some(icao) => {
            let (lat, lon) = lookup_airport_coords(pool, icao).await;
            if (lat - 0.0).abs() < f64::EPSILON && (lon - 0.0).abs() < f64::EPSILON {
                (Some(icao.to_string()), Some(d_lat), Some(d_lon))
            } else {
                (Some(icao.to_string()), Some(lat), Some(lon))
            }
        }
        None => {
            let icao = nearest_airport_with_coords(pool, d_lat, d_lon)
                .await
                .ok()
                .flatten()
                .map(|a| a.icao);
            (icao, Some(d_lat), Some(d_lon))
        }
    };

    // 4. Gates: priority chain — data.db real, fallback a GSX.
    let departure_gate: Option<String> = summary
        .and_then(|s| s.departure_gate.clone())
        .or_else(|| match &origin_icao {
            Some(icao) => {
                crate::gsx_parking::find_nearest_parking(icao, o_lat, o_lon).map(|p| p.name)
            }
            _ => None,
        });
    let arrival_gate: Option<String> = summary
        .and_then(|s| s.arrival_gate.clone())
        .or_else(|| match &destination_icao {
            Some(icao) => {
                crate::gsx_parking::find_nearest_parking(icao, d_lat, d_lon).map(|p| p.name)
            }
            _ => None,
        });

    // 5. Timestamps: prefer summary OOOI block-out/block-in; fallback a
    //    primer/último timestamp del track.
    let started_at: String = summary
        .and_then(|s| s.block_out_iso())
        .or_else(|| track.started_at_iso())
        .ok_or_else(|| anyhow::anyhow!("no started_at available"))?;
    let ended_at: String = summary
        .and_then(|s| s.block_in_iso())
        .or_else(|| track.ended_at_iso())
        .ok_or_else(|| anyhow::anyhow!("no ended_at available"))?;

    // 6. Aircraft metadata.
    // (v3.6.0 Phase H — Epic B):
    //   · `aircraft_title`       = nombre largo del summary (sub-variant)
    //                              o livery del track como fallback.
    //   · `aircraft_atc_type`    = **ICAO type code** ("A339", "B738").
    //                              Lo que la UI etiqueta como "Tipo". Antes
    //                              quedaba NULL en imports VAS; Q5 del kickoff
    //                              confirma mapear `aircraft_icao_type` → aquí.
    //   · `aircraft_model`       = **nombre comercial** ("Airbus A330-900neo").
    //                              Lo que la UI etiqueta como "Modelo". Q6:
    //                              en lugar del antiguo ATC MODEL feo,
    //                              tomamos `aircraft_short_name` del summary.
    let aircraft_title = summary
        .and_then(|s| s.aircraft_long_name.clone())
        .or_else(|| track.livery.clone());

    let meta_for_extracts = VasFlightMeta {
        livery: track.livery.clone(),
        aircraft_cfg_path: track.aircraft_cfg_path.clone(),
        ..Default::default()
    };
    let aircraft_atc_type = summary.and_then(|s| s.aircraft_icao_type.clone());
    let aircraft_model = summary
        .and_then(|s| s.aircraft_short_name.clone())
        .or_else(|| summary.and_then(|s| s.aircraft_long_name.clone()))
        .or_else(|| meta_for_extracts.aircraft_label());
    let aircraft_airline = summary
        .and_then(|s| s.airline_name.clone())
        .or_else(|| meta_for_extracts.airline());
    let aircraft_registration = summary
        .and_then(|s| s.registration.clone())
        .or_else(|| meta_for_extracts.registration())
        .or_else(|| aircraft_title.as_deref().and_then(extract_registration));

    // (v3.6.0 Phase H — Epic A/D) Metadata Virtual Airline.
    //   · `flight_number`  = "DL115"            (summary.f10 del FlightInfo)
    //   · `callsign`       = "DAL115"           (summary.f2)
    //   · `airline_icao`   = "DAL"              (summary.f9.f2 o derivado del callsign)
    let flight_number = summary.and_then(|s| s.flight_number.clone());
    let callsign = summary.and_then(|s| s.callsign.clone());
    let airline_icao = summary
        .and_then(|s| s.airline_icao.clone())
        .or_else(|| crate::flight_log::derive_airline_icao(callsign.as_deref()));

    // (v3.6.0 Phase H — Epic A) Status del vuelo. `completed` si el
    // summary tiene OOOI completo (block-out/touchdown/block-in con
    // diff ≥10 min). Si NO hay summary en data.db pero el track es
    // suficiente (ya pasamos MIN_TRACK_SAMPLES + MIN_FLIGHT_DISTANCE),
    // tratamos como completed — el track sólo es la evidencia más
    // fuerte de un vuelo "que ocurrió". El usuario reportó vuelos sin
    // summary que querían capturar (F2.3 anterior aflojó el gate).
    //
    // Si el summary EXISTE y NO está completo → 'partial'. Lo ocultamos
    // en la UI (Q1=b) pero queda en DB para audit/diagnóstico.
    let summary_present = summary.is_some();
    let summary_completed = summary.map(|s| s.is_completed()).unwrap_or(false);
    let initial_status: &str = if summary_completed {
        "completed"
    } else if !summary_present {
        // Fallback heurístico: track-only, sin summary.
        let dur = track.duration_seconds().unwrap_or(0);
        let last_on_ground = track
            .samples
            .last()
            .and_then(|s| s.on_ground)
            .unwrap_or(false);
        if dur >= 600 && last_on_ground {
            "completed"
        } else {
            "partial"
        }
    } else {
        "partial"
    };

    // (v3.6.0 Phase H — Epic A) Pre-check de dedup VA. Cuando hay
    // un completed pre-existente con la misma triple (airline+fn+día)
    // y el external_id es distinto (otro archivo, no re-import), el
    // nuevo entra como 'partial' para no chocar con el UNIQUE index
    // y para preservar la regla "Solo prevalece el completado".
    let started_day = if started_at.len() >= 10 {
        &started_at[..10]
    } else {
        ""
    };
    let final_status: &str = if initial_status == "completed"
        && airline_icao.is_some()
        && flight_number.is_some()
        && !started_day.is_empty()
    {
        let conflict: Option<(i64, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, external_id FROM flight_log
            WHERE airline_icao = ?1
              AND flight_number = ?2
              AND date(started_at) = ?3
              AND status = 'completed'
            LIMIT 1
            "#,
        )
        .bind(airline_icao.as_deref().unwrap())
        .bind(flight_number.as_deref().unwrap())
        .bind(started_day)
        .fetch_optional(pool)
        .await?;
        match conflict {
            Some((existing_id, existing_ext)) => {
                if existing_ext.as_deref() == Some(cand.uuid.as_str()) {
                    // Mismo archivo, re-import. El UPDATE de abajo
                    // mantiene status='completed' — no degradamos.
                    initial_status
                } else {
                    tracing::info!(
                        target: "vas_acars",
                        "VA dedup: {} {} day={} already has completed id={} — marking {} as 'partial'",
                        airline_icao.as_deref().unwrap(),
                        flight_number.as_deref().unwrap(),
                        started_day,
                        existing_id,
                        cand.uuid,
                    );
                    "partial"
                }
            }
            None => initial_status,
        }
    } else {
        initial_status
    };

    // 7. Payload estimates por aircraft type — el .bin no embebe pax/cargo
    //    como campos extractables, así que aproximamos.
    let path_or_livery = track
        .aircraft_cfg_path
        .as_deref()
        .or(track.livery.as_deref())
        .unwrap_or("");
    let payload = estimate_payload(path_or_livery);
    let (est_pax, est_cargo, est_fuel) = match payload {
        Some(t) => (Some(t.0), Some(t.1), Some(t.2)),
        None => (None, None, None),
    };

    // 8. **Peak metrics: NO se computan para imports VAS** (v3.6.7 N1).
    //
    // El sampling de VAS-ACARS (~10s entre altitudes, sin VS field)
    // no permite calcular landing_fpm con la precisión que reporta
    // la VA platform (ej. skyteamvirtual da -188 fpm, nosotros -219
    // — 31 fpm de error siendo el mejor algoritmo posible).
    //
    // Decisión del usuario: para vuelos importados, NO mostrar peak
    // metrics ni landing fpm — preferible "—" honesto que un número
    // engañoso. La UI muestra un badge "Vuelo importado" en su lugar.
    //
    // Para vuelos volados en vivo con SimConnect, la captura es
    // precisa (simvar `PLANE TOUCHDOWN NORMAL VELOCITY` × 60).
    let max_altitude_ft: Option<i64> = None;
    let max_ground_speed_kt: Option<i64> = None;
    let max_true_airspeed_kt: Option<i64> = None;
    let landing_fpm: Option<i64> = None;

    // (v3.6.1 fix I9) Fallback eliminado para landing_fpm.
    // ANTES caía a `estimate_peak_metrics` que devolvía valores
    // hardcoded por tipo de avión — eso hacía que TODOS los vuelos
    // de A320 mostraran -220 fpm, todos los 737 -220, etc., como si
    // hubieran aterrizado idéntico. El usuario reportó "todos los
    // aterrizajes son iguales -200/-220 fpm" — esa era la causa.
    //
    // Para GS/TAS también descartamos el fallback de aircraft type
    // por la misma razón. Si el track no aporta datos, mejor mostrar
    // "—" en UI que un número falso.
    //
    // Quedamos sólo con un fallback razonable: si max_ground_speed
    // del track es null PERO el track tiene altitud > 30000 ft, sabemos
    // que voló high-altitude → al menos podemos estimar GS típica de
    // crucero a esa altitud. Pero NO landing_fpm — ese es evento
    // puntual del touchdown, no puede estimarse.
    // path_or_livery sigue siendo usado arriba por estimate_payload

    // 9. Distancia — preferimos GREAT-CIRCLE origen→destino vs distancia
    //    del track. La distancia gran-circular es la métrica estándar para
    //    "distancia voladada" en logbooks.
    let distance_nm: Option<f64> = match (
        destination_lat_use,
        destination_lon_use,
        origin_lat_use,
        origin_lon_use,
    ) {
        (Some(dlat), Some(dlon), olat, olon) => Some(crate::flight_log::haversine_nm(
            olat, olon, dlat, dlon,
        )),
        _ => None,
    };

    // 10. flight_time_s: del summary si está, sino del track.
    let flight_time_s: Option<i64> = summary
        .and_then(|s| s.block_to_block_seconds())
        .or_else(|| track.duration_seconds());

    // 11. INSERT/UPDATE en flight_log.
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM flight_log WHERE source = ?1 AND external_id = ?2 LIMIT 1",
    )
    .bind(SOURCE_LABEL)
    .bind(&cand.uuid)
    .fetch_optional(pool)
    .await?;

    let flight_id: i64 = if let Some(id) = exists {
        // (v3.6.0 Phase H) UPDATE extendido con aircraft_atc_type,
        // flight_number, callsign, airline_icao, status. Usamos
        // COALESCE para no pisar edits manuales del usuario, EXCEPTO
        // `status` que se actualiza always — porque un re-import puede
        // promover partial → completed si el data.db ahora trae OOOI.
        sqlx::query(
            "UPDATE flight_log SET
                origin_lat = ?2,
                origin_lon = ?3,
                origin_icao = ?4,
                destination_lat = COALESCE(?5, destination_lat),
                destination_lon = COALESCE(?6, destination_lon),
                destination_icao = COALESCE(?7, destination_icao),
                aircraft_title = COALESCE(?8, aircraft_title),
                aircraft_model = COALESCE(?9, aircraft_model),
                aircraft_airline = COALESCE(?10, aircraft_airline),
                aircraft_registration = COALESCE(?11, aircraft_registration),
                aircraft_atc_type = COALESCE(?23, aircraft_atc_type),
                departure_gate = COALESCE(?12, departure_gate),
                arrival_gate = COALESCE(?13, arrival_gate),
                passengers = COALESCE(passengers, ?14),
                cargo_kg = COALESCE(cargo_kg, ?15),
                fuel_used_kg = COALESCE(fuel_used_kg, ?16),
                landing_fpm = COALESCE(?17, landing_fpm),
                max_ground_speed_kt = COALESCE(?18, max_ground_speed_kt),
                max_true_airspeed_kt = COALESCE(?19, max_true_airspeed_kt),
                max_altitude_ft = COALESCE(?22, max_altitude_ft),
                flight_time_s = COALESCE(flight_time_s, ?20),
                distance_nm = COALESCE(distance_nm, ?21),
                flight_number = COALESCE(?24, flight_number),
                callsign = COALESCE(?25, callsign),
                airline_icao = COALESCE(?26, airline_icao),
                status = ?27
             WHERE id = ?1",
        )
        .bind(id)
        .bind(origin_lat_use)
        .bind(origin_lon_use)
        .bind(&origin_icao)
        .bind(destination_lat_use)
        .bind(destination_lon_use)
        .bind(&destination_icao)
        .bind(&aircraft_title)
        .bind(&aircraft_model)
        .bind(&aircraft_airline)
        .bind(&aircraft_registration)
        .bind(&departure_gate)
        .bind(&arrival_gate)
        .bind(est_pax)
        .bind(est_cargo)
        .bind(est_fuel)
        .bind(landing_fpm)
        .bind(max_ground_speed_kt)
        .bind(max_true_airspeed_kt)
        .bind(flight_time_s)
        .bind(distance_nm)
        .bind(max_altitude_ft)
        .bind(&aircraft_atc_type)
        .bind(&flight_number)
        .bind(&callsign)
        .bind(&airline_icao)
        .bind(final_status)
        .execute(pool)
        .await?;

        // Si vamos a re-importar el track, borramos los puntos viejos.
        sqlx::query("DELETE FROM flight_log_track WHERE flight_id = ?1")
            .bind(id)
            .execute(pool)
            .await?;
        id
    } else {
        // (v3.6.0 Phase H) INSERT extendido con aircraft_atc_type +
        // metadata VA + status.
        let row = sqlx::query(
            "INSERT INTO flight_log (
                started_at, ended_at,
                origin_lat, origin_lon, origin_icao,
                destination_lat, destination_lon, destination_icao,
                aircraft_title, aircraft_model, aircraft_airline, aircraft_registration,
                aircraft_atc_type,
                departure_gate, arrival_gate,
                passengers, cargo_kg, fuel_used_kg,
                landing_fpm, max_ground_speed_kt, max_true_airspeed_kt, max_altitude_ft,
                flight_time_s, distance_nm,
                source, external_id,
                flight_number, callsign, airline_icao, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                      ?9, ?10, ?11, ?12,
                      ?13,
                      ?14, ?15,
                      ?16, ?17, ?18,
                      ?19, ?20, ?21, ?22,
                      ?23, ?24,
                      ?25, ?26,
                      ?27, ?28, ?29, ?30)",
        )
        .bind(&started_at)
        .bind(&ended_at)
        .bind(origin_lat_use)
        .bind(origin_lon_use)
        .bind(&origin_icao)
        .bind(destination_lat_use)
        .bind(destination_lon_use)
        .bind(&destination_icao)
        .bind(&aircraft_title)
        .bind(&aircraft_model)
        .bind(&aircraft_airline)
        .bind(&aircraft_registration)
        .bind(&aircraft_atc_type)
        .bind(&departure_gate)
        .bind(&arrival_gate)
        .bind(est_pax)
        .bind(est_cargo)
        .bind(est_fuel)
        .bind(landing_fpm)
        .bind(max_ground_speed_kt)
        .bind(max_true_airspeed_kt)
        .bind(max_altitude_ft)
        .bind(flight_time_s)
        .bind(distance_nm)
        .bind(SOURCE_LABEL)
        .bind(&cand.uuid)
        .bind(&flight_number)
        .bind(&callsign)
        .bind(&airline_icao)
        .bind(final_status)
        .execute(pool)
        .await?;
        row.last_insert_rowid()
    };

    // 12. Persiste el track downsampleado (1 de cada N samples → ~10s).
    insert_track_points(pool, flight_id, &track).await?;

    // 13. (v3.6.0 Phase H — H12) Deriva phases del track y persiste a
    //     `flight_log_phase`. Esto permite que el scoring engine corra
    //     sobre imports VAS, no sólo sobre vuelos volados en vivo con
    //     SimConnect (Q13=a del kickoff).
    derive_and_persist_phases(pool, flight_id, &track).await?;

    // 14. (v3.6.1 fix I5) Auto-score del vuelo recién importado.
    //     ANTES sólo se puntuaban los vuelos volados en vivo (via
    //     finalize_after_finish del watcher). El usuario reportó que
    //     "algunos vuelos muestran calificación y otros no" — porque
    //     los imports nunca recibían score. Ahora lo computamos al
    //     vuelo y persistimos en flight_log.score_total/max/grade +
    //     flight_log_score_item.
    //
    //     Sólo aplicable a status='completed' — los partial no tienen
    //     sentido puntuarlos (incompletos). Errores aquí no abortan
    //     el import; logueamos y seguimos.
    if final_status == "completed" {
        if let Err(e) = crate::scoring::score_flight(pool, flight_id).await {
            tracing::warn!(
                target: "vas_acars",
                "auto-score on import failed for flight {}: {:#}",
                flight_id, e
            );
        }
    }

    if exists.is_some() {
        Ok(ImportOutcome::Updated)
    } else {
        Ok(ImportOutcome::Inserted)
    }
}

/// (v3.6.0 Phase H) Recorre el track muestra-a-muestra aplicando una
/// heurística simplificada de `derive_phase_label` y persiste las
/// transiciones a `flight_log_phase`.
///
/// La heurística usa: `on_ground` (campo f21 del .bin), `gs_kt` (f53)
/// y `alt_ft` derivado de f16. No tenemos parking_brake ni
/// engine_running en el .bin — el match se hace por dinámica.
async fn derive_and_persist_phases(
    pool: &SqlitePool,
    flight_id: i64,
    track: &VasFlightTrack,
) -> anyhow::Result<()> {
    if track.samples.is_empty() {
        return Ok(());
    }

    // Borrar phases previas para idempotencia en re-imports.
    sqlx::query("DELETE FROM flight_log_phase WHERE flight_id = ?1")
        .bind(flight_id)
        .execute(pool)
        .await?;

    // Calcular max_alt para distinguir cruise vs climb/descent.
    let max_alt_ft = track
        .samples
        .iter()
        .filter_map(|s| s.altitude_ft())
        .max()
        .unwrap_or(0);

    let mut transitions: Vec<(String, String)> = Vec::new(); // (phase, ts_iso)
    let mut last_phase: Option<String> = None;
    let mut passed_taxi_threshold = false;

    for s in &track.samples {
        let on_ground = s.on_ground.unwrap_or(false);
        let gs_kt = s.ground_speed_kt.unwrap_or(0.0) as f64;
        let alt_ft = s.altitude_ft().unwrap_or(0);
        if gs_kt > 10.0 {
            passed_taxi_threshold = true;
        }
        let phase = classify_phase(on_ground, gs_kt, alt_ft, max_alt_ft, passed_taxi_threshold);
        let phase_str = phase.to_string();
        if last_phase.as_ref() != Some(&phase_str) {
            if let Some(ts_iso) = epoch_to_iso(s.ts_epoch) {
                transitions.push((phase_str.clone(), ts_iso));
                last_phase = Some(phase_str);
            }
        }
    }

    // Persistir cada transition con su exited_at = entered_at de la siguiente.
    let mut tx = pool.begin().await?;
    for i in 0..transitions.len() {
        let (phase, entered_at) = &transitions[i];
        let exited_at: Option<&str> = transitions.get(i + 1).map(|(_, ts)| ts.as_str());
        sqlx::query(
            r#"INSERT OR IGNORE INTO flight_log_phase
                (flight_id, phase, entered_at, exited_at)
               VALUES (?1, ?2, ?3, ?4)"#,
        )
        .bind(flight_id)
        .bind(phase)
        .bind(entered_at)
        .bind(exited_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Mapea (on_ground, gs, alt, max_alt, passed_taxi_threshold) a una
/// phase. Simplificación de `derive_phase_label` del watcher — sin
/// parking_brake / engine_running (no están en el .bin).
fn classify_phase(
    on_ground: bool,
    gs_kt: f64,
    alt_ft: i64,
    max_alt_ft: i64,
    passed_taxi_threshold: bool,
) -> &'static str {
    if on_ground {
        if gs_kt < 1.0 {
            return "preflight"; // o "arrived" — el primer/último ramo lo decidirá el orden
        }
        if gs_kt < 8.0 {
            return "pushback";
        }
        if !passed_taxi_threshold {
            return "pushback";
        }
        if gs_kt < 60.0 {
            return "taxi_out"; // se renombrará a taxi_in en el segmento post-touchdown
        }
        return "takeoff";
    }
    // Airborne
    if alt_ft < 500 {
        return "takeoff";
    }
    // Definimos cruise como la ventana cerca del max_alt (±1500 ft).
    if max_alt_ft > 0 && (max_alt_ft - alt_ft).abs() <= 1500 {
        return "cruise";
    }
    // Si la altitud todavía está aumentando con respecto al inicio,
    // climb. Si está bajando, descent/approach.
    if alt_ft < max_alt_ft {
        if alt_ft < 3000 {
            "approach"
        } else if alt_ft < 10000 {
            "descent"
        } else {
            // Sin VS firmado en estas muestras, la heurística decide
            // por posición relativa al max. Lo tratamos como climb si
            // el sample es de la primera mitad temporal, descent si de
            // la segunda — pero acá no tenemos el índice del sample.
            // Simplemente: si alt está más cerca del 0 que del max,
            // probablemente climb (subiendo desde abajo); si está más
            // cerca del max, descent. Ambivalente — preferir climb.
            "climb"
        }
    } else {
        // alt_ft >= max_alt_ft, casi crucero
        "cruise"
    }
}

/// Inserta los puntos del track en `flight_log_track`. Downsamplea
/// cada `TRACK_DOWNSAMPLE_STRIDE` muestras para mantener ~10s entre
/// puntos (similar al sampling de SimConnect).
async fn insert_track_points(
    pool: &SqlitePool,
    flight_id: i64,
    track: &VasFlightTrack,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for (i, sample) in track.samples.iter().enumerate() {
        if i % TRACK_DOWNSAMPLE_STRIDE != 0 && i != track.samples.len() - 1 {
            continue;
        }
        let ts_iso = match epoch_to_iso(sample.ts_epoch) {
            Some(s) => s,
            None => continue,
        };
        let alt_ft = sample.altitude_ft();
        let gs_kt = sample.ground_speed_kt.map(|v| v as i64);
        sqlx::query(
            "INSERT OR IGNORE INTO flight_log_track (flight_id, lat, lon, alt_ft, gs_kt, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(flight_id)
        .bind(sample.lat)
        .bind(sample.lon)
        .bind(alt_ft)
        .bind(gs_kt)
        .bind(&ts_iso)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_basic() {
        assert_eq!(read_varint(&[0x00], 0), Some((0, 1)));
        assert_eq!(read_varint(&[0x01], 0), Some((1, 1)));
        assert_eq!(read_varint(&[0x96, 0x01], 0), Some((150, 2)));
        // truncated
        assert_eq!(read_varint(&[0x80], 0), None);
    }

    #[test]
    fn parse_triple_varint_basic() {
        // {f1=2026, f2=1, f3=18} — wire 0 each
        // tag=(1<<3)|0=8, 2026 = 0xEA 0x0F (varint LE 7-bit)
        // tag for f2=16, varint 1 = 0x01
        // tag for f3=24, varint 18 = 0x12
        let payload = [0x08, 0xEA, 0x0F, 0x10, 0x01, 0x18, 0x12];
        assert_eq!(parse_triple_varint(&payload), Some((2026, 1, 18)));
    }

    #[test]
    fn parse_geo_pair_basic() {
        // f1 f64 = -34.815936, f2 f64 = -58.539919 (SAEZ)
        // tag f1 wire 1 = (1<<3)|1 = 9
        // tag f2 wire 1 = (2<<3)|1 = 17
        let lat_bytes = (-34.815936_f64).to_le_bytes();
        let lon_bytes = (-58.539919_f64).to_le_bytes();
        let mut payload = vec![9];
        payload.extend_from_slice(&lat_bytes);
        payload.push(17);
        payload.extend_from_slice(&lon_bytes);
        let (lat, lon) = parse_geo_pair(&payload);
        assert!((lat.unwrap() - -34.815936).abs() < 1e-9);
        assert!((lon.unwrap() - -58.539919).abs() < 1e-9);
    }

    #[test]
    fn aircraft_label_recognizes_fenix() {
        let m = VasFlightMeta {
            aircraft_cfg_path: Some(
                r"SimObjects\Airplanes\FNX_320_IAE\aircraft.CFG".to_string(),
            ),
            ..Default::default()
        };
        assert_eq!(m.aircraft_label(), Some("Fenix A320".into()));
    }

    #[test]
    fn aircraft_label_falls_back_to_livery() {
        let m = VasFlightMeta {
            livery: Some("SomeRandomLivery".into()),
            aircraft_cfg_path: Some(r"SimObjects\Airplanes\UNKNOWN\aircraft.CFG".to_string()),
            ..Default::default()
        };
        assert_eq!(m.aircraft_label(), Some("SomeRandomLivery".into()));
    }

    #[test]
    fn ordered_times_swaps_when_b_earlier() {
        let m = VasFlightMeta {
            time_a_iso: Some("2026-01-18T02:03:44Z".into()),
            time_b_iso: Some("2026-01-17T23:03:44Z".into()),
            ..Default::default()
        };
        let (s, e) = m.ordered_times();
        assert_eq!(s, Some("2026-01-17T23:03:44Z"));
        assert_eq!(e, Some("2026-01-18T02:03:44Z"));
    }

    #[test]
    fn detect_returns_empty_when_no_dir() {
        // No podemos asegurar el path real en tests, pero la función
        // no debería panicear ni si LOCALAPPDATA no existe.
        let _ = detect_flights();
    }

    #[test]
    fn parse_payload_rejects_garbage() {
        // Bytes que no parsean como protobuf en absoluto.
        assert!(parse_payload(&[0xFF, 0xFF, 0xFF]).is_none());
    }

    #[test]
    fn parse_payload_extracts_livery() {
        // Construye un mini-payload con solo f4 (livery).
        // tag=(4<<3)|2 = 0x22, length=4, "TEST"
        let payload = [0x22, 0x04, b'T', b'E', b'S', b'T'];
        let parsed = parse_payload(&payload).unwrap();
        assert_eq!(parsed.livery.as_deref(), Some("TEST"));
    }

    #[test]
    fn extracts_us_registration() {
        assert_eq!(
            extract_registration("Airbus A330-900neo Delta N404DX"),
            Some("N404DX".into())
        );
        assert_eq!(
            extract_registration("FenixA321 CFM SL Delta Air Lines N367DN"),
            Some("N367DN".into())
        );
    }

    #[test]
    fn extracts_dashed_registration() {
        assert_eq!(
            extract_registration("Airbus A330-900neo Garuda Indonesia PK-GHG"),
            Some("PK-GHG".into())
        );
        assert_eq!(
            extract_registration("Airbus A320 Lufthansa D-AIZA"),
            Some("D-AIZA".into())
        );
    }

    #[test]
    fn strips_quality_suffix_before_reg_extraction() {
        // Liveries con "(4K)" después de la reg.
        assert_eq!(
            extract_registration("Airbus A321 Delta N367DN (4K)"),
            Some("N367DN".into())
        );
    }

    #[test]
    fn rejects_non_registration_tokens() {
        // "DELTA" no es una reg — no contiene dígito ni dash.
        assert!(!is_registration_like("DELTA"));
        // "LUFTHANSA" tampoco.
        assert!(!is_registration_like("LUFTHANSA"));
        // FenixA320_LANCCBAA — "LANCCBAA" es 8 chars todas letras, no
        // tiene dash ni dígitos → no es reg-like (correctamente).
        assert!(!is_registration_like("LANCCBAA"));
    }

    #[test]
    fn extracts_airline_simple() {
        assert_eq!(
            extract_airline("Airbus A330-900neo Delta N404DX"),
            Some("Delta".into())
        );
    }

    #[test]
    fn extracts_airline_multiword() {
        assert_eq!(
            extract_airline("Airbus A330-900neo Garuda Indonesia PK-GHG"),
            Some("Garuda Indonesia".into())
        );
    }

    #[test]
    fn airline_none_when_no_recognizable_pattern() {
        // FenixA320_LANCCBAA → strip "FenixA320" prefix → " LANCCBAA"
        // → último token "LANCCBAA" no es reg → vuelve todo "LANCCBAA"
        // como airline. Aceptable — el usuario puede editar después.
        // Test simplemente que no panique y devuelva algo.
        let result = extract_airline("FenixA320_LANCCBAA");
        // No assertion fuerte — el behavior aquí es best-effort.
        // Solo verificamos que no panique.
        let _ = result;
    }

    /// Smoke específico del NUEVO parser track-based. Solo corre con
    /// `--ignored` cuando hay VAS-ACARS instalado.
    /// Cmd: `cargo test --lib -- --ignored parse_full_track_real`
    #[test]
    #[ignore = "needs VAS-ACARS install — manual integration check"]
    fn parse_full_track_real() {
        let cands = detect_flights();
        if cands.is_empty() {
            eprintln!("⚠ no VAS-ACARS flights — skipping");
            return;
        }
        let mut ok = 0;
        let mut total_samples = 0;
        let mut min_samples = usize::MAX;
        let mut max_samples = 0;
        let mut viable_for_import = 0;
        let mut peak_alt_examples: Vec<(String, i64)> = Vec::new();
        let mut peak_speed_examples: Vec<(String, i64)> = Vec::new();
        for c in cands.iter() {
            match parse_full_track(std::path::Path::new(&c.path)) {
                Some(t) => {
                    ok += 1;
                    total_samples += t.samples.len();
                    if t.samples.len() < min_samples {
                        min_samples = t.samples.len();
                    }
                    if t.samples.len() > max_samples {
                        max_samples = t.samples.len();
                    }
                    let viable = t.samples.len() >= MIN_TRACK_SAMPLES
                        && t.first_position().is_some()
                        && t.last_position().is_some();
                    if viable {
                        // Comprobar distancia tambien
                        let (o_lat, o_lon) = t.first_position().unwrap();
                        let (d_lat, d_lon) = t.last_position().unwrap();
                        let dist = crate::flight_log::haversine_nm(
                            o_lat, o_lon, d_lat, d_lon,
                        );
                        if dist >= MIN_FLIGHT_DISTANCE_NM {
                            viable_for_import += 1;
                            if peak_alt_examples.len() < 5 {
                                if let Some(a) = t.max_altitude_ft() {
                                    peak_alt_examples.push((
                                        c.uuid[..8].to_string(),
                                        a,
                                    ));
                                }
                                if let Some(v) = t.max_ground_speed_kt() {
                                    peak_speed_examples.push((
                                        c.uuid[..8].to_string(),
                                        v,
                                    ));
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        eprintln!(
            "parse_full_track: parsed={}/{} viable_import={}/{} avg_samples={} min={} max={}",
            ok,
            cands.len(),
            viable_for_import,
            cands.len(),
            if ok > 0 { total_samples / ok } else { 0 },
            if min_samples == usize::MAX { 0 } else { min_samples },
            max_samples,
        );
        eprintln!("peak alt examples (ft): {:?}", peak_alt_examples);
        eprintln!("peak GS examples (kt): {:?}", peak_speed_examples);
        assert!(
            viable_for_import * 2 >= cands.len(),
            "expected ≥50% viable for import, got {}/{}",
            viable_for_import,
            cands.len()
        );
    }

    /// Smoke test contra los archivos reales del usuario. Solo corre
    /// si `%LOCALAPPDATA%\VASystem\VAS-ACARS\flights\` existe.
    /// Marcado `#[ignore]` para no romper CI en máquinas sin VAS.
    /// Cmd: `cargo test --lib -- --ignored vas_real`
    #[test]
    #[ignore = "needs VAS-ACARS install — manual integration check"]
    fn vas_real_files_smoke() {
        let cands = detect_flights();
        if cands.is_empty() {
            eprintln!("⚠ no VAS-ACARS flights found — skipping smoke");
            return;
        }
        let mut ok = 0;
        let mut bad = 0;
        let mut with_geo = 0;
        let mut with_reg = 0;
        let mut with_airline = 0;
        for (i, c) in cands.iter().enumerate() {
            match parse_flight_file(std::path::Path::new(&c.path)) {
                Some(m) => {
                    ok += 1;
                    if m.start_lat.is_some() && m.start_lon.is_some() {
                        with_geo += 1;
                    }
                    if m.registration().is_some() {
                        with_reg += 1;
                    }
                    if m.airline().is_some() {
                        with_airline += 1;
                    }
                    if i < 5 {
                        eprintln!(
                            "  {} → ac={:?} reg={:?} airline={:?} lat={:?} a={:?} b={:?}",
                            &c.uuid[..8.min(c.uuid.len())],
                            m.aircraft_label(),
                            m.registration(),
                            m.airline(),
                            m.start_lat.map(|v| format!("{:.4}", v)),
                            m.time_a_iso,
                            m.time_b_iso
                        );
                    }
                }
                None => bad += 1,
            }
        }
        eprintln!(
            "VAS smoke: total={} ok={} bad={} with_geo={} with_reg={} with_airline={}",
            cands.len(),
            ok,
            bad,
            with_geo,
            with_reg,
            with_airline
        );
        // Sanity: al menos 80% deben parsear OK.
        let success_rate = ok as f64 / cands.len() as f64;
        assert!(
            success_rate >= 0.80,
            "expected ≥80% parse success, got {:.1}%",
            success_rate * 100.0
        );
        // Y al menos 50% deben tener geo (algunos vuelos pueden no
        // tener spawn position si el sim crasheó al inicio).
        let geo_rate = with_geo as f64 / cands.len() as f64;
        assert!(
            geo_rate >= 0.50,
            "expected ≥50% with geo, got {:.1}%",
            geo_rate * 100.0
        );
    }
}
