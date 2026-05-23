//! Escáner de **liveries de aircraft addons** instaladas en Community.
//!
//! Originalmente sólo PMDG (v3.0.0). En v3.1.0 generalizado a cubrir
//! cualquier vendor con la convención MSFS estándar:
//!   · `{vendor}-aircraft-{model}` (base aircraft con todas las
//!     variantes adentro — patrón Fenix, iniBuilds, FlyByWire…)
//!   · `{vendor}-aircraft-{model}-liveries` (paquete dedicado de
//!     liveries — patrón PMDG)
//!
//! Estructura común dentro de cada paquete:
//!   `{paquete}/SimObjects/Airplanes/{Aircraft Title}/aircraft.cfg`
//!
//! El `aircraft.cfg` define una o más [fltsim.N] sections, una por
//! variante. Campos relevantes que parseamos:
//!   · title          → nombre canónico de la variante.
//!   · ui_variation   → nombre amigable en el sim.
//!   · atc_id         → tail number / matrícula.
//!   · atc_airline    → operador (United, Lufthansa…).
//!   · texture        → subcarpeta del livery.
//!   · ui_thumbnailfile → ruta relativa del thumbnail (DDS/PNG/JPG).
//!
//! Vendors reconocidos (extensible):
//!   · pmdg, fnx (Fenix), inibuilds, fbw (FlyByWire), aerosoft,
//!     leonardo, headwind, dc-designs, justflight…
//!
//! Esta operación es síncrona y bloqueante porque la carpeta vive
//! en SSD local. En el comando lo envolvemos en `spawn_blocking`.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Una livery individual instalada. Se mapea 1:1 a una sección
/// `[fltsim.N]` del `aircraft.cfg`. (Mantengo el nombre `PmdgLivery`
/// por retrocompat con el frontend — el campo `vendor` distingue
/// PMDG / Fenix / etc.)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PmdgLivery {
    /// Vendor del addon: "pmdg" | "fnx" | "inibuilds" | "fbw" | …
    pub vendor: String,
    /// Modelo normalizado: "77er", "77w", "320", "a350", "320neo", etc.
    pub model: String,
    /// Nombre del paquete tal como aparece en Community
    /// (ej. "pmdg-aircraft-77er-liveries", "fnx-aircraft-320").
    pub package_folder: String,
    /// `title` de la sección — único dentro del aircraft.cfg.
    pub title: String,
    /// `ui_variation` o `ui_type` — string amigable de la UI MSFS.
    pub variation: Option<String>,
    /// `atc_id` — registro / tail number (ej. "N12345", "G-VIIA").
    pub tail_number: Option<String>,
    /// `atc_airline` — operador en plain text (ej. "United Airlines").
    pub airline: Option<String>,
    /// `texture` — subcarpeta del livery dentro del paquete.
    pub texture: Option<String>,
    /// Data URL del thumbnail (PNG/JPG → base64). DDS no se soporta
    /// porque WebView2 no las renderiza. Si el livery sólo trae .dds
    /// devolvemos `None` y la UI muestra un placeholder.
    pub thumbnail_data_url: Option<String>,
}

/// Lista de prefijos de vendor reconocidos. Cualquier carpeta de
/// Community cuyo nombre matchea `{vendor}-aircraft-…` (case
/// insensitive) entra al scanner. Extensible — añadir nuevos vendors
/// es trivial.
const KNOWN_VENDORS: &[&str] = &[
    "pmdg",
    "fnx",      // Fenix Simulations
    "inibuilds",
    "fbw",      // FlyByWire
    "aerosoft",
    "leonardo",
    "headwind",
    "dc-designs",
    "justflight",
    "asobo",    // shouldn't show — pero por si acaso
];

/// Recorre la carpeta Community y devuelve TODAS las liveries de
/// aircraft addons detectadas en cualquier paquete con prefijo de
/// vendor reconocido.
///
/// **Robustez**:
///   · Si un paquete tiene varios subdirs `SimObjects/Airplanes/*`,
///     parseamos el `aircraft.cfg` de cada uno y mergeamos.
///   · Si un aircraft.cfg está mal-formado / inválido UTF-8, lo
///     logueamos y pasamos al siguiente.
///   · Las thumbnails ausentes no rompen la entrada — devolvemos
///     `thumbnail_data_url = None`.
pub fn scan_pmdg_liveries(community_path: &Path) -> anyhow::Result<Vec<PmdgLivery>> {
    let mut liveries = Vec::new();
    if !community_path.is_dir() {
        anyhow::bail!(
            "Community no es un directorio: {}",
            community_path.display()
        );
    }

    let entries = std::fs::read_dir(community_path)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let lower = folder_name.to_lowercase();

        // Detectamos vendor + modelo. Patterns soportados:
        //   {vendor}-aircraft-{model}-liveries  (PMDG style)
        //   {vendor}-aircraft-{model}           (Fenix/iniBuilds/FBW style)
        let Some((vendor, model)) = detect_vendor_model(&lower) else {
            continue;
        };

        // Cada paquete contiene SimObjects/Airplanes/* — uno o más
        // subdirs. Iteramos todos.
        let airplanes_dir = path.join("SimObjects").join("Airplanes");
        if !airplanes_dir.is_dir() {
            tracing::debug!(
                target: "aircraft_liveries",
                "{}: sin SimObjects/Airplanes — skip",
                folder_name
            );
            continue;
        }
        let plane_entries = match std::fs::read_dir(&airplanes_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    target: "aircraft_liveries",
                    "{}: no se pudo leer SimObjects/Airplanes: {}",
                    folder_name, e
                );
                continue;
            }
        };
        for plane_entry in plane_entries.flatten() {
            let plane_dir = plane_entry.path();
            if !plane_dir.is_dir() {
                continue;
            }
            let cfg_path = plane_dir.join("aircraft.cfg");
            if !cfg_path.is_file() {
                continue;
            }
            match parse_aircraft_cfg(&cfg_path, &plane_dir) {
                Ok(found) => {
                    for mut liv in found {
                        liv.vendor = vendor.clone();
                        liv.model = model.clone();
                        liv.package_folder = folder_name.clone();
                        liveries.push(liv);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "aircraft_liveries",
                        "{}: aircraft.cfg parse falló: {}",
                        plane_dir.display(),
                        e
                    );
                }
            }
        }
    }

    tracing::info!(
        target: "aircraft_liveries",
        "scan completo: {} liveries de aircraft addons encontradas en {}",
        liveries.len(),
        community_path.display()
    );
    Ok(liveries)
}

/// Match `{vendor}-aircraft-{model}` con opcional `-liveries` al
/// final. Devuelve `(vendor, model)` si matchea un vendor conocido.
///
/// Ejemplos:
///   · "pmdg-aircraft-77er-liveries" → ("pmdg", "77er")
///   · "fnx-aircraft-320"            → ("fnx", "320")
///   · "inibuilds-aircraft-a350"     → ("inibuilds", "a350")
///   · "fbw-aircraft-a32nx"          → ("fbw", "a32nx")
fn detect_vendor_model(lower_folder: &str) -> Option<(String, String)> {
    for vendor in KNOWN_VENDORS {
        let prefix = format!("{}-aircraft-", vendor);
        if let Some(rest) = lower_folder.strip_prefix(&prefix) {
            // Strip `-liveries` opcional al final.
            let model = rest.trim_end_matches("-liveries").to_string();
            if model.is_empty() {
                continue;
            }
            return Some((vendor.to_string(), model));
        }
    }
    None
}

/// Parsea un `aircraft.cfg` en formato INI-like (no es INI puro:
/// usa `=` sin espacios mandatorios, comentarios con `//` o `;`,
/// secciones `[fltsim.N]`).
///
/// Devuelve UNA `PmdgLivery` por cada sección `[fltsim.N]`.
/// `plane_dir` se usa para resolver `ui_thumbnailfile` a path
/// absoluto (relativo a la carpeta del aircraft.cfg).
fn parse_aircraft_cfg(
    cfg_path: &Path,
    plane_dir: &Path,
) -> anyhow::Result<Vec<PmdgLivery>> {
    let bytes = std::fs::read(cfg_path)?;
    let text = decode_aircraft_cfg(&bytes);

    let mut out = Vec::new();
    let mut current: Option<PmdgLiveryDraft> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("//") || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            if let Some(draft) = current.take() {
                if let Some(liv) = draft.finalize(plane_dir) {
                    out.push(liv);
                }
            }
            let header = line[1..line.len() - 1].to_lowercase();
            if header.starts_with("fltsim.") {
                current = Some(PmdgLiveryDraft::default());
            } else {
                current = None;
            }
            continue;
        }

        let Some(draft) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value
            .split_once("//")
            .map(|(v, _)| v)
            .unwrap_or(value)
            .trim()
            .trim_matches('"')
            .to_string();
        match key.as_str() {
            "title" => draft.title = Some(value),
            "ui_variation" | "ui_type" => draft.variation = Some(value),
            "atc_id" => draft.tail_number = Some(value),
            "atc_airline" => draft.airline = Some(value),
            "texture" => draft.texture = Some(value),
            "ui_thumbnailfile" => draft.thumbnail = Some(value),
            _ => {}
        }
    }
    if let Some(draft) = current.take() {
        if let Some(liv) = draft.finalize(plane_dir) {
            out.push(liv);
        }
    }
    Ok(out)
}

fn decode_aircraft_cfg(bytes: &[u8]) -> String {
    let trimmed = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    String::from_utf8_lossy(trimmed).into_owned()
}

#[derive(Debug, Default)]
struct PmdgLiveryDraft {
    title: Option<String>,
    variation: Option<String>,
    tail_number: Option<String>,
    airline: Option<String>,
    texture: Option<String>,
    thumbnail: Option<String>,
}

impl PmdgLiveryDraft {
    fn finalize(self, plane_dir: &Path) -> Option<PmdgLivery> {
        let title = self.title?;
        let thumbnail_path: Option<PathBuf> = self.thumbnail.as_ref().and_then(|rel| {
            let candidate = plane_dir.join(rel);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        });
        let thumbnail_path = thumbnail_path.or_else(|| {
            let tex = self.texture.as_ref()?;
            let folder = plane_dir.join(format!("texture.{}", tex));
            for ext in &["jpg", "png", "jpeg"] {
                let p: PathBuf = folder.join(format!("thumbnail.{}", ext));
                if p.is_file() {
                    return Some(p);
                }
            }
            None
        });

        let thumbnail_data_url = thumbnail_path.and_then(|p| encode_thumbnail(&p));

        Some(PmdgLivery {
            vendor: String::new(), // populado por el caller
            model: String::new(),
            package_folder: String::new(),
            title,
            variation: self.variation,
            tail_number: self.tail_number,
            airline: self.airline,
            texture: self.texture,
            thumbnail_data_url,
        })
    }
}

fn encode_thumbnail(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())?;
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}
