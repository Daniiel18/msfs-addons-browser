//! GSX INI parser — lee los nombres y coordenadas de los parkings
//! desde los archivos de customization de GSX que viven en el disco.
//!
//! ## Por qué este approach
//!
//! Después de 6 hotfixes peleando con SimConnect Facility Data + GSX
//! Client Data Areas (la DLL de MSFS 2020 rechaza fields claves +
//! los CDA names públicos no son los reales), confirmamos
//! empíricamente que el camino estable es leer los INIs en disco
//! que GSX escribe por aeropuerto en:
//!
//!   `%APPDATA%\Virtuali\GSX\MSFS\<icao>-<scenery>-...-GSXVDGS.ini`
//!
//! ## Schema descubierto por survey empírico
//!
//! Cada section `[<header>]` define un parking. Headers vistos en
//! INIs reales del usuario (sample 8 aeropuertos: KATL, EDDF, EDDM,
//! CYUL, EGLL, EHAM, CYYZ, FMMI):
//!
//!   - `[gate <letter> <num>[suffix]]`  → "A10", "D1A"
//!   - `[gate <num>]`         (sin letra) → "Gate 12"
//!   - `[gate <letter> <num>_mars_]` → "<letter><num>" (sufijo MARS interno
//!                                     de GSX — Multiple Aircraft Ramp
//!                                     System; ignoramos el `_mars_`)
//!   - `[gate <num>_mars_]`   → "Gate <num>"
//!   - `[gate]` / `[gate ]`   → descartado (placeholder vacío)
//!   - `[<cardinal> parking <num>]`  → "NE Parking 7"
//!   - `[parking <num>]`      → "Parking 12"
//!   - `[parking]` / `[parking ]` → descartado
//!   - `[ramp <num>]`         → "Ramp 5"
//!   - `[stand <num>]`        → "Stand 3"
//!   - `[cargo <num>]`        → "Cargo 1"
//!   - `[dock <num>]`         → "Dock 2"
//!   - `[mil <num>]`          → "Military 4"
//!   - `[pad <num>]`          → "Pad 1"        (helipads)
//!   - `[pad]` / `[pad ]`     → descartado
//!   - `[none <num>]`         → descartado (placeholder GSX)
//!   - `[general]`            → descartado (header del archivo)
//!
//! ## Fields de posición — priority chain
//!
//! Cada section puede tener varios fields con coordenadas. Survey
//! mostró que en TODOS los INIs el más confiable + más común es
//! `parkingsystem_stopposition` (la coord exacta donde el Safedock
//! detiene al avión — es la posición real del avión cuando está
//! parado en el gate). Fallback chain en orden decreciente de
//! precisión:
//!
//!   1. `parkingsystem_stopposition` — coord exacta de parking
//!   2. `this_parking_pos`           — centro del parking
//!   3. `parkingsystem_objectposition` — donde está el Safedock visual
//!   4. `pushback_pos`               — donde se ubica el truck (offset ~10m)
//!
//! Si la section no tiene ninguno de estos, la descartamos.
//!
//! ## No depende de
//!
//!   · SimConnect Facility Data API (buggy en MSFS 2020 DLL)
//!   · GSX Client Data Areas (nombres no públicos)
//!   · WASM modules / GSX SDK private
//!   · MSFS estar corriendo
//!
//! Solo lectura de archivos local + math. Cero fragilidad.

use std::path::{Path, PathBuf};

/// Un parking listado en el INI de un aeropuerto.
#[derive(Debug, Clone)]
pub struct GsxParking {
    /// Nombre legible — "A10", "D1A", "NE Parking 7", "Ramp 5".
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Qué field del INI nos dio la coord, para diagnóstico.
    pub source_field: &'static str,
}

/// Localiza la carpeta de customizations de GSX. Sólo Windows —
/// `%APPDATA%\Virtuali\GSX\MSFS`. En otros OS devuelve None.
pub fn find_gsx_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let path = PathBuf::from(appdata)
        .join("Virtuali")
        .join("GSX")
        .join("MSFS");
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

/// Busca el INI más adecuado para un ICAO dado. Los archivos siguen
/// el patrón `<icao_lower>-<scenery>-...-GSXVDGS.ini` (con variaciones
/// de separador). Si hay varios candidatos (custom + default +
/// variants), elegimos el **más reciente** asumiendo que es el que
/// el usuario configuró últimamente.
pub fn find_ini_for_airport(gsx_dir: &Path, icao: &str) -> Option<PathBuf> {
    let icao_lower = icao.to_lowercase();
    let entries = std::fs::read_dir(gsx_dir).ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let name_lower = name.to_lowercase();
        if !name_lower.ends_with(".ini") {
            continue;
        }
        let matches_icao = name_lower.starts_with(&format!("{}-", icao_lower))
            || name_lower.starts_with(&format!("{} ", icao_lower))
            || name_lower.starts_with(&format!("{}_", icao_lower))
            || name_lower == format!("{}.ini", icao_lower);
        if matches_icao {
            candidates.push(path);
        }
    }
    candidates.into_iter().max_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    })
}

/// Lee dos floats desde una línea tipo `key = LAT LON [HEADING] [...]`.
/// El valor puede tener 2-4 números separados por espacio; sólo nos
/// importan los primeros dos. Sanitiza rangos lat/lon válidos.
fn parse_pos_value(value: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let lat = parts[0].parse::<f64>().ok()?;
    let lon = parts[1].parse::<f64>().ok()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

/// Acumulador de fields de posición candidatos para una section,
/// con priority chain. Solo retorna el field más preciso disponible.
#[derive(Default)]
struct PositionCandidates {
    stop_position: Option<(f64, f64)>,
    this_parking_pos: Option<(f64, f64)>,
    object_position: Option<(f64, f64)>,
    pushback_pos: Option<(f64, f64)>,
}

impl PositionCandidates {
    /// Devuelve la mejor coord disponible + qué field la dio.
    fn pick(self) -> Option<((f64, f64), &'static str)> {
        self.stop_position
            .map(|p| (p, "stop_position"))
            .or_else(|| self.this_parking_pos.map(|p| (p, "this_parking_pos")))
            .or_else(|| self.object_position.map(|p| (p, "object_position")))
            .or_else(|| self.pushback_pos.map(|p| (p, "pushback_pos")))
    }
}

/// Parsea un INI de GSX y devuelve todos los parkings con
/// posición conocida.
pub fn parse_ini(path: &Path) -> Vec<GsxParking> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                target: "gsx_parking",
                "no pude leer {}: {}",
                path.display(),
                e
            );
            return Vec::new();
        }
    };
    let mut current_section: Option<String> = None;
    let mut current_pos = PositionCandidates::default();
    let mut parkings: Vec<GsxParking> = Vec::new();

    fn flush(
        section: &mut Option<String>,
        pos: &mut PositionCandidates,
        parkings: &mut Vec<GsxParking>,
    ) {
        if let Some(s) = section.take() {
            if let Some(name) = format_section_header_to_name(&s) {
                let pc = std::mem::take(pos);
                if let Some(((lat, lon), field)) = pc.pick() {
                    parkings.push(GsxParking {
                        name,
                        lat,
                        lon,
                        source_field: field,
                    });
                }
            } else {
                // Section no es un parking real — descartar pos
                *pos = PositionCandidates::default();
            }
        }
    }

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut current_section, &mut current_pos, &mut parkings);
            current_section = Some(line[1..line.len() - 1].to_string());
            continue;
        }
        // Parsear los 4 fields que nos interesan. Strict prefix match
        // con '=' o whitespace después para no confundir
        // parkingsystem_stopposition con parkingsystem_objectposition.
        let (field_key, value) = match line.find('=') {
            Some(eq) => (line[..eq].trim(), line[eq + 1..].trim()),
            None => continue,
        };
        match field_key {
            "parkingsystem_stopposition" => {
                if let Some(p) = parse_pos_value(value) {
                    current_pos.stop_position = Some(p);
                }
            }
            "this_parking_pos" => {
                if let Some(p) = parse_pos_value(value) {
                    current_pos.this_parking_pos = Some(p);
                }
            }
            "parkingsystem_objectposition" => {
                if let Some(p) = parse_pos_value(value) {
                    current_pos.object_position = Some(p);
                }
            }
            "pushback_pos" => {
                if let Some(p) = parse_pos_value(value) {
                    current_pos.pushback_pos = Some(p);
                }
            }
            _ => {}
        }
    }
    flush(&mut current_section, &mut current_pos, &mut parkings);

    tracing::debug!(
        target: "gsx_parking",
        "parsed {} parkings from {}",
        parkings.len(),
        path.display()
    );
    parkings
}

/// Convierte el section header bruto (lowercase) en el nombre
/// legible que el usuario reconoce. Cobertura empírica del survey
/// sobre 8 INIs distintos:
///
///   "gate a 10"            → "A10"
///   "gate d 1a"            → "D1A"
///   "gate t 5"             → "T5"
///   "gate a 12_mars_"      → "A12"      (sufijo _mars_ ignorado)
///   "gate 5_mars_"         → "Gate 5"
///   "gate 12"              → "Gate 12"  (sin letra)
///   "gate"                 → None       (placeholder vacío)
///   "ramp 5"               → "Ramp 5"
///   "stand 3"              → "Stand 3"
///   "cargo 1"              → "Cargo 1"
///   "dock 2"               → "Dock 2"
///   "mil 4"                → "Military 4"
///   "pad 1"                → "Pad 1"
///   "pad"                  → None
///   "n parking 6"          → "N Parking 6"
///   "ne parking 7"         → "NE Parking 7"
///   "nw parking 1"         → "NW Parking 1"
///   "parking 12"           → "Parking 12"
///   "parking"              → None
///   "none 0"               → None
///   "general"              → None
fn format_section_header_to_name(section: &str) -> Option<String> {
    let s = section.trim().to_lowercase();
    if s.is_empty() || s == "general" || s.starts_with("none") {
        return None;
    }
    let parts: Vec<&str> = s.split_whitespace().collect();

    // Helper para limpiar el sufijo _mars_ (Multiple Aircraft Ramp
    // System) que GSX usa como flag interno. El name visible al user
    // es el mismo, sin el sufijo.
    fn strip_mars(s: &str) -> &str {
        s.trim_end_matches("_mars_")
            .trim_end_matches("_mars")
    }

    let formatted = match parts.as_slice() {
        // Placeholders vacíos
        ["gate"] | ["parking"] | ["pad"] | ["ramp"] | ["stand"] => return None,

        // "gate <letter> <num+suffix>" → "A10" / "D1A" / "A12" (de mars)
        ["gate", letter, num_suffix] if letter.len() <= 2 => {
            let ns = strip_mars(num_suffix);
            if ns.is_empty() {
                return None;
            }
            format!("{}{}", letter.to_uppercase(), ns.to_uppercase())
        }
        // "gate <num>" (sin letra) → "Gate 12"
        ["gate", num] => {
            let n = strip_mars(num);
            if n.is_empty() {
                return None;
            }
            format!("Gate {}", n)
        }

        // "ramp <num>" / "stand <num>" / "cargo <num>" / "dock <num>"
        ["ramp", num] => format!("Ramp {}", strip_mars(num)),
        ["stand", num] => format!("Stand {}", strip_mars(num)),
        ["cargo", num] => format!("Cargo {}", strip_mars(num)),
        ["dock", num] => format!("Dock {}", strip_mars(num)),
        ["mil", num] => format!("Military {}", strip_mars(num)),
        ["pad", num] => format!("Pad {}", strip_mars(num)),

        // "<cardinal> parking <num>" — N/S/E/W/NE/NW/SE/SW
        [card, "parking", num]
            if matches!(*card, "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw") =>
        {
            format!("{} Parking {}", card.to_uppercase(), strip_mars(num))
        }

        // "parking <num>" genérico
        ["parking", num] => format!("Parking {}", strip_mars(num)),

        // Fallback: capitalizamos la primera letra de cada palabra
        _ => {
            let mut result = String::with_capacity(s.len());
            for (i, part) in parts.iter().enumerate() {
                let cleaned = strip_mars(part);
                if cleaned.is_empty() {
                    continue;
                }
                if !result.is_empty() && i > 0 {
                    result.push(' ');
                }
                let mut chars = cleaned.chars();
                if let Some(first) = chars.next() {
                    result.extend(first.to_uppercase());
                    result.push_str(chars.as_str());
                }
            }
            if result.is_empty() {
                return None;
            }
            result
        }
    };
    Some(formatted)
}

/// Busca el parking más cercano a `(lat, lon)` dentro del INI del
/// aeropuerto `icao`. Devuelve `None` si:
///
///   · No hay carpeta `%APPDATA%\Virtuali\GSX\MSFS` (GSX no instalado).
///   · No hay INI para ese ICAO.
///   · Todos los parkings están a > 75m del player (probablemente en
///     taxiway / runway, no en parking).
///
/// El threshold de 75m es ajustado vs el v3.4.8 que era 200m: GSX
/// gates son de ~25-30m de radio, así que cualquier cosa más allá
/// de 75m del centro del parking ya no es "estoy en este gate".
pub fn find_nearest_parking(icao: &str, lat: f64, lon: f64) -> Option<GsxParking> {
    let gsx_dir = find_gsx_dir()?;
    let ini_path = find_ini_for_airport(&gsx_dir, icao)?;
    let parkings = parse_ini(&ini_path);
    if parkings.is_empty() {
        tracing::debug!(
            target: "gsx_parking",
            "{} → INI tiene 0 parkings con posición conocida ({})",
            icao,
            ini_path.display()
        );
        return None;
    }

    let mut best: Option<(f64, GsxParking)> = None;
    for p in parkings {
        let dlat = p.lat - lat;
        let dlon = p.lon - lon;
        let d = dlat * dlat + dlon * dlon;
        if best.as_ref().map(|(b, _)| d < *b).unwrap_or(true) {
            best = Some((d, p));
        }
    }
    let (d2, picked) = best?;

    let m_per_deg = 111_320.0_f64;
    let dist_m = d2.sqrt() * m_per_deg;
    if dist_m > 75.0 {
        tracing::debug!(
            target: "gsx_parking",
            "{} más cercano '{}' a {:.0}m (> 75m); no asumimos gate",
            icao,
            picked.name,
            dist_m
        );
        return None;
    }
    tracing::info!(
        target: "gsx_parking",
        "{} → '{}' a {:.1}m del player ({:.6}, {:.6}) [field={}]",
        icao,
        picked.name,
        dist_m,
        lat,
        lon,
        picked.source_field
    );
    Some(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_format_basics() {
        assert_eq!(format_section_header_to_name("gate a 10"), Some("A10".to_string()));
        assert_eq!(format_section_header_to_name("gate d 1a"), Some("D1A".to_string()));
        assert_eq!(format_section_header_to_name("gate t 5"), Some("T5".to_string()));
        assert_eq!(format_section_header_to_name("ramp 5"), Some("Ramp 5".to_string()));
        assert_eq!(format_section_header_to_name("stand 3"), Some("Stand 3".to_string()));
        assert_eq!(format_section_header_to_name("cargo 1"), Some("Cargo 1".to_string()));
        assert_eq!(format_section_header_to_name("dock 2"), Some("Dock 2".to_string()));
        assert_eq!(format_section_header_to_name("mil 4"), Some("Military 4".to_string()));
        assert_eq!(format_section_header_to_name("pad 1"), Some("Pad 1".to_string()));
    }

    #[test]
    fn header_format_cardinals() {
        assert_eq!(format_section_header_to_name("n parking 6"), Some("N Parking 6".to_string()));
        assert_eq!(format_section_header_to_name("ne parking 7"), Some("NE Parking 7".to_string()));
        assert_eq!(format_section_header_to_name("nw parking 1"), Some("NW Parking 1".to_string()));
        assert_eq!(format_section_header_to_name("sw parking 99"), Some("SW Parking 99".to_string()));
        assert_eq!(format_section_header_to_name("parking 12"), Some("Parking 12".to_string()));
    }

    #[test]
    fn header_format_gate_without_letter() {
        // [gate 12] (sin letra, muy común — 436 ocurrencias en survey)
        assert_eq!(format_section_header_to_name("gate 12"), Some("Gate 12".to_string()));
        assert_eq!(format_section_header_to_name("gate 99"), Some("Gate 99".to_string()));
    }

    #[test]
    fn header_format_mars_suffix() {
        // _mars_ se ignora — el name visible es el mismo
        assert_eq!(format_section_header_to_name("gate a 12_mars_"), Some("A12".to_string()));
        assert_eq!(format_section_header_to_name("gate b 5_mars"), Some("B5".to_string()));
        assert_eq!(format_section_header_to_name("gate 5_mars_"), Some("Gate 5".to_string()));
    }

    #[test]
    fn header_format_discards() {
        assert_eq!(format_section_header_to_name("general"), None);
        assert_eq!(format_section_header_to_name("none 0"), None);
        assert_eq!(format_section_header_to_name("none"), None);
        assert_eq!(format_section_header_to_name("gate"), None);
        assert_eq!(format_section_header_to_name("parking"), None);
        assert_eq!(format_section_header_to_name("pad"), None);
        assert_eq!(format_section_header_to_name("ramp"), None);
        assert_eq!(format_section_header_to_name(""), None);
    }

    #[test]
    fn parse_pos_value_basic() {
        assert_eq!(
            parse_pos_value("33.6389763280749 -84.439785182476 89.97"),
            Some((33.6389763280749, -84.439785182476))
        );
        assert_eq!(
            parse_pos_value("33.6389763280749 -84.439785182476"),
            Some((33.6389763280749, -84.439785182476))
        );
        assert_eq!(parse_pos_value("33.6"), None);
        assert_eq!(parse_pos_value(""), None);
        assert_eq!(parse_pos_value("999 999"), None); // fuera de rango
    }

    #[test]
    fn position_priority_chain() {
        // stop_position gana sobre los demás
        let mut pc = PositionCandidates::default();
        pc.this_parking_pos = Some((10.0, 20.0));
        pc.stop_position = Some((30.0, 40.0));
        pc.pushback_pos = Some((50.0, 60.0));
        assert_eq!(pc.pick(), Some(((30.0, 40.0), "stop_position")));

        // sin stop_position, gana this_parking_pos
        let mut pc = PositionCandidates::default();
        pc.this_parking_pos = Some((10.0, 20.0));
        pc.pushback_pos = Some((50.0, 60.0));
        assert_eq!(pc.pick(), Some(((10.0, 20.0), "this_parking_pos")));

        // sin stop ni parking, gana object_position
        let mut pc = PositionCandidates::default();
        pc.object_position = Some((10.0, 20.0));
        pc.pushback_pos = Some((50.0, 60.0));
        assert_eq!(pc.pick(), Some(((10.0, 20.0), "object_position")));

        // sin nada, None
        let pc = PositionCandidates::default();
        assert_eq!(pc.pick(), None);

        // solo pushback_pos (caso E26 del usuario)
        let mut pc = PositionCandidates::default();
        pc.pushback_pos = Some((33.6412681488914, -84.4259597340044));
        assert_eq!(
            pc.pick(),
            Some(((33.6412681488914, -84.4259597340044), "pushback_pos"))
        );
    }
}
