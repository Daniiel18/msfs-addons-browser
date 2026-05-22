//! Escáner de liveries de PMDG instaladas en la carpeta Community.
//!
//! PMDG distribuye las liveries en paquetes separados con la
//! convención `pmdg-aircraft-{model}-liveries`, por ejemplo:
//!   · `pmdg-aircraft-77er-liveries`  → 777-200ER
//!   · `pmdg-aircraft-77w-liveries`   → 777-300ER
//!   · `pmdg-aircraft-738-liveries`   → 737-800
//!   · `pmdg-aircraft-739-liveries`   → 737-900
//!
//! Dentro de cada paquete:
//!   `{paquete}/SimObjects/Airplanes/{Aircraft Title}/aircraft.cfg`
//! `aircraft.cfg` define una o más [fltsim.N] sections, una por
//! variante. Campos relevantes que parseamos:
//!   · title          → nombre canónico de la variante.
//!   · ui_variation   → nombre amigable en el sim.
//!   · atc_id         → tail number / matrícula.
//!   · atc_airline    → operador (United, Lufthansa…).
//!   · texture        → subcarpeta del livery.
//!   · ui_thumbnailfile → ruta relativa del thumbnail (DDS/PNG/JPG).
//!
//! Esta operación es síncrona y bloqueante porque la carpeta vive
//! en SSD local (los manifests pesan KB). En el comando lo
//! envolvemos en `spawn_blocking` para no bloquear el reactor de
//! Tokio.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Una livery individual instalada. Se mapea 1:1 a una sección
/// `[fltsim.N]` del `aircraft.cfg` del paquete PMDG.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PmdgLivery {
    /// Modelo PMDG normalizado: "77er", "77w", "738", "739", etc.
    pub model: String,
    /// Nombre del paquete tal como aparece en Community
    /// (ej. "pmdg-aircraft-77er-liveries").
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

/// Recorre la carpeta Community y devuelve TODAS las liveries PMDG
/// detectadas en cualquier paquete `pmdg-aircraft-{model}-liveries`.
///
/// **Robustez**:
///   · Si un paquete tiene varios subdirs `SimObjects/Airplanes/*`,
///     parseamos el `aircraft.cfg` de cada uno y mergeamos.
///   · Si un aircraft.cfg está mal-formado / inválido UTF-8, lo
///     logueamos y pasamos al siguiente.
///   · Las thumbnails ausentes no rompen la entrada — devolvemos
///     `thumbnail_path = None`.
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
        // Match `pmdg-aircraft-{model}-liveries` (case-insensitive).
        let lower = folder_name.to_lowercase();
        if !lower.starts_with("pmdg-aircraft-") || !lower.ends_with("-liveries") {
            continue;
        }
        // Extrae el modelo entre los prefijos. ej:
        // "pmdg-aircraft-77er-liveries" → "77er"
        let model = lower
            .trim_start_matches("pmdg-aircraft-")
            .trim_end_matches("-liveries")
            .to_string();

        // Cada paquete contiene SimObjects/Airplanes/* — uno o más
        // subdirs. Iteramos todos.
        let airplanes_dir = path.join("SimObjects").join("Airplanes");
        if !airplanes_dir.is_dir() {
            tracing::debug!(
                target: "pmdg_liveries",
                "{}: sin SimObjects/Airplanes — skip",
                folder_name
            );
            continue;
        }
        let plane_entries = match std::fs::read_dir(&airplanes_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    target: "pmdg_liveries",
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
                        liv.model = model.clone();
                        liv.package_folder = folder_name.clone();
                        liveries.push(liv);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "pmdg_liveries",
                        "{}: aircraft.cfg parse falló: {}",
                        plane_dir.display(),
                        e
                    );
                }
            }
        }
    }

    tracing::info!(
        target: "pmdg_liveries",
        "scan completo: {} liveries PMDG encontradas en {}",
        liveries.len(),
        community_path.display()
    );
    Ok(liveries)
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
    // El cfg puede venir en UTF-8 o UTF-8-BOM o, raramente, en
    // Windows-1252. Leemos como bytes y normalizamos.
    let bytes = std::fs::read(cfg_path)?;
    let text = decode_aircraft_cfg(&bytes);

    let mut out = Vec::new();
    let mut current: Option<PmdgLiveryDraft> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Comentarios.
        if line.starts_with("//") || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            // Cierra la sección anterior si era fltsim.N.
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
    // Cierra el último.
    if let Some(draft) = current.take() {
        if let Some(liv) = draft.finalize(plane_dir) {
            out.push(liv);
        }
    }
    Ok(out)
}

/// Heurística de decoding del aircraft.cfg. PMDG mayoritariamente
/// usa UTF-8 con BOM; algunos legacy/conversiones quedan en CP1252.
/// `String::from_utf8_lossy` cubre ambos casos sin panic; perdemos
/// algún acento exótico pero los nombres ASCII (los que importan
/// para parseo de keys) siempre sobreviven.
fn decode_aircraft_cfg(bytes: &[u8]) -> String {
    // Strip BOM si está.
    let trimmed = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    String::from_utf8_lossy(trimmed).into_owned()
}

/// Buffer intermedio para acumular campos de una sección [fltsim.N]
/// antes de finalizarla. Si no tiene `title`, no produce livery
/// (`fltsim.N` sin title es inválido).
#[derive(Debug, Default)]
struct PmdgLiveryDraft {
    title: Option<String>,
    variation: Option<String>,
    tail_number: Option<String>,
    airline: Option<String>,
    texture: Option<String>,
    /// Path relativo del thumbnail tal como aparece en el cfg.
    thumbnail: Option<String>,
}

impl PmdgLiveryDraft {
    fn finalize(self, plane_dir: &Path) -> Option<PmdgLivery> {
        let title = self.title?;
        // Resolver thumbnail a path absoluto. PMDG suele poner
        // `ui_thumbnailfile=textures.XXX/thumbnail.jpg` relativo al
        // aircraft.cfg. Algunos liveries no traen thumbnail propio
        // y heredan del base aircraft; en ese caso buscamos
        // `texture.<texture>/thumbnail.jpg` como fallback.
        let thumbnail_path: Option<PathBuf> = self.thumbnail.as_ref().and_then(|rel| {
            let candidate = plane_dir.join(rel);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        });
        // Fallback: `texture.<texture>/thumbnail.{jpg,png,jpeg}`.
        // (No DDS — WebView2 no las renderiza.)
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

        // Encodear el archivo a base64 data URL si tenemos uno.
        // El WebView no soporta DDS, así que filtramos por extensión
        // antes de leer; los DDS llegan a este punto sólo por el path
        // `ui_thumbnailfile` (no por nuestro fallback de textura).
        let thumbnail_data_url = thumbnail_path.and_then(|p| encode_thumbnail(&p));

        Some(PmdgLivery {
            model: String::new(), // populado por el caller
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

/// Lee la imagen y devuelve un data URL base64 si la extensión es
/// soportada por el WebView (JPG/PNG/JPEG). DDS se descarta — el
/// usuario verá el placeholder de avión.
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
    // Limit a 1 MB para no inundar la UI con thumbnails enormes.
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}
