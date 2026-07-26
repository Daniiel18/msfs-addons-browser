//! (v6.2.45) Instalador de liveries PMDG/iFly en **formato abierto**
//! (carpeta/`.zip` con `livery.cfg` + `texture*`), portado de la app
//! `doguer27/livery_manager_fsto` (verificada su lógica en `main.py`).
//!
//! Qué hace (mecanismo "A" de esa app — el que funciona bien):
//!   1. Detecta el avión objetivo leyendo `required_tags` de `livery.cfg`
//!      (PMDG) o el `base_container` de `aircraft.cfg` (iFly).
//!   2. Copia la livery al paquete Community `pmdg-flightmods-manager` /
//!      `ifly-flightmods-manager` bajo `SimObjects/Airplanes/…` en la
//!      variante correcta (`b738_ext`, `b77w_ext`, `900ER`, `GE`, …).
//!   3. Regenera `layout.json` **en Rust nativo** (sin depender del
//!      `MSFSLayoutGenerator.exe` de terceros).
//!
//! NO instalamos los `.ini` de config aquí — esos siguen su propio flujo
//! selectable en `drop_install` (carpeta `work` del avión). Aquí sólo va
//! el texturizado real de la livery.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Familia del avión: decide la estructura de carpetas destino.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcType {
    Pmdg,
    Ifly,
}

/// Livery detectada dentro de un drop, lista para instalar.
#[derive(Clone, Debug)]
pub struct LiveryInfo {
    /// Carpeta de la livery (contiene `livery.cfg`/`aircraft.cfg` + `texture*`).
    pub path: PathBuf,
    /// Nombre de la carpeta (será el nombre del destino).
    pub name: String,
    pub ac_type: AcType,
    /// Clave del avión: "PMDG 737-800", "iFly 737 MAX 8", …
    pub ac_key: String,
    /// Subcarpeta de SimObjects para PMDG ("PMDG 737-800"). Vacío en iFly.
    pub sim_folder: String,
    /// Etiqueta de variante para la UI: "PAX", "BCF", "900ER", "GE", …
    pub variant_label: Option<String>,
    /// Aerolínea (de `name =` en `livery.cfg`), para el label.
    pub airline: Option<String>,
}

/// Un avión PMDG conocido: clave visible, carpeta SimObjects y el mapa
/// tag→variante para etiquetar. El `wasm` lo usa el instalador de `.ini`
/// (en `drop_install`), aquí sólo enrutamos texturas.
struct PmdgSpec {
    key: &'static str,
    sim_folder: &'static str,
    /// (substring del required_tags → etiqueta de variante).
    variants: &'static [(&'static str, &'static str)],
}

/// Base de datos de aviones PMDG (portada de `AIRCRAFT_DB`). El orden
/// importa: los tags más específicos (`b739er_ext`) van antes que los
/// genéricos para ganar en el match por substring.
const PMDG_DB: &[PmdgSpec] = &[
    PmdgSpec {
        key: "PMDG 737-600",
        sim_folder: "PMDG 737-600",
        variants: &[("b736_ext", "600")],
    },
    PmdgSpec {
        key: "PMDG 737-800",
        sim_folder: "PMDG 737-800",
        variants: &[
            ("b738bdsf_ext", "BDSF"),
            ("b738bcf_ext", "BCF"),
            ("b73bbj2_ext", "BBJ2"),
            ("b738_ext", "PAX"),
        ],
    },
    PmdgSpec {
        key: "PMDG 737-900",
        sim_folder: "PMDG 737-900",
        variants: &[("b739er_ext", "900ER"), ("b739_ext", "900")],
    },
    PmdgSpec {
        key: "PMDG 777F",
        sim_folder: "PMDG 777F",
        variants: &[("b77f_ext", "F")],
    },
    PmdgSpec {
        key: "PMDG 777-300ER",
        sim_folder: "PMDG 777-300ER",
        variants: &[("b77w_ext", "300ER")],
    },
    PmdgSpec {
        key: "PMDG 777-200LR",
        sim_folder: "PMDG 777-200LR",
        variants: &[("b77l_ext", "200LR")],
    },
    PmdgSpec {
        key: "PMDG 777-200ER",
        sim_folder: "PMDG 777-200ER",
        variants: &[
            ("b772_ext,engine_ge", "GE"),
            ("b772_ext,engine_rr", "RR"),
            ("b772_ext,engine_pw", "PW"),
            ("b772_ext", "200ER"),
        ],
    },
];

/// Nombre del paquete Community contenedor. Coincide con el que crea la
/// app de doguer27 → si el usuario ya lo tiene, las liveries se mezclan
/// en el mismo paquete (no se duplica).
pub fn manager_package_name(ac_type: AcType) -> &'static str {
    match ac_type {
        AcType::Pmdg => "pmdg-flightmods-manager",
        AcType::Ifly => "ifly-flightmods-manager",
    }
}

/// ¿La carpeta `dir` es una livery instalable? Lee `livery.cfg` (PMDG) o
/// `aircraft.cfg` (iFly) y exige una subcarpeta `texture*`.
pub fn resolve_livery(dir: &Path) -> Option<LiveryInfo> {
    if !dir.is_dir() {
        return None;
    }
    let mut has_livery_cfg = false;
    let mut has_aircraft_cfg = false;
    let mut has_texture_dir = false;
    for entry in std::fs::read_dir(dir).ok()? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let lower = name.to_ascii_lowercase();
        if entry.path().is_dir() {
            if lower.starts_with("texture") {
                has_texture_dir = true;
            }
        } else if lower == "livery.cfg" {
            has_livery_cfg = true;
        } else if lower == "aircraft.cfg" {
            has_aircraft_cfg = true;
        }
    }
    if !has_texture_dir {
        return None;
    }

    let folder_name = dir.file_name()?.to_string_lossy().into_owned();

    // --- PMDG: por required_tags de livery.cfg ---
    if has_livery_cfg {
        let cfg = read_lossy(&dir.join_ci("livery.cfg")?);
        let tags = ini_value(&cfg, "required_tags")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if let Some((spec, variant)) = pmdg_match(&tags) {
            return Some(LiveryInfo {
                path: dir.to_path_buf(),
                name: folder_name,
                ac_type: AcType::Pmdg,
                ac_key: spec.key.to_string(),
                sim_folder: spec.sim_folder.to_string(),
                variant_label: Some(variant.to_string()),
                airline: ini_value(&cfg, "name"),
            });
        }
    }

    // --- iFly: aircraft.cfg con base_container hacia iFly ---
    if has_aircraft_cfg {
        let cfg = read_lossy(&dir.join_ci("aircraft.cfg")?).to_ascii_lowercase();
        if cfg.contains("ifly 737-max8") || cfg.contains("ifly-aircraft-737max8") {
            return Some(LiveryInfo {
                path: dir.to_path_buf(),
                name: folder_name,
                ac_type: AcType::Ifly,
                ac_key: "iFly 737 MAX 8".to_string(),
                sim_folder: String::new(),
                variant_label: Some("MAX 8".to_string()),
                airline: None,
            });
        }
    }

    None
}

/// Recorre `root` y devuelve todas las liveries instalables. No desciende
/// dentro de una livery ya detectada (sus `texture*` no son liveries).
pub fn find_liveries(root: &Path) -> Vec<LiveryInfo> {
    let mut out = Vec::new();
    let mut skip_under: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        if skip_under.iter().any(|p| path.starts_with(p) && path != p) {
            continue;
        }
        if let Some(info) = resolve_livery(path) {
            skip_under.push(path.to_path_buf());
            out.push(info);
        }
    }
    out
}

/// Instala UNA livery en su paquete manager de Community y devuelve la
/// ruta del paquete (para que el caller regenere `layout.json` una vez).
pub fn install_livery(livery_dir: &Path, community: &Path) -> anyhow::Result<PathBuf> {
    let info = resolve_livery(livery_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "No pude resolver el avión de la livery en {}",
            livery_dir.display()
        )
    })?;

    let manager = community.join(manager_package_name(info.ac_type));
    ensure_manager_package(&manager, manager_package_name(info.ac_type))?;

    let target_base = match info.ac_type {
        AcType::Pmdg => manager
            .join("SimObjects")
            .join("Airplanes")
            .join(&info.sim_folder)
            .join("liveries")
            .join("pmdg"),
        AcType::Ifly => manager.join("SimObjects").join("Airplanes"),
    };
    std::fs::create_dir_all(&target_base)?;

    let target = target_base.join(&info.name);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    copy_dir(livery_dir, &target)?;
    tracing::info!(
        target: "drop",
        "livery instalada: {} → {}",
        info.name, target.display()
    );
    Ok(manager)
}

/// Crea el paquete contenedor (manifest + layout placeholder) si no existe.
/// No sobrescribe un manifest previo (p.ej. el de la app de doguer27).
fn ensure_manager_package(manager: &Path, title: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(manager)?;
    let manifest = manager.join("manifest.json");
    if !manifest.exists() {
        // Manifest mínimo — EXACTAMENTE el que usa la app original (probado
        // en el sim). content_type AIRCRAFT para que MSFS lo cargue.
        let body = serde_json::json!({
            "dependencies": [],
            "content_type": "AIRCRAFT",
            "title": title,
            "creator": "SimFleet",
            "package_version": "1.0.0"
        });
        std::fs::write(&manifest, serde_json::to_string_pretty(&body)?)?;
    }
    let layout = manager.join("layout.json");
    if !layout.exists() {
        std::fs::write(&layout, "{\n  \"content\": []\n}")?;
    }
    Ok(())
}

#[derive(Serialize)]
struct LayoutEntry {
    path: String,
    size: u64,
    date: u64,
}
#[derive(Serialize)]
struct Layout {
    content: Vec<LayoutEntry>,
}

/// Regenera `layout.json` del paquete listando todos los archivos (rutas
/// relativas con `/`, tamaño y fecha en FILETIME de Windows), excluyendo
/// `layout.json` y `manifest.json` — comportamiento estándar del SDK.
pub fn regenerate_layout(pkg_dir: &Path) -> anyhow::Result<()> {
    let mut content = Vec::new();
    for entry in WalkDir::new(pkg_dir).sort_by_file_name() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = match path.strip_prefix(pkg_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str == "layout.json" || rel_str == "manifest.json" {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        content.push(LayoutEntry {
            path: rel_str,
            size: meta.len(),
            date: filetime_ticks(&meta),
        });
    }
    let layout = Layout { content };
    let json = serde_json::to_string_pretty(&layout)?;
    std::fs::write(pkg_dir.join("layout.json"), json)?;
    tracing::info!(
        target: "drop",
        "layout.json regenerado: {} ({} archivos)",
        pkg_dir.display(), layout.content.len()
    );
    Ok(())
}

/// (v6.2.45) `atc_id` (matrícula) de la livery cuyo `.ini` está en `dir`.
/// Es como la app original nombra el config PMDG en `work\Aircraft`:
/// `{atc_id}.ini` — así PMDG lo asocia a esa matrícula. Lee `livery.cfg`.
pub fn atc_id_in_dir(dir: &Path) -> Option<String> {
    let cfg = dir.join_ci("livery.cfg")?;
    let content = read_lossy(&cfg);
    ini_value(&content, "atc_id").map(sanitize_atc_id)
}

/// Deja el atc_id apto para nombre de archivo (PMDG usa la matrícula tal cual,
/// pero limpiamos separadores raros por seguridad).
fn sanitize_atc_id(raw: String) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

// ===========================================================================
// Helpers
// ===========================================================================

/// FILETIME de Windows (ticks de 100ns desde 1601-01-01) del mtime.
fn filetime_ticks(meta: &std::fs::Metadata) -> u64 {
    // Offset segundos entre 1601-01-01 y 1970-01-01.
    const EPOCH_DIFF: u64 = 11_644_473_600;
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs() + EPOCH_DIFF) * 10_000_000 + (d.subsec_nanos() as u64) / 100)
        .unwrap_or(0)
}

/// Match del required_tags contra el PMDG_DB. Devuelve (spec, etiqueta).
fn pmdg_match(tags_lower: &str) -> Option<(&'static PmdgSpec, &'static str)> {
    for spec in PMDG_DB {
        for (tag, label) in spec.variants {
            // El tag puede ser compuesto ("b772_ext,engine_ge"): TODAS las
            // partes deben estar presentes.
            let all = tag.split(',').all(|part| tags_lower.contains(part.trim()));
            if all {
                return Some((spec, label));
            }
        }
    }
    None
}

/// Lee un archivo como texto lossy (los .cfg de PMDG a veces traen BOM/latin1).
fn read_lossy(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

/// Valor de una clave `key = "valor"` en un .cfg estilo INI (case-insensitive
/// en la clave; quita comillas). Devuelve None si no está o está vacío.
fn ini_value(content: &str, key: &str) -> Option<String> {
    let key_lower = key.to_ascii_lowercase();
    for line in content.lines() {
        let trimmed = line.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        if k.trim().to_ascii_lowercase() == key_lower {
            let val = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Copia recursiva de un directorio.
fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Extensión: busca un archivo hijo por nombre case-insensitive.
trait JoinCi {
    fn join_ci(&self, name: &str) -> Option<PathBuf>;
}
impl JoinCi for Path {
    fn join_ci(&self, name: &str) -> Option<PathBuf> {
        let target = name.to_ascii_lowercase();
        std::fs::read_dir(self).ok()?.flatten().find_map(|e| {
            let n = e.file_name();
            if n.to_string_lossy().to_ascii_lowercase() == target {
                Some(e.path())
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pmdg_bcf_variant() {
        let (spec, label) = pmdg_match("b738bcf_ext").unwrap();
        assert_eq!(spec.key, "PMDG 737-800");
        assert_eq!(label, "BCF");
    }

    #[test]
    fn matches_pmdg_pax_not_bcf() {
        let (spec, label) = pmdg_match("b738_ext").unwrap();
        assert_eq!(spec.key, "PMDG 737-800");
        assert_eq!(label, "PAX");
    }

    #[test]
    fn matches_777_200er_engine() {
        let (spec, label) = pmdg_match("b772_ext,engine_rr").unwrap();
        assert_eq!(spec.key, "PMDG 777-200ER");
        assert_eq!(label, "RR");
    }

    #[test]
    fn matches_739er_before_739() {
        let (_spec, label) = pmdg_match("b739er_ext").unwrap();
        assert_eq!(label, "900ER");
    }

    #[test]
    fn ini_value_strips_quotes() {
        assert_eq!(
            ini_value("name = \"American Airlines\"\n", "name").as_deref(),
            Some("American Airlines")
        );
    }
}
