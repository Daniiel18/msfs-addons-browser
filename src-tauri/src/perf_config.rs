//! (v5.0.0) Motor de rendimiento / optimización de FPS por aeropuerto.
//!
//! Muchos sceneries premium (ORBX, FlyTampa, Flightbeam…) traen objetos
//! pesados OPCIONALES — autos estáticos, personal de rampa 3D, pasajeros
//! animados, GSE, clutter — que el usuario puede desactivar para ganar
//! FPS renombrando archivos `.bgl` a `.bgl.off`. MSFS ignora cualquier
//! `.bgl.off` al cargar el escenario.
//!
//! La forma de hacerlo viene en la nota "Optional Configuration" de la
//! descripción del addon en SceneryAddons (ver imagen ORBX KATL). Este
//! módulo:
//!
//!  1. **Genera** un manifiesto `config/simfleet_perf.json` dentro de la
//!     carpeta del addon a partir de (a) esa nota parseada de
//!     SceneryAddons y (b) un escaneo local de `.bgl` opcionales por
//!     patrón de nombre (lo que de verdad existe en disco).
//!  2. **Lee** ese manifiesto reconciliando el estado real en disco
//!     (`.bgl` = activo, `.bgl.off` = desactivado).
//!  3. **Aplica** un toggle renombrando TODOS los archivos de una opción
//!     de forma atómica con rollback si algo falla a mitad (p. ej. MSFS
//!     tiene el escenario abierto → `os error 32`).
//!
//! El JSON soporta múltiples archivos por opción (`files: []`) porque
//! una sola opción ("Remove GSE") a veces toca 2-3 `.bgl` a la vez.

use std::fs;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Subcarpeta protegida dentro del addon donde guardamos el manifiesto.
const CONFIG_DIR: &str = "config";
const CONFIG_FILE: &str = "simfleet_perf.json";

/// Profundidad máxima al escanear `.bgl` dentro del addon. `scenery/
/// <icao>/<area>/placement/x.bgl` rara vez pasa de 5-6 niveles.
const SCAN_DEPTH: usize = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfOption {
    /// Identificador estable (= categoría). Úsalo en el toggle.
    pub id: String,
    /// Etiqueta visible. Si vino de SceneryAddons, la del dev
    /// ("Remove Static Cars"); si no, la nuestra en ES.
    pub label: String,
    /// Explicación clara de qué hace en ES.
    pub description: String,
    /// Estimado teórico de ahorro ("+2–5 FPS").
    pub fps_hint: String,
    /// Categoría canónica (static_cars, passengers, gse, …).
    pub category: String,
    /// Rutas RELATIVAS al addon del/los `.bgl` ACTIVO(s) (sin `.off`).
    pub files: Vec<String>,
    /// true = objetos presentes (`.bgl`); false = desactivados (`.bgl.off`).
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfConfig {
    pub icao: Option<String>,
    pub folder_name: String,
    /// "sceneryaddons" | "local-scan" | "mixed".
    pub source: String,
    pub options: Vec<PerfOption>,
}

/// Opción cruda antes de fusionar/decorar.
struct RawOpt {
    category: String,
    /// Etiqueta del dev (None → usamos la ES por categoría).
    label: Option<String>,
    files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Categorización por nombre de archivo
// ---------------------------------------------------------------------------

/// Mapea el basename de un `.bgl` a una categoría de objeto OPCIONAL, o
/// `None` si no parece clutter desactivable (terminal, ground, etc. NO
/// se tocan jamás). Conservador a propósito.
fn categorize(basename_lower: &str) -> Option<&'static str> {
    // Animado primero (gana sobre car/gse/personnel cuando ambos están).
    // Movimiento = "anim"/"walk"/"moving"; exigimos contexto de agente y
    // que NO sea "static" para no confundir un pasillo (walkway) ni el
    // personal ESTÁTICO con los agentes animados.
    let motion = basename_lower.contains("anim")
        || basename_lower.contains("walk")
        || basename_lower.contains("moving");
    let agent_ctx = basename_lower.contains("car")
        || basename_lower.contains("gse")
        || basename_lower.contains("ramp")
        || basename_lower.contains("vehicle")
        || basename_lower.contains("agent")
        || basename_lower.contains("crew")
        || basename_lower.contains("people")
        || basename_lower.contains("passenger")
        || basename_lower.contains("marshall");
    if motion && agent_ctx && !basename_lower.contains("static") {
        return Some("animated");
    }
    if basename_lower.contains("passenger") || basename_lower.contains("_pax") {
        return Some("passengers");
    }
    if basename_lower.contains("rampguy")
        || basename_lower.contains("rampagent")
        || basename_lower.contains("personnel")
        || basename_lower.contains("marshall")
        || basename_lower.contains("rampcrew")
    {
        return Some("ramp_personnel");
    }
    if basename_lower.contains("gse") {
        return Some("gse");
    }
    // "car" pero no "cargo"/"carrier"/"scenery".
    if (basename_lower.contains("_car") || basename_lower.contains("car_") || basename_lower.contains("cars"))
        && !basename_lower.contains("cargo")
        && !basename_lower.contains("carrier")
    {
        return Some("static_cars");
    }
    if basename_lower.contains("static_aircraft")
        || basename_lower.contains("staticaircraft")
        || basename_lower.contains("static_ac")
        || basename_lower.contains("parked")
    {
        return Some("static_aircraft");
    }
    if basename_lower.contains("clutter") {
        return Some("clutter");
    }
    None
}

fn default_label(category: &str) -> &'static str {
    match category {
        "static_cars" => "Autos estáticos",
        "ramp_personnel" => "Personal de rampa 3D",
        "passengers" => "Pasajeros 3D",
        "gse" => "Equipo de tierra (GSE)",
        "animated" => "Agentes / vehículos animados",
        "static_aircraft" => "Aeronaves estáticas",
        "clutter" => "Clutter / objetos de detalle",
        _ => "Objetos opcionales",
    }
}

fn description_for(category: &str) -> &'static str {
    match category {
        "static_cars" => "Quita los autos estáticos del aparcamiento y viales del aeropuerto.",
        "ramp_personnel" => "Quita el personal de rampa estático (operarios 3D en plataforma).",
        "passengers" => "Quita los pasajeros 3D de terminales y puertas de embarque.",
        "gse" => "Quita el equipo de tierra estático (tractores, cintas, escaleras, GPU).",
        "animated" => "Quita los agentes y vehículos ANIMADOS (los que más cuestan FPS).",
        "static_aircraft" => "Quita las aeronaves estáticas (AI parqueadas decorativas).",
        "clutter" => "Quita el clutter / objetos de detalle no esenciales del escenario.",
        _ => "Objetos opcionales del escenario que puedes desactivar para ganar FPS.",
    }
}

fn fps_hint_for(category: &str) -> &'static str {
    match category {
        "animated" => "+3–6 FPS",
        "passengers" => "+2–5 FPS",
        "static_aircraft" => "+2–4 FPS",
        "static_cars" | "clutter" | "gse" => "+1–3 FPS",
        _ => "+1–2 FPS",
    }
}

// ---------------------------------------------------------------------------
// Parser de la nota "Optional Configuration" de SceneryAddons
// ---------------------------------------------------------------------------

// `path/x.bgl` con `.off` opcional capturado aparte. Non-greedy en el
// path. Sin lookahead (el crate `regex` no lo soporta): distinguimos el
// `.bgl` "activo" del `.bgl.off` por si el grupo 2 está presente.
static BGL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([A-Za-z0-9_\-./\\]+?\.bgl)(\.off)?\b").unwrap());

// "Remove <label>:" — la etiqueta del dev en la nota.
static REMOVE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bremove\s+([^:]{2,60}?)\s*:").unwrap());

/// Parsea el texto de la descripción del addon buscando instrucciones
/// del tipo `* Remove Static Cars: x.bgl -> x.bgl.off`. Devuelve una
/// opción cruda por línea con `.bgl` "activos". Soporta múltiples
/// archivos por línea.
fn parse_optional_config(text: &str) -> Vec<RawOpt> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.to_lowercase().contains(".bgl") {
            continue;
        }
        // Lado izquierdo del primer "->" (o de "to"/" a ") = los activos.
        let left = line
            .split("->")
            .next()
            .unwrap_or(line)
            // por si usan "x.bgl to x.bgl.off"
            .split(" to ")
            .next()
            .unwrap_or(line);

        let mut files: Vec<String> = Vec::new();
        for cap in BGL_RE.captures_iter(left) {
            // Sólo el `.bgl` SIN `.off` (grupo 2 ausente) → es el activo.
            if cap.get(2).is_some() {
                continue;
            }
            let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let norm = normalize_rel(raw);
            if !norm.is_empty() && !files.contains(&norm) {
                files.push(norm);
            }
        }
        if files.is_empty() {
            continue;
        }

        // Categoría: por la etiqueta "Remove X" si existe; si no, por el
        // primer filename.
        let label = REMOVE_RE
            .captures(line)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());
        let cat_src = label
            .clone()
            .unwrap_or_else(|| files[0].clone())
            .to_lowercase();
        let category = categorize(&cat_src)
            .or_else(|| categorize(&files[0].to_lowercase()))
            .unwrap_or("other")
            .to_string();

        out.push(RawOpt {
            category,
            label,
            files,
        });
    }
    out
}

/// Extrae el cuerpo de texto de la página de detalle de SceneryAddons.
/// Tomamos todo el texto del `<article>`/contenido del post — el parser
/// de Optional Configuration ya filtra sólo las líneas con `.bgl`.
pub fn extract_description_from_html(html: &str) -> String {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    // Preferimos el contenedor de contenido del tema; si no, el article.
    for sel in [
        "div.entry-content",
        "div.nv-content-wrap",
        "article",
        "main",
        "body",
    ] {
        if let Ok(s) = Selector::parse(sel) {
            if let Some(el) = doc.select(&s).next() {
                // `\n` entre nodos de bloque para que cada "Remove …"
                // quede en su propia línea.
                let text = el
                    .text()
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.to_lowercase().contains(".bgl") {
                    return text;
                }
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Escaneo local de `.bgl` opcionales
// ---------------------------------------------------------------------------

/// Normaliza una ruta relativa a separador `/` y sin `./` inicial.
fn normalize_rel(s: &str) -> String {
    s.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

/// Escanea el addon buscando `.bgl`/`.bgl.off` cuyo nombre matchee un
/// patrón de objeto opcional. Agrupa por categoría. Las rutas son
/// relativas al addon y SIN `.off` (apuntan al activo canónico).
fn detect_local(addon_dir: &Path) -> Vec<RawOpt> {
    let mut by_cat: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    walk_bgls(addon_dir, addon_dir, 0, &mut by_cat);
    by_cat
        .into_iter()
        .map(|(category, files)| RawOpt {
            category,
            label: None,
            files,
        })
        .collect()
}

fn walk_bgls(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut std::collections::BTreeMap<String, Vec<String>>,
) {
    if depth > SCAN_DEPTH || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_bgls(root, &path, depth + 1, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        // Activo (.bgl) o desactivado (.bgl.off): normalizamos al activo.
        let active_name = if let Some(stripped) = name.strip_suffix(".bgl.off") {
            Some(format!("{stripped}.bgl"))
        } else if name.ends_with(".bgl") {
            Some(name.clone())
        } else {
            None
        };
        let Some(active_name) = active_name else { continue };
        let Some(category) = categorize(&active_name) else { continue };

        // Ruta relativa al addon del archivo ACTIVO.
        let active_path = if name.ends_with(".bgl.off") {
            path.with_file_name(path.file_stem().unwrap_or_default())
        } else {
            path.clone()
        };
        let rel = active_path
            .strip_prefix(root)
            .ok()
            .map(|p| normalize_rel(&p.to_string_lossy()))
            .unwrap_or_default();
        if rel.is_empty() {
            continue;
        }
        let bucket = out.entry(category.to_string()).or_default();
        if !bucket.contains(&rel) {
            bucket.push(rel);
        }
    }
}

// ---------------------------------------------------------------------------
// Construcción / reconciliación del manifiesto
// ---------------------------------------------------------------------------

/// Fusiona opciones crudas (de SceneryAddons + escaneo local) por
/// categoría, decora con etiqueta/descripción/FPS, reconcilia el estado
/// real en disco y devuelve la config. `None` si no hay opciones.
fn build_config(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    source: &str,
    raws: Vec<RawOpt>,
) -> Option<PerfConfig> {
    use std::collections::BTreeMap;
    let mut merged: BTreeMap<String, (Option<String>, Vec<String>)> = BTreeMap::new();
    for r in raws {
        let e = merged.entry(r.category).or_insert((None, Vec::new()));
        if e.0.is_none() {
            e.0 = r.label;
        }
        for f in r.files {
            if !e.1.contains(&f) {
                e.1.push(f);
            }
        }
    }

    let mut options = Vec::new();
    for (category, (dev_label, mut files)) in merged {
        // Sólo conservamos archivos que de verdad existan en disco (como
        // `.bgl` o `.bgl.off`). Así no ofrecemos toggles fantasma cuando
        // la nota de SceneryAddons menciona archivos que este pack no trae.
        files.retain(|rel| {
            let active = addon_dir.join(rel);
            let off = with_off(&active);
            active.exists() || off.exists()
        });
        if files.is_empty() {
            continue;
        }
        let enabled = files.iter().any(|rel| addon_dir.join(rel).exists());
        let label = dev_label.unwrap_or_else(|| default_label(&category).to_string());
        options.push(PerfOption {
            id: category.clone(),
            label,
            description: description_for(&category).to_string(),
            fps_hint: fps_hint_for(&category).to_string(),
            category,
            files,
            enabled,
        });
    }

    if options.is_empty() {
        return None;
    }
    // Orden: los de mayor ahorro primero (animated, passengers, …).
    options.sort_by_key(|o| category_rank(&o.category));
    Some(PerfConfig {
        icao,
        folder_name: folder_name.to_string(),
        source: source.to_string(),
        options,
    })
}

fn category_rank(category: &str) -> u8 {
    match category {
        "animated" => 0,
        "passengers" => 1,
        "static_aircraft" => 2,
        "gse" => 3,
        "static_cars" => 4,
        "clutter" => 5,
        "ramp_personnel" => 6,
        _ => 9,
    }
}

fn with_off(active: &Path) -> PathBuf {
    let mut s = active.as_os_str().to_owned();
    s.push(".off");
    PathBuf::from(s)
}

fn config_dir(addon_dir: &Path) -> PathBuf {
    addon_dir.join(CONFIG_DIR)
}
fn config_file(addon_dir: &Path) -> PathBuf {
    config_dir(addon_dir).join(CONFIG_FILE)
}

/// Escribe el manifiesto en `config/simfleet_perf.json`.
fn write_config(addon_dir: &Path, cfg: &PerfConfig) -> std::io::Result<()> {
    let dir = config_dir(addon_dir);
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".into());
    fs::write(config_file(addon_dir), json)
}

/// Lee el manifiesto si existe. No reconcilia (eso lo hace el caller).
fn read_config_raw(addon_dir: &Path) -> Option<PerfConfig> {
    let path = config_file(addon_dir);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<PerfConfig>(&bytes).ok()
}

/// Reconcilia el campo `enabled` de cada opción con el estado real en
/// disco (`.bgl` presente = activo).
fn reconcile(addon_dir: &Path, cfg: &mut PerfConfig) {
    for opt in &mut cfg.options {
        opt.files.retain(|rel| {
            let active = addon_dir.join(rel);
            active.exists() || with_off(&active).exists()
        });
        opt.enabled = opt.files.iter().any(|rel| addon_dir.join(rel).exists());
    }
    cfg.options.retain(|o| !o.files.is_empty());
}

/// Comprobación rápida (early-exit, con presupuesto de archivos) de si
/// un addon tiene objetos opcionales desactivables. Para el badge de la
/// UI — no construye ni escribe nada. Funciona igual con escenarios
/// LOCALES (escanea el directorio instalado directamente).
pub fn has_optional_objects(addon_dir: &Path) -> bool {
    // Si ya hay manifiesto generado, basta con mirarlo (rápido).
    if let Some(cfg) = read_config_raw(addon_dir) {
        if cfg.options.iter().any(|o| {
            o.files
                .iter()
                .any(|rel| addon_dir.join(rel).exists() || with_off(&addon_dir.join(rel)).exists())
        }) {
            return true;
        }
    }
    let mut budget: usize = 4000; // tope de archivos a inspeccionar
    scan_for_any(addon_dir, 0, &mut budget)
}

fn scan_for_any(dir: &Path, depth: usize, budget: &mut usize) -> bool {
    if depth > SCAN_DEPTH || *budget == 0 || !dir.is_dir() {
        return false;
    }
    let Ok(iter) = fs::read_dir(dir) else { return false };
    for entry in iter.flatten() {
        if *budget == 0 {
            return false;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if scan_for_any(&path, depth + 1, budget) {
                return true;
            }
            continue;
        }
        *budget -= 1;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let active = if let Some(s) = name.strip_suffix(".bgl.off") {
            format!("{s}.bgl")
        } else if name.ends_with(".bgl") {
            name
        } else {
            continue;
        };
        if categorize(&active).is_some() {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Lee el manifiesto del addon; si no existe, lo genera a partir del
/// escaneo local y lo escribe. Siempre reconcilia con el disco. `None`
/// si el escenario no tiene objetos opcionales detectables.
pub fn read_or_generate(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
) -> Option<PerfConfig> {
    if let Some(mut cfg) = read_config_raw(addon_dir) {
        reconcile(addon_dir, &mut cfg);
        if cfg.options.is_empty() {
            return None;
        }
        return Some(cfg);
    }
    let raws = detect_local(addon_dir);
    let cfg = build_config(addon_dir, folder_name, icao, "local-scan", raws)?;
    if let Err(e) = write_config(addon_dir, &cfg) {
        tracing::warn!("perf: no se pudo escribir {CONFIG_FILE} en {}: {e}", addon_dir.display());
    }
    Some(cfg)
}

/// Enriquecе el manifiesto con la nota "Optional Configuration"
/// parseada del HTML de la página de SceneryAddons + el escaneo local,
/// y reescribe el archivo. Devuelve la config resultante.
pub fn enrich_with_html(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    page_html: &str,
) -> Option<PerfConfig> {
    let desc = extract_description_from_html(page_html);
    let mut raws = parse_optional_config(&desc);
    let parsed_n = raws.len();
    raws.extend(detect_local(addon_dir));
    let source = if parsed_n > 0 { "mixed" } else { "local-scan" };
    let cfg = build_config(addon_dir, folder_name, icao, source, raws)?;
    if let Err(e) = write_config(addon_dir, &cfg) {
        tracing::warn!("perf: no se pudo escribir {CONFIG_FILE}: {e}");
    }
    Some(cfg)
}

/// (Descarga) Genera el manifiesto en segundo plano tras instalar un
/// aeropuerto. Registra SIEMPRE en logs si el archivo se creó o no.
/// `description` opcional = nota de SceneryAddons si la conocemos.
pub fn generate_on_install(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    description: Option<&str>,
) {
    let mut raws = description.map(parse_optional_config).unwrap_or_default();
    let parsed_n = raws.len();
    raws.extend(detect_local(addon_dir));
    let source = if parsed_n > 0 { "mixed" } else { "local-scan" };
    match build_config(addon_dir, folder_name, icao, source, raws) {
        Some(cfg) => match write_config(addon_dir, &cfg) {
            Ok(()) => tracing::info!(
                "perf: config OK para {} → {} opción(es) ({})",
                folder_name,
                cfg.options.len(),
                config_file(addon_dir).display()
            ),
            Err(e) => tracing::warn!("perf: FALLÓ escribir config de {folder_name}: {e}"),
        },
        None => tracing::info!(
            "perf: {} no tiene objetos opcionales detectables (sin config)",
            folder_name
        ),
    }
}

/// Resultado del toggle: la opción con su nuevo estado + cuántos
/// archivos se renombraron.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleResult {
    pub option: PerfOption,
    pub renamed: usize,
}

/// Aplica un toggle a una opción: renombra TODOS sus `.bgl`↔`.bgl.off`
/// de forma atómica. Si algo falla a mitad, revierte lo ya hecho y
/// devuelve un error claro. `enable=false` desactiva (gana FPS).
pub fn apply_toggle(
    addon_dir: &Path,
    option_id: &str,
    enable: bool,
) -> anyhow::Result<ToggleResult> {
    let mut cfg = read_or_generate(addon_dir, "", None)
        .ok_or_else(|| anyhow::anyhow!("Este escenario no tiene opciones de rendimiento."))?;
    let opt_idx = cfg
        .options
        .iter()
        .position(|o| o.id == option_id)
        .ok_or_else(|| anyhow::anyhow!("Opción '{option_id}' no encontrada."))?;

    // Plan de renombrados (from → to) sólo para los que hace falta tocar.
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    for rel in &cfg.options[opt_idx].files {
        let active = addon_dir.join(rel);
        let off = with_off(&active);
        if enable {
            // Restaurar: .bgl.off → .bgl (sólo si está desactivado).
            if off.exists() && !active.exists() {
                plan.push((off, active));
            }
        } else {
            // Desactivar: .bgl → .bgl.off (sólo si está activo).
            if active.exists() && !off.exists() {
                plan.push((active, off));
            }
        }
    }

    // Ejecuta con rollback.
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in &plan {
        match fs::rename(from, to) {
            Ok(()) => done.push((from.clone(), to.clone())),
            Err(e) => {
                // Revertir lo ya renombrado para no dejar el addon a medias.
                for (df, dt) in done.iter().rev() {
                    let _ = fs::rename(dt, df);
                }
                let hint = if e.raw_os_error() == Some(32) {
                    " — MSFS tiene el escenario abierto. Cierra el simulador (o vuelve al menú principal) e inténtalo de nuevo."
                } else {
                    ""
                };
                return Err(anyhow::anyhow!(
                    "No se pudo renombrar {}: {e}{hint}",
                    from.display()
                ));
            }
        }
    }

    cfg.options[opt_idx].enabled = enable;
    let renamed = done.len();
    // Persistimos el nuevo estado en el manifiesto (best-effort).
    let _ = write_config(addon_dir, &cfg);
    Ok(ToggleResult {
        option: cfg.options.swap_remove(opt_idx),
        renamed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_orbx_katl_note() {
        let note = "\
Optional Configuration: To remove various objects, rename:\n\
* Remove Static Cars: placement_full_car_flatten.bgl -> placement_full_car_flatten.bgl.off\n\
* Remove Static Ramp Personnel: placement_full_gse_rampguy_static_flatten.bgl -> placement_full_gse_rampguy_static_flatten.bgl.off\n\
* Remove 3D Passengers: placement_full_passengers_flatten.bgl -> placement_full_passengers_flatten.bgl.off\n\
* Remove Animated Ramp Agents: placement_rampguy_walking_flatten.bgl -> placement_rampguy_walking_flatten.bgl.off\n\
* Remove Animated GSE: placement_car_gse_anim_flatten.bgl -> placement_car_gse_anim_flatten.bgl.off\n";
        let opts = parse_optional_config(note);
        assert!(opts.len() >= 4, "esperaba >=4 opciones, hubo {}", opts.len());
        // Cada opción captura exactamente 1 archivo activo (sin el .off).
        for o in &opts {
            assert_eq!(o.files.len(), 1, "cat {} files {:?}", o.category, o.files);
            assert!(o.files[0].ends_with(".bgl"));
            assert!(!o.files[0].ends_with(".off"));
        }
    }

    #[test]
    fn categorizes_known_patterns() {
        assert_eq!(categorize("placement_full_car_flatten.bgl"), Some("static_cars"));
        assert_eq!(categorize("placement_full_passengers_flatten.bgl"), Some("passengers"));
        assert_eq!(categorize("ymml_airside_gse.bgl"), Some("gse"));
        assert_eq!(categorize("placement_rampguy_walking_flatten.bgl"), Some("animated"));
        // Core del aeropuerto NO se toca.
        assert_eq!(categorize("ymml_terminal.bgl"), None);
        assert_eq!(categorize("aptmarkings.bgl"), None);
        assert_eq!(categorize("cargo_building.bgl"), None);
    }
}
