//! GSX INI parser — lee los nombres y coordenadas de los parkings
//! desde los archivos de customization de GSX que viven en el disco.
//!
//! ## Por qué este approach
//!
//! Después de 5 hotfixes consecutivos peleando con SimConnect
//! Facility Data + Client Data Areas, confirmamos empíricamente que:
//!
//!   · La DLL bundled de MSFS 2020 rechaza fields críticos del
//!     TAXI_PARKING (BIAS_Y, LATITUDE) → DEFINITION/DATA_ERROR.
//!   · Los nombres documentados públicos de CDA de GSX (`FSDT_GSX_
//!     AIRCRAFT_DATA`, `FSDT_GSX_MENU`, `FSDT_GSX_PIPE_TO_PLANE`,
//!     `FSDT_GSX_BYPASS_PIN`) devuelven `ILLEGAL_OPERATION` en
//!     `RequestClientData` — no son los nombres reales o no están
//!     activos como CDAs lectura por 3rd-party.
//!
//! En cambio, GSX **escribe en disco** sus archivos de customization
//! por aeropuerto en:
//!
//!   `%APPDATA%\Virtuali\GSX\MSFS\<icao>-<scenery>-...-GSXVDGS.ini`
//!
//! Cada section `[gate a 10]`, `[ramp 5]`, `[ne parking 7]` etc.
//! contiene un campo `this_parking_pos = LAT LON HEADING`. Con eso
//! podemos:
//!
//!   1. Cargar el INI del aeropuerto donde está el avión.
//!   2. Calcular la distancia 2D entre la posición del player y cada
//!      parking del INI.
//!   3. Devolver el más cercano (con un threshold de 100m para no
//!      asumir "estoy en el gate" cuando estás en taxiway).
//!
//! **No depende de SimConnect, no depende de GSX corriendo, no
//! depende de WASM mensajes.** Solo lectura de archivos local —
//! cero fragilidad.

use std::path::{Path, PathBuf};

/// Un parking listado en el INI de un aeropuerto.
#[derive(Debug, Clone)]
pub struct GsxParking {
    /// Nombre legible — "A10", "D1A", "NE Parking 7", "Ramp 5".
    pub name: String,
    pub lat: f64,
    pub lon: f64,
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
/// el patrón `<icao_lower>-<scenery>-...-GSXVDGS.ini` o
/// `<icao_lower>-<scenery>.ini`. Si hay varios candidatos (custom +
/// default + variants), elegimos el **más reciente** asumiendo que
/// es el que el usuario configuró últimamente.
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
        // Match: "katl-..." o "katl_..." o "katl.ini"
        let matches_icao = name_lower.starts_with(&format!("{}-", icao_lower))
            || name_lower.starts_with(&format!("{} ", icao_lower))
            || name_lower.starts_with(&format!("{}_", icao_lower))
            || name_lower == format!("{}.ini", icao_lower);
        if matches_icao {
            candidates.push(path);
        }
    }
    // El INI más recientemente escrito es el activo. Si solo hay uno,
    // lo devolvemos directo.
    candidates.into_iter().max_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    })
}

/// Parsea un INI de GSX y devuelve todos los parkings con
/// posición conocida. Ignora `[general]`, `[none ...]` y secciones
/// sin `this_parking_pos`.
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
    let mut current_pos: Option<(f64, f64)> = None;
    let mut parkings: Vec<GsxParking> = Vec::new();

    // Helper para "flush" la sección actual cuando empieza una nueva
    // o al llegar al final del archivo.
    fn flush(
        section: &mut Option<String>,
        pos: &mut Option<(f64, f64)>,
        parkings: &mut Vec<GsxParking>,
    ) {
        if let (Some(s), Some((lat, lon))) = (section.take(), pos.take()) {
            if let Some(name) = format_section_header_to_name(&s) {
                parkings.push(GsxParking { name, lat, lon });
            }
        } else {
            // Si solo había sección pero no pos, descartamos la sección
            // sin tirar la pos (que es None).
            *section = None;
            *pos = None;
        }
    }

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut current_section, &mut current_pos, &mut parkings);
            current_section = Some(line[1..line.len() - 1].to_string());
            continue;
        }
        // Solo nos interesa la clave `this_parking_pos`
        if let Some(rest) = line.strip_prefix("this_parking_pos") {
            // Trim "=" o " " hasta el valor
            let value = rest
                .trim_start_matches(|c: char| c == '=' || c.is_whitespace())
                .trim();
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(lat), Ok(lon)) = (
                    parts[0].parse::<f64>(),
                    parts[1].parse::<f64>(),
                ) {
                    // Sanity check — descartar coords inválidas.
                    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                        current_pos = Some((lat, lon));
                    }
                }
            }
        }
    }
    // Flush la última section.
    flush(&mut current_section, &mut current_pos, &mut parkings);

    tracing::debug!(
        target: "gsx_parking",
        "parsed {} parkings from {}",
        parkings.len(),
        path.display()
    );
    parkings
}

/// Convierte el section header bruto (lowercase) en el nombre legible
/// que el usuario reconoce. Maneja todos los patrones que GSX usa:
///
///   "gate a 10"       → "A10"
///   "gate d 1a"       → "D1A"
///   "gate t 5"        → "T5"
///   "ramp 5"          → "Ramp 5"
///   "stand 3"         → "Stand 3"
///   "cargo 1"         → "Cargo 1"
///   "dock 2"          → "Dock 2"
///   "mil 4"           → "Military 4"
///   "n parking 6"     → "N Parking 6"
///   "ne parking 7"    → "NE Parking 7"
///   "nw parking 1"    → "NW Parking 1"
///   "parking 12"      → "Parking 12"
///   "none 0"          → None (descartado, es placeholder)
///   "general"         → None (no es parking)
fn format_section_header_to_name(section: &str) -> Option<String> {
    let s = section.trim().to_lowercase();
    if s == "general" || s.starts_with("none") {
        return None;
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    let formatted = match parts.as_slice() {
        // "gate <letter> <num+suffix>" — el último token puede traer
        // letra al final ("1a", "7b"). Lo capitalizamos entero.
        ["gate", letter, num_suffix] if letter.len() <= 2 => {
            format!("{}{}", letter.to_uppercase(), num_suffix.to_uppercase())
        }
        // "ramp <num>" / "stand <num>" / "cargo <num>" / "dock <num>"
        ["ramp", num] => format!("Ramp {}", num),
        ["stand", num] => format!("Stand {}", num),
        ["cargo", num] => format!("Cargo {}", num),
        ["dock", num] => format!("Dock {}", num),
        ["mil", num] => format!("Military {}", num),
        // "<cardinal> parking <num>" — N/S/E/W/NE/NW/SE/SW
        [card, "parking", num]
            if matches!(*card, "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw") =>
        {
            format!("{} Parking {}", card.to_uppercase(), num)
        }
        // "parking <num>" genérico
        ["parking", num] => format!("Parking {}", num),
        // Fallback: capitalizamos la primera letra de cada palabra y
        // dejamos los números sin tocar.
        _ => {
            let mut result = String::with_capacity(s.len());
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    result.push(' ');
                }
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => {
                        result.extend(first.to_uppercase());
                        result.push_str(chars.as_str());
                    }
                    None => {}
                }
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
///   · No hay INI para ese ICAO (aeropuerto sin customization).
///   · El INI está vacío o todos los parkings están a > 200m del
///     player (probablemente el avión está en taxiway / runway, no
///     en un parking).
pub fn find_nearest_parking(icao: &str, lat: f64, lon: f64) -> Option<GsxParking> {
    let gsx_dir = find_gsx_dir()?;
    let ini_path = find_ini_for_airport(&gsx_dir, icao)?;
    let parkings = parse_ini(&ini_path);
    if parkings.is_empty() {
        return None;
    }

    // Distancia 2D simple en grados. Convertimos al final para
    // tener un threshold en metros razonable.
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

    // Convertir distancia 2D grados² a metros aproximados.
    //   1° lat ≈ 111_320 m
    //   1° lon ≈ 111_320 * cos(lat) m
    // Para el threshold no necesitamos precisión, usamos lat solamente.
    let m_per_deg = 111_320.0_f64;
    let dist_m = d2.sqrt() * m_per_deg;
    if dist_m > 200.0 {
        tracing::debug!(
            target: "gsx_parking",
            "{} más cercano '{}' está a {:.0}m del player; > 200m threshold → no asumimos gate",
            icao,
            picked.name,
            dist_m
        );
        return None;
    }
    tracing::info!(
        target: "gsx_parking",
        "{} → '{}' a {:.0}m del player ({}, {}) — GSX INI",
        icao,
        picked.name,
        dist_m,
        lat,
        lon
    );
    Some(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_formatting_covers_all_patterns() {
        assert_eq!(
            format_section_header_to_name("gate a 10"),
            Some("A10".to_string())
        );
        assert_eq!(
            format_section_header_to_name("gate d 1a"),
            Some("D1A".to_string())
        );
        assert_eq!(
            format_section_header_to_name("gate t 5"),
            Some("T5".to_string())
        );
        assert_eq!(
            format_section_header_to_name("ramp 5"),
            Some("Ramp 5".to_string())
        );
        assert_eq!(
            format_section_header_to_name("stand 3"),
            Some("Stand 3".to_string())
        );
        assert_eq!(
            format_section_header_to_name("cargo 1"),
            Some("Cargo 1".to_string())
        );
        assert_eq!(
            format_section_header_to_name("dock 2"),
            Some("Dock 2".to_string())
        );
        assert_eq!(
            format_section_header_to_name("mil 4"),
            Some("Military 4".to_string())
        );
        assert_eq!(
            format_section_header_to_name("ne parking 7"),
            Some("NE Parking 7".to_string())
        );
        assert_eq!(
            format_section_header_to_name("nw parking 1"),
            Some("NW Parking 1".to_string())
        );
        assert_eq!(
            format_section_header_to_name("parking 12"),
            Some("Parking 12".to_string())
        );
        assert_eq!(format_section_header_to_name("none 0"), None);
        assert_eq!(format_section_header_to_name("general"), None);
    }
}
