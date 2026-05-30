//! Parser de `%LOCALAPPDATA%\VASystem\VAS-ACARS\data.db` — el índice
//! maestro de VAS-ACARS con metadata completa por vuelo.
//!
//! `data.db` es una base **bbolt** (magic `0xED0CDAED`, formato Go
//! embedded DB). Dentro de las páginas, cada vuelo se guarda como un
//! mensaje **Protocol Buffers** con esta estructura observada por
//! reverse engineering sobre los 138 records del usuario:
//!
//! ```text
//! Flight {
//!   f2: string         // UUID
//!   f3: varint         // id interno
//!   f4: varint         // id interno
//!   f5: User           // { f2: name, f5: userId }
//!   f6: FlightInfo {
//!     f2: string       // callsign (ej. "DAL115")
//!     f6: Airport      // ORIGEN
//!     f7: Airport      // DESTINO
//!     f9: Airline      // { f2: icao, f3: iata, f4: name }
//!     f10: string      // flight number (ej. "DL115")
//!     f12: string      // numero corto (ej. "115")
//!   }
//!   f7: Aircraft {
//!     f1: string       // registration (matrícula)
//!     f3: AircraftType {
//!       f2: string     // nombre largo (ej. "Airbus A330-941")
//!       f3: AircraftModel {
//!         f1: string   // ICAO type (ej. "A339")
//!         f2: string   // short name (ej. "Airbus A330-900neo")
//!       }
//!     }
//!   }
//!   f21: Gates {
//!     f1: string       // departure_gate (ej. "T4-2A")
//!     f2: string       // arrival_gate (ej. "11A")
//!   }
//! }
//!
//! Airport {
//!   f2: string         // ICAO (ej. "KJFK")
//!   f3: string         // IATA (ej. "JFK")
//!   f4: string         // nombre completo
//!   f6: bytes[10]      // <f1 fixed32 lat><f2 fixed32 lon> en degrees
//!   f7: string         // timezone (ej. "America/New_York")
//! }
//! ```
//!
//! Estrategia: NO parseamos bbolt formalmente (B+tree con páginas) —
//! escaneamos el archivo en busca de UUIDs (regex), y para cada UUID
//! decodificamos el mensaje protobuf que empieza 2 bytes antes (el
//! tag de field 2 + length-byte 0x24 = 36 chars del UUID). Esto es
//! robusto porque cada record cabe en una página de 4KB y empieza
//! con `0x12 0x24` (field 2 wire 2 + length 36).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Resumen estructurado de un vuelo desde `data.db` — todo opcional
/// porque no todos los records tienen todos los campos (vuelos
/// abortados, sin gate asignada, etc.).
#[derive(Debug, Clone, Default)]
pub struct VasFlightSummary {
    pub uuid: String,
    pub callsign: Option<String>,
    pub origin_icao: Option<String>,
    pub origin_iata: Option<String>,
    pub origin_name: Option<String>,
    pub destination_icao: Option<String>,
    pub destination_iata: Option<String>,
    pub destination_name: Option<String>,
    pub airline_icao: Option<String>,
    pub airline_iata: Option<String>,
    pub airline_name: Option<String>,
    pub flight_number: Option<String>,
    pub registration: Option<String>,
    /// ICAO type code (ej. "A339", "B738", "A359")
    pub aircraft_icao_type: Option<String>,
    /// Nombre largo (ej. "Airbus A330-941" — con sub-variant)
    pub aircraft_long_name: Option<String>,
    /// Nombre corto (ej. "Airbus A330-900neo")
    pub aircraft_short_name: Option<String>,
    pub departure_gate: Option<String>,
    pub arrival_gate: Option<String>,
    /// Timestamp **block-out** (push back from origin gate). UNIX
    /// epoch seconds. None si el vuelo nunca pasó del gate.
    pub block_out_epoch: Option<i64>,
    /// Timestamp **touchdown** (wheels-down at destination). None si
    /// nunca aterrizó.
    pub touchdown_epoch: Option<i64>,
    /// Timestamp **block-in** (parked at destination gate). None si
    /// nunca llegó al gate de destino.
    pub block_in_epoch: Option<i64>,
}

impl VasFlightSummary {
    /// True si el vuelo está completo: tiene los 3 timestamps OOOI
    /// (block-out, touchdown, block-in) y la diferencia block-in -
    /// block-out es ≥ 600s (10 min). Esto descarta:
    /// - Vuelos cancelados en gate (no hay block-out)
    /// - Vuelos abortados mid-flight (sin touchdown ni block-in)
    /// - "Vuelos de prueba" cortos del usuario (< 10 min total)
    pub fn is_completed(&self) -> bool {
        let (Some(b_out), Some(td), Some(b_in)) = (
            self.block_out_epoch,
            self.touchdown_epoch,
            self.block_in_epoch,
        ) else {
            return false;
        };
        b_out > 0 && td > b_out && b_in >= td && (b_in - b_out) >= 600
    }

    /// Devuelve el block-to-block time en segundos si está completo.
    pub fn block_to_block_seconds(&self) -> Option<i64> {
        match (self.block_out_epoch, self.block_in_epoch) {
            (Some(a), Some(b)) if b > a => Some(b - a),
            _ => None,
        }
    }

    /// Convierte UNIX epoch a string ISO 8601 UTC ("2026-05-26T16:35:41Z").
    pub fn block_out_iso(&self) -> Option<String> {
        self.block_out_epoch.and_then(epoch_to_iso)
    }
    pub fn block_in_iso(&self) -> Option<String> {
        self.block_in_epoch.and_then(epoch_to_iso)
    }
}

fn epoch_to_iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Path al `data.db` de VAS-ACARS.
pub fn data_db_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        Path::new(&local)
            .join("VASystem")
            .join("VAS-ACARS")
            .join("data.db"),
    )
}

/// Carga el índice completo de vuelos desde `data.db`. Devuelve
/// `HashMap<uuid, VasFlightSummary>`. Si el archivo no existe o
/// está corrupto, devuelve un mapa vacío (no es un error — el
/// usuario simplemente no tiene VAS-ACARS).
///
/// Performance: archivo típico = 524KB, parseo en memoria <100ms.
pub fn load_index() -> HashMap<String, VasFlightSummary> {
    let Some(path) = data_db_path() else {
        return HashMap::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        tracing::debug!(target: "vas_summary", "data.db not readable at {}", path.display());
        return HashMap::new();
    };
    if bytes.len() < 32 {
        return HashMap::new();
    }

    // Sanity: confirmar bbolt magic 0xED0CDAED en offset 16. Si no
    // matchea, no abortamos — solo loggeamos warn (futuras versiones
    // podrían cambiar formato).
    let magic = u32::from_le_bytes(bytes[16..20].try_into().unwrap_or([0; 4]));
    if magic != 0xED0CDAED {
        tracing::warn!(
            target: "vas_summary",
            "data.db magic mismatch: got 0x{:08X}, expected 0xED0CDAED — formato cambió?",
            magic
        );
    }

    let mut index = HashMap::new();
    let mut pos = 0;
    // Cada record empieza con `0x12 0x24` (tag para field 2 wire 2 +
    // length 36 del UUID). Escaneamos esos marcadores.
    while pos + 38 < bytes.len() {
        if bytes[pos] == 0x12 && bytes[pos + 1] == 0x24 {
            // Verifica que los siguientes 36 bytes parecen un UUID
            // (formato `[0-9a-f]{8}-[0-9a-f]{4}-7fdd-[0-9a-f]{4}-[0-9a-f]{12}`).
            let uuid_bytes = &bytes[pos + 2..pos + 38];
            if looks_like_uuid(uuid_bytes) {
                // Toma una ventana de 2KB desde el marcador y parseala.
                // Cada flight record es ~700-1500 bytes en disco —
                // 2KB cubre con margen.
                let end = (pos + 2048).min(bytes.len());
                let window = &bytes[pos..end];
                if let Some(summary) = parse_flight_message(window) {
                    index.insert(summary.uuid.clone(), summary);
                }
                pos += 38; // saltea este record y sigue
                continue;
            }
        }
        pos += 1;
    }

    tracing::info!(
        target: "vas_summary",
        "loaded {} flight summaries from data.db ({}KB)",
        index.len(),
        bytes.len() / 1024
    );
    index
}

/// True si los 36 bytes parecen un UUID VAS (segmentos hex + guiones).
fn looks_like_uuid(b: &[u8]) -> bool {
    if b.len() != 36 {
        return false;
    }
    // Positions of dashes: 8, 13, 18, 23
    if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
        return false;
    }
    // Resto debe ser hex
    for (i, &c) in b.iter().enumerate() {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            continue;
        }
        let is_hex = c.is_ascii_digit() || (b'a'..=b'f').contains(&c);
        if !is_hex {
            return false;
        }
    }
    true
}

// =============================================================================
// Protobuf minimal decoder (similar al de vas_acars.rs pero adaptado)
// =============================================================================

fn read_varint(d: &[u8], p: usize) -> Option<(u64, usize)> {
    let mut val: u64 = 0;
    let mut shift: u32 = 0;
    let mut pos = p;
    while pos < d.len() {
        let b = d[pos];
        pos += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((val, pos));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

/// Parsea el top-level flight message. La ventana contiene tag(0x12) +
/// len(0x24) + uuid + resto. Devuelve None si el mensaje no se puede
/// decodificar (cortado por boundary de página bbolt, etc.).
fn parse_flight_message(window: &[u8]) -> Option<VasFlightSummary> {
    let mut s = VasFlightSummary::default();
    let mut pos = 0;
    let mut fields_extracted = 0u32;

    while pos < window.len() {
        let (key, np) = read_varint(window, pos)?;
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;

        match wire {
            0 => {
                let (_, np) = read_varint(window, pos)?;
                pos = np;
            }
            1 => {
                if pos + 8 > window.len() {
                    break;
                }
                pos += 8;
            }
            2 => {
                let (ln, np) = read_varint(window, pos)?;
                pos = np;
                let ln = ln as usize;
                if pos + ln > window.len() {
                    // Mensaje truncado — devolvemos lo que tengamos hasta ahora.
                    break;
                }
                let chunk = &window[pos..pos + ln];
                pos += ln;
                fields_extracted += 1;

                // (v3.5.0 F2) VAS-ACARS usa números de field
                // duplicados en el mismo record: f4/f5/f6 aparecen
                // PRIMERO como FlightInfo/User/Aircraft (msgs grandes
                // >40 bytes) y DESPUÉS como timestamps OOOI (msgs
                // pequeños de 11-12 bytes, `{f1: seconds, f2: nanos}`).
                // Desambiguamos por tamaño: ≤16 bytes → timestamp,
                // si no → estructura semántica conocida del campo.
                let is_small = chunk.len() <= 16;
                match field {
                    2 => {
                        if let Ok(s_str) = std::str::from_utf8(chunk) {
                            s.uuid = s_str.to_string();
                        }
                    }
                    4 if is_small => {
                        // Block-out timestamp (push back from origin gate).
                        // Solo lo sobreescribimos si parseamos un epoch
                        // razonable; vuelos abortados NO tienen este field.
                        if let Some(ts) = parse_timestamp_seconds(chunk) {
                            s.block_out_epoch = Some(ts);
                        }
                    }
                    5 if is_small => {
                        // Touchdown timestamp.
                        if let Some(ts) = parse_timestamp_seconds(chunk) {
                            s.touchdown_epoch = Some(ts);
                        }
                    }
                    6 if is_small => {
                        // Block-in timestamp (parked at dest gate).
                        if let Some(ts) = parse_timestamp_seconds(chunk) {
                            s.block_in_epoch = Some(ts);
                        }
                    }
                    6 => parse_flight_info(chunk, &mut s),
                    7 => parse_aircraft(chunk, &mut s),
                    21 => parse_gates(chunk, &mut s),
                    _ => {}
                }
            }
            5 => {
                if pos + 4 > window.len() {
                    break;
                }
                pos += 4;
            }
            _ => break,
        }
    }

    if fields_extracted > 0 && !s.uuid.is_empty() {
        Some(s)
    } else {
        None
    }
}

/// Decodifica el sub-message `f6: FlightInfo` y rellena los campos
/// `callsign`, `origin_*`, `destination_*`, `airline_*`, `flight_number`.
fn parse_flight_info(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    2 => out.callsign = utf8(chunk),
                    6 => parse_airport(chunk, out, true), // origin
                    7 => parse_airport(chunk, out, false), // destination
                    9 => parse_airline(chunk, out),
                    10 => out.flight_number = utf8(chunk),
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Decodifica un sub-message Airport. `is_origin` indica si los
/// campos van a `origin_*` o `destination_*`.
fn parse_airport(payload: &[u8], out: &mut VasFlightSummary, is_origin: bool) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                let s = utf8(chunk);
                match field {
                    2 => {
                        if is_origin {
                            out.origin_icao = s;
                        } else {
                            out.destination_icao = s;
                        }
                    }
                    3 => {
                        if is_origin {
                            out.origin_iata = s;
                        } else {
                            out.destination_iata = s;
                        }
                    }
                    4 => {
                        if is_origin {
                            out.origin_name = s;
                        } else {
                            out.destination_name = s;
                        }
                    }
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Decodifica el sub-message Airline.
fn parse_airline(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    2 => out.airline_icao = utf8(chunk),
                    3 => out.airline_iata = utf8(chunk),
                    4 => out.airline_name = utf8(chunk),
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Decodifica el sub-message Aircraft → registration + aircraft type.
fn parse_aircraft(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    1 => {
                        // Registration — preferimos f1 sobre f2 (suelen ser iguales).
                        if out.registration.is_none() {
                            out.registration = utf8(chunk);
                        }
                    }
                    3 => parse_aircraft_type(chunk, out),
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Sub-message AircraftType { f2: long_name, f3: AircraftModel }.
fn parse_aircraft_type(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    2 => out.aircraft_long_name = utf8(chunk),
                    3 => {
                        // AircraftModel { f1: icao_type, f2: short_name }
                        parse_aircraft_model(chunk, out);
                    }
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

fn parse_aircraft_model(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    1 => out.aircraft_icao_type = utf8(chunk),
                    2 => out.aircraft_short_name = utf8(chunk),
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Sub-message Gates { f1: departure_gate, f2: arrival_gate }.
fn parse_gates(payload: &[u8], out: &mut VasFlightSummary) {
    let mut pos = 0;
    while pos < payload.len() {
        let Some((key, np)) = read_varint(payload, pos) else {
            return;
        };
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let Some((_, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
            }
            1 => pos += 8,
            2 => {
                let Some((ln, np)) = read_varint(payload, pos) else {
                    return;
                };
                pos = np;
                let ln = ln as usize;
                if pos + ln > payload.len() {
                    return;
                }
                let chunk = &payload[pos..pos + ln];
                pos += ln;
                match field {
                    1 => out.departure_gate = utf8(chunk),
                    2 => out.arrival_gate = utf8(chunk),
                    _ => {}
                }
            }
            5 => pos += 4,
            _ => return,
        }
    }
}

/// Convierte bytes a String si son UTF-8 válido; None si no.
fn utf8(b: &[u8]) -> Option<String> {
    std::str::from_utf8(b).ok().map(|s| s.to_string())
}

/// Decodifica un Timestamp protobuf `{f1: seconds, f2: nanos}` y
/// devuelve los **segundos epoch** si están en rango razonable
/// (1e9 = año 2001, 4e9 ≈ año 2096). None si la estructura no
/// matchea — esto sirve como filtro para descartar mensajes que
/// NO son timestamps (User msg con `f2: string "Daniel Espinal"`
/// que pasaría `wire 0 varint` test pero con valor pequeño).
fn parse_timestamp_seconds(payload: &[u8]) -> Option<i64> {
    let mut pos = 0;
    let mut seconds: Option<i64> = None;
    while pos < payload.len() {
        let (key, np) = read_varint(payload, pos)?;
        pos = np;
        let field = (key >> 3) as u32;
        let wire = (key & 0x7) as u8;
        match wire {
            0 => {
                let (v, np) = read_varint(payload, pos)?;
                pos = np;
                if field == 1 {
                    // Sanity: epoch entre año 2001 (1e9) y año 2096 (4e9).
                    if (1_000_000_000..4_000_000_000).contains(&v) {
                        seconds = Some(v as i64);
                    } else {
                        // Valor fuera de rango — no es un timestamp.
                        return None;
                    }
                }
            }
            1 => {
                if pos + 8 > payload.len() {
                    return seconds;
                }
                pos += 8;
            }
            2 => {
                // Si hay un field length-delimited es una string/sub-msg
                // — el mensaje NO es un Timestamp puro. Aborta.
                return None;
            }
            5 => {
                if pos + 4 > payload.len() {
                    return seconds;
                }
                pos += 4;
            }
            _ => return None,
        }
    }
    seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_valid_uuid() {
        assert!(looks_like_uuid(
            b"019bf7ad-e887-7fdd-99d1-9517870d9b74"
        ));
        assert!(!looks_like_uuid(b"019bf7ad-e887-7fdd-99d1-9517870d9b7"));
        // wrong dashes
        assert!(!looks_like_uuid(
            b"019bf7adee887x7fddx99d1x9517870d9b74"
        ));
    }

    #[test]
    fn parses_minimal_flight_message() {
        // Construye un mensaje protobuf con solo UUID (f2) + flight_info (f6) con callsign.
        let uuid = b"019bf7ad-e887-7fdd-99d1-9517870d9b74";

        let mut msg = Vec::new();
        // f2 wire 2, length 36, UUID
        msg.push(0x12);
        msg.push(36);
        msg.extend_from_slice(uuid);
        // f6 wire 2, length 8, FlightInfo { f2: "ABC123" }
        msg.push(0x32); // (6<<3)|2 = 50 = 0x32
        msg.push(0x08); // length 8 = 2 (header) + 6 (string)
        msg.push(0x12);
        msg.push(0x06);
        msg.extend_from_slice(b"ABC123");

        let parsed = parse_flight_message(&msg).unwrap();
        assert_eq!(parsed.uuid, "019bf7ad-e887-7fdd-99d1-9517870d9b74");
        assert_eq!(parsed.callsign.as_deref(), Some("ABC123"));
    }

    /// Smoke test contra el data.db real del usuario.
    #[test]
    #[ignore = "needs VAS-ACARS install"]
    fn loads_real_data_db() {
        let idx = load_index();
        eprintln!("Loaded {} summaries", idx.len());
        if idx.is_empty() {
            return;
        }
        let mut with_dest = 0;
        let mut with_reg = 0;
        let mut with_airline = 0;
        let mut with_gates = 0;
        let mut samples = 0;
        for (uuid, s) in &idx {
            if s.destination_icao.is_some() {
                with_dest += 1;
            }
            if s.registration.is_some() {
                with_reg += 1;
            }
            if s.airline_name.is_some() {
                with_airline += 1;
            }
            if s.departure_gate.is_some() || s.arrival_gate.is_some() {
                with_gates += 1;
            }
            if samples < 5 {
                eprintln!(
                    "  {} → {} → {} | {} {} | reg={:?} aircraft={:?} gates=({:?},{:?})",
                    &uuid[..8],
                    s.origin_icao.as_deref().unwrap_or("?"),
                    s.destination_icao.as_deref().unwrap_or("?"),
                    s.airline_name.as_deref().unwrap_or(""),
                    s.flight_number.as_deref().unwrap_or(""),
                    s.registration,
                    s.aircraft_icao_type,
                    s.departure_gate,
                    s.arrival_gate
                );
                samples += 1;
            }
        }
        eprintln!(
            "Total={} with_dest={} with_reg={} with_airline={} with_gates={}",
            idx.len(),
            with_dest,
            with_reg,
            with_airline,
            with_gates
        );
        // Sanity: al menos 50% deben tener destino
        assert!(
            with_dest * 2 >= idx.len(),
            "expected ≥50% with destination, got {}/{}",
            with_dest,
            idx.len()
        );
    }
}
