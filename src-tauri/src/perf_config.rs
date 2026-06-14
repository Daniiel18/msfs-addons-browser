//! (v5.0.0) Motor de rendimiento / optimización de FPS por aeropuerto.
//!
//! Muchos sceneries premium (Aerosoft, ORBX, FlyTampa…) traen objetos
//! pesados OPCIONALES — autos estáticos, personal de rampa, pasajeros,
//! GSE, clutter — que el usuario puede desactivar para ganar FPS
//! renombrando archivos `.bgl`. La forma de hacerlo viene en la nota
//! "Optional Configuration" de la página del addon en SceneryAddons.
//!
//! Dos convenciones de "desactivado" conviven en el mundo real:
//!   · `X.bgl` → `X.bgl.off`  (ORBX y otros; sufijo añadido)
//!   · `X.bgl` → `X.off`      (Aerosoft; se reemplaza la extensión)
//! MSFS ignora cualquier archivo que NO termine exactamente en `.bgl`,
//! así que ambas funcionan. Detectamos las dos y, al desactivar,
//! estandarizamos en `.bgl.off`.
//!
//! La nota de SceneryAddons da SÓLO el nombre del archivo
//! (`EDDF_Placements_passengers.bgl`), pero en disco vive en una
//! subcarpeta (`scenery/EDDF/scenery/…`). Por eso construimos un índice
//! `basename → ruta real` y resolvemos cada archivo de la nota contra él.
//!
//! El JSON `config/simfleet_perf.json` soporta varios archivos por
//! opción (`files: []`) y se escribe SÓLO cuando hay una nota real de la
//! página (o tras un toggle) — el badge de "optimizable" refleja eso.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = "config";
const CONFIG_FILE: &str = "simfleet_perf.json";

/// Profundidad máxima al escanear `.bgl`. `scenery/<icao>/<area>/x.bgl`
/// rara vez pasa de 5-6 niveles.
const SCAN_DEPTH: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub fps_hint: String,
    pub category: String,
    /// Rutas RELATIVAS al addon del/los `.bgl` ACTIVO(s) (sin `.off`).
    pub files: Vec<String>,
    /// true = objetos presentes (`.bgl`); false = desactivados.
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

// ---------------------------------------------------------------------------
// Categorización por nombre de archivo / etiqueta
// ---------------------------------------------------------------------------

/// Mapea un texto (basename o etiqueta del dev) a una categoría de
/// objeto OPCIONAL, o `None` si no parece clutter desactivable.
fn categorize(text_lower: &str) -> Option<&'static str> {
    let motion = text_lower.contains("anim")
        || text_lower.contains("walk")
        || text_lower.contains("moving");
    let agent_ctx = text_lower.contains("car")
        || text_lower.contains("gse")
        || text_lower.contains("ramp")
        || text_lower.contains("vehicle")
        || text_lower.contains("agent")
        || text_lower.contains("crew")
        || text_lower.contains("people")
        || text_lower.contains("passenger")
        || text_lower.contains("marshall");
    if motion && agent_ctx && !text_lower.contains("static") {
        return Some("animated");
    }
    if text_lower.contains("passenger") || text_lower.contains("_pax") {
        return Some("passengers");
    }
    if text_lower.contains("road") && text_lower.contains("traffic") {
        return Some("road_traffic");
    }
    if text_lower.contains("servicetraffic")
        || (text_lower.contains("service") && text_lower.contains("traffic"))
        || text_lower.contains("service vehicle")
    {
        return Some("service_traffic");
    }
    if text_lower.contains("rampguy")
        || text_lower.contains("rampagent")
        || text_lower.contains("personnel")
        || text_lower.contains("marshall")
        || text_lower.contains("rampcrew")
    {
        return Some("ramp_personnel");
    }
    if text_lower.contains("gse") {
        return Some("gse");
    }
    if (text_lower.contains("_car") || text_lower.contains("car_") || text_lower.contains("cars") || text_lower.contains(" car"))
        && !text_lower.contains("cargo")
        && !text_lower.contains("carrier")
    {
        return Some("static_cars");
    }
    if text_lower.contains("static_aircraft")
        || text_lower.contains("staticaircraft")
        || text_lower.contains("static_ac")
        || text_lower.contains("staticac")
        || text_lower.contains("static aircraft")
        || text_lower.contains("parked")
    {
        return Some("static_aircraft");
    }
    if text_lower.contains("clutter")
        || text_lower.contains("extra")
        || text_lower.contains("streetlight")
        || text_lower.contains("fence")
    {
        return Some("clutter");
    }
    if text_lower.contains("vdgs") {
        return Some("vdgs");
    }
    None
}

fn default_label(category: &str) -> &'static str {
    match category {
        "static_cars" => "Autos estáticos",
        "ramp_personnel" => "Personal de rampa",
        "passengers" => "Pasajeros 3D",
        "gse" => "Equipo de tierra (GSE)",
        "animated" => "Agentes / vehículos animados",
        "static_aircraft" => "Aeronaves estáticas",
        "clutter" => "Clutter / objetos extra",
        "road_traffic" => "Tráfico de carretera",
        "service_traffic" => "Tráfico de vehículos de servicio",
        "vdgs" => "Cajas VDGS",
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
        "clutter" => "Quita el clutter / objetos de detalle no esenciales (modelos, farolas, vallas).",
        "road_traffic" => "Quita el tráfico de carretera alrededor del aeropuerto.",
        "service_traffic" => "Quita los vehículos de servicio en movimiento por la plataforma.",
        "vdgs" => "Quita las cajas VDGS de Aerosoft (úsalo si manejas VDGS con GSX).",
        _ => "Objetos opcionales del escenario que puedes desactivar para ganar FPS.",
    }
}

fn fps_hint_for(category: &str) -> &'static str {
    match category {
        "animated" | "service_traffic" => "+3–6 FPS",
        "passengers" | "road_traffic" => "+2–5 FPS",
        "static_aircraft" => "+2–4 FPS",
        "static_cars" | "clutter" | "gse" => "+1–3 FPS",
        _ => "+1–2 FPS",
    }
}

fn category_rank(category: &str) -> u8 {
    match category {
        "animated" => 0,
        "service_traffic" => 1,
        "passengers" => 2,
        "road_traffic" => 3,
        "static_aircraft" => 4,
        "gse" => 5,
        "static_cars" => 6,
        "clutter" => 7,
        "ramp_personnel" => 8,
        "vdgs" => 9,
        _ => 15,
    }
}

// ---------------------------------------------------------------------------
// Parser de la nota "Optional Configuration" de SceneryAddons
// ---------------------------------------------------------------------------

// `path/x.bgl` con `.off` opcional capturado aparte (sin lookahead).
static BGL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([A-Za-z0-9_\-./\\]+?\.bgl)(\.off)?\b").unwrap());

// "Remove/Disable/Delete <label>:" — etiqueta del dev.
static REMOVE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:remove|disable|delete)\s+([^:]{2,80}?)\s*:").unwrap());

struct NoteOption {
    label: String,
    /// Basenames (sólo el nombre, sin ruta) de los `.bgl` ACTIVOS.
    files: Vec<String>,
}

/// Parsea la nota buscando líneas de REMOCIÓN del tipo
/// `• Remove Static Cars: X.bgl -> X.off`. Sólo toma líneas con un verbo
/// de remoción (remove/disable/delete) y un `.bgl` a la izquierda del
/// `->` — así NO captura los "swaps" (p. ej. interiores simples) ni los
/// `.bat`, que no son simples on/off.
fn parse_note_options(text: &str) -> Vec<NoteOption> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['•', '*', '-', ' ']);
        let lower = line.to_lowercase();
        if !lower.contains(".bgl") {
            continue;
        }
        if !(lower.contains("remove") || lower.contains("disable") || lower.contains("delete")) {
            continue;
        }
        // Lado izquierdo del primer "->" = los `.bgl` ACTIVOS a desactivar.
        let left = line.split("->").next().unwrap_or(line);
        let mut files: Vec<String> = Vec::new();
        for cap in BGL_RE.captures_iter(left) {
            if cap.get(2).is_some() {
                continue; // es un `.bgl.off`, no el activo
            }
            let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let bn = basename_lower(raw);
            if !bn.is_empty() && !files.contains(&bn) {
                files.push(bn);
            }
        }
        if files.is_empty() {
            continue;
        }
        let label = REMOVE_RE
            .captures(line)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| pretty_from_basename(&files[0]));
        out.push(NoteOption { label, files });
    }
    out
}

/// Extrae el cuerpo de texto de la página de detalle de SceneryAddons.
pub fn extract_description_from_html(html: &str) -> String {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    for sel in ["div.entry-content", "div.nv-content-wrap", "article", "main", "body"] {
        if let Ok(s) = Selector::parse(sel) {
            if let Some(el) = doc.select(&s).next() {
                let text = el.text().collect::<Vec<_>>().join("\n");
                if text.to_lowercase().contains(".bgl") {
                    return text;
                }
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Índice de `.bgl` en disco + escaneo local por patrón
// ---------------------------------------------------------------------------

fn basename_lower(s: &str) -> String {
    s.trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(s)
        .to_lowercase()
}

/// Nombre ACTIVO (con `.bgl`) preservando el case original, sea el
/// archivo `X.bgl`, `X.bgl.off` o `X.off`.
fn to_active_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".bgl.off") {
        format!("{}.bgl", &name[..name.len() - 8])
    } else if lower.ends_with(".off") {
        format!("{}.bgl", &name[..name.len() - 4])
    } else {
        name.to_string()
    }
}

fn normalize_rel(s: &str) -> String {
    s.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

/// Construye `basename_activo(lower) → ruta relativa ACTIVA` recorriendo
/// el addon. Cubre `.bgl`, `.bgl.off` y `.off`.
fn index_bgls(addon_dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    walk_index(addon_dir, addon_dir, 0, &mut map);
    map
}

fn walk_index(root: &Path, dir: &Path, depth: usize, map: &mut HashMap<String, String>) {
    if depth > SCAN_DEPTH || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_index(root, &path, depth + 1, map);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if !(lower.ends_with(".bgl") || lower.ends_with(".bgl.off") || lower.ends_with(".off")) {
            continue;
        }
        let active_name = to_active_name(&name);
        let active_path = path.with_file_name(&active_name);
        let Ok(rel) = active_path.strip_prefix(root) else { continue };
        let rel = normalize_rel(&rel.to_string_lossy());
        if rel.is_empty() {
            continue;
        }
        map.entry(active_name.to_lowercase()).or_insert(rel);
    }
}

struct LocalOpt {
    category: String,
    files: Vec<String>,
}

/// Escaneo LOCAL por patrón de nombre — fallback cuando no hay nota.
/// Agrupa por categoría. Rutas relativas (activas).
fn detect_local(addon_dir: &Path) -> Vec<LocalOpt> {
    let mut by_cat: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    walk_local(addon_dir, addon_dir, 0, &mut by_cat);
    by_cat
        .into_iter()
        .map(|(category, files)| LocalOpt { category, files })
        .collect()
}

fn walk_local(
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
            walk_local(root, &path, depth + 1, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if !(lower.ends_with(".bgl") || lower.ends_with(".bgl.off") || lower.ends_with(".off")) {
            continue;
        }
        let active_name = to_active_name(&name);
        let Some(category) = categorize(&active_name.to_lowercase()) else { continue };
        let active_path = path.with_file_name(&active_name);
        let Ok(rel) = active_path.strip_prefix(root) else { continue };
        let rel = normalize_rel(&rel.to_string_lossy());
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
// Rutas de archivo desactivado / activo
// ---------------------------------------------------------------------------

/// `X.bgl` → `X.bgl.off` (nuestra convención al desactivar).
fn disabled_path(active: &Path) -> PathBuf {
    let mut s = active.as_os_str().to_owned();
    s.push(".off");
    PathBuf::from(s)
}
/// `X.bgl` → `X.off` (convención de Aerosoft).
fn alt_disabled_path(active: &Path) -> PathBuf {
    active.with_extension("off")
}

fn current_disabled(addon_dir: &Path, rel: &str) -> Option<PathBuf> {
    let active = addon_dir.join(rel);
    let a = disabled_path(&active);
    if a.exists() {
        return Some(a);
    }
    let b = alt_disabled_path(&active);
    if b.exists() {
        return Some(b);
    }
    None
}

fn is_enabled(addon_dir: &Path, rel: &str) -> bool {
    addon_dir.join(rel).exists()
}
fn file_exists_any(addon_dir: &Path, rel: &str) -> bool {
    is_enabled(addon_dir, rel) || current_disabled(addon_dir, rel).is_some()
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn pretty_from_basename(bn: &str) -> String {
    bn.trim_end_matches(".bgl")
        .replace(['_', '-'], " ")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Construcción / reconciliación del manifiesto
// ---------------------------------------------------------------------------

/// Construye la config a partir de (a) la nota de SceneryAddons resuelta
/// contra el índice de archivos reales y (b) el escaneo local como
/// relleno. `None` si no hay nada.
fn build_config(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    note_text: Option<&str>,
) -> Option<PerfConfig> {
    let index = index_bgls(addon_dir);
    let mut options: Vec<PerfOption> = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut note_opts = 0usize;

    // 1) Opciones de la NOTA (autoritativas, una por línea "Remove …").
    if let Some(text) = note_text {
        for note in parse_note_options(text) {
            let mut files = Vec::new();
            for bn in &note.files {
                if let Some(path) = index.get(bn) {
                    if !used.contains(path) && !files.contains(path) {
                        files.push(path.clone());
                    }
                }
            }
            if files.is_empty() {
                continue;
            }
            note_opts += 1;
            for f in &files {
                used.insert(f.clone());
            }
            let category = categorize(&note.label.to_lowercase())
                .or_else(|| categorize(&note.files[0]))
                .unwrap_or("clutter")
                .to_string();
            let enabled = files.iter().any(|rel| is_enabled(addon_dir, rel));
            let id = {
                let base = slug(&note.label);
                if base.is_empty() {
                    category.clone()
                } else {
                    base
                }
            };
            options.push(PerfOption {
                id,
                label: note.label.clone(),
                description: description_for(&category).to_string(),
                fps_hint: fps_hint_for(&category).to_string(),
                category,
                files,
                enabled,
            });
        }
    }

    // 2) Escaneo LOCAL por patrón — sólo lo no cubierto por la nota.
    let mut local_opts = 0usize;
    for raw in detect_local(addon_dir) {
        let files: Vec<String> = raw
            .files
            .into_iter()
            .filter(|p| !used.contains(p))
            .collect();
        if files.is_empty() {
            continue;
        }
        local_opts += 1;
        for f in &files {
            used.insert(f.clone());
        }
        let enabled = files.iter().any(|rel| is_enabled(addon_dir, rel));
        options.push(PerfOption {
            id: raw.category.clone(),
            label: default_label(&raw.category).to_string(),
            description: description_for(&raw.category).to_string(),
            fps_hint: fps_hint_for(&raw.category).to_string(),
            category: raw.category,
            files,
            enabled,
        });
    }

    if options.is_empty() {
        return None;
    }
    options.sort_by_key(|o| category_rank(&o.category));
    let source = match (note_opts > 0, local_opts > 0) {
        (true, true) => "mixed",
        (true, false) => "sceneryaddons",
        _ => "local-scan",
    };
    Some(PerfConfig {
        icao,
        folder_name: folder_name.to_string(),
        source: source.to_string(),
        options,
    })
}

fn config_dir(addon_dir: &Path) -> PathBuf {
    addon_dir.join(CONFIG_DIR)
}
fn config_file(addon_dir: &Path) -> PathBuf {
    config_dir(addon_dir).join(CONFIG_FILE)
}

fn write_config(addon_dir: &Path, cfg: &PerfConfig) -> std::io::Result<()> {
    fs::create_dir_all(config_dir(addon_dir))?;
    let json = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".into());
    fs::write(config_file(addon_dir), json)
}

fn read_config_raw(addon_dir: &Path) -> Option<PerfConfig> {
    let bytes = fs::read(config_file(addon_dir)).ok()?;
    serde_json::from_slice::<PerfConfig>(&bytes).ok()
}

fn reconcile(addon_dir: &Path, cfg: &mut PerfConfig) {
    for opt in &mut cfg.options {
        opt.files.retain(|rel| file_exists_any(addon_dir, rel));
        opt.enabled = opt.files.iter().any(|rel| is_enabled(addon_dir, rel));
    }
    cfg.options.retain(|o| !o.files.is_empty());
}

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Badge "optimizable": SÓLO true si ya hay una config escrita (de la
/// página de SceneryAddons o tras un toggle) con archivos reales. No
/// hace escaneo permisivo — el badge refleja Optional Configuration real.
pub fn has_optional_objects(addon_dir: &Path) -> bool {
    read_config_raw(addon_dir).is_some_and(|cfg| {
        cfg.options
            .iter()
            .any(|o| o.files.iter().any(|rel| file_exists_any(addon_dir, rel)))
    })
}

/// Lee la config si existe (reconciliando con disco); si no, hace un
/// escaneo local EN MEMORIA (sin escribir, para que el modal no salga
/// vacío). `None` si no hay objetos opcionales.
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
    build_config(addon_dir, folder_name, icao, None)
}

/// Enriquece la config con la nota "Optional Configuration" del HTML de
/// SceneryAddons + el escaneo local, y la ESCRIBE. Devuelve la config.
pub fn enrich_with_html(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    page_html: &str,
) -> Option<PerfConfig> {
    let desc = extract_description_from_html(page_html);
    let cfg = build_config(addon_dir, folder_name, icao, Some(&desc))?;
    if let Err(e) = write_config(addon_dir, &cfg) {
        tracing::warn!("perf: no se pudo escribir {CONFIG_FILE}: {e}");
    }
    Some(cfg)
}

/// (Escáner masivo) Como `enrich_with_html` pero escribe SÓLO si la
/// página trae una nota "Optional Configuration" real (no si únicamente
/// el escaneo local encontró algo). Así el badge automático refleja sólo
/// los aeropuertos con Optional Configuration en SceneryAddons. Devuelve
/// la config (con ≥1 opción de la nota) o `None`.
pub fn enrich_page_note_only(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    page_html: &str,
) -> Option<PerfConfig> {
    let desc = extract_description_from_html(page_html);
    let cfg = build_config(addon_dir, folder_name, icao, Some(&desc))?;
    if cfg.source == "local-scan" {
        return None; // la página no tenía nota — no marcamos optimizable
    }
    if let Err(e) = write_config(addon_dir, &cfg) {
        tracing::warn!("perf: no se pudo escribir {CONFIG_FILE}: {e}");
    }
    Some(cfg)
}

/// (Descarga) Genera la config tras instalar si tenemos la nota de la
/// página. Sin nota, NO escribe (el badge es para los que tienen
/// Optional Configuration real — los puebla el escáner masivo). Registra
/// siempre en logs.
pub fn generate_on_install(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    description: Option<&str>,
) {
    let Some(desc) = description else {
        tracing::info!("perf: {folder_name} instalado sin nota — config la hará el escáner de SceneryAddons");
        return;
    };
    match build_config(addon_dir, folder_name, icao, Some(desc)) {
        Some(cfg) if cfg.source != "local-scan" => match write_config(addon_dir, &cfg) {
            Ok(()) => tracing::info!(
                "perf: config OK para {folder_name} → {} opción(es)",
                cfg.options.len()
            ),
            Err(e) => tracing::warn!("perf: FALLÓ escribir config de {folder_name}: {e}"),
        },
        _ => tracing::info!("perf: {folder_name} sin 'Optional Configuration' en la nota"),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleResult {
    pub option: PerfOption,
    pub renamed: usize,
}

/// Aplica un toggle a una opción: renombra TODOS sus archivos de forma
/// atómica con rollback. `enable=false` desactiva (gana FPS). Soporta
/// las dos convenciones de desactivado (`.bgl.off` y `.off`).
pub fn apply_toggle(
    addon_dir: &Path,
    option_id: &str,
    enable: bool,
) -> anyhow::Result<ToggleResult> {
    let folder = addon_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut cfg = read_or_generate(addon_dir, &folder, None)
        .ok_or_else(|| anyhow::anyhow!("Este escenario no tiene opciones de rendimiento."))?;
    let idx = cfg
        .options
        .iter()
        .position(|o| o.id == option_id)
        .ok_or_else(|| anyhow::anyhow!("Opción '{option_id}' no encontrada."))?;

    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    for rel in &cfg.options[idx].files {
        let active = addon_dir.join(rel);
        if enable {
            if !active.exists() {
                if let Some(dis) = current_disabled(addon_dir, rel) {
                    plan.push((dis, active));
                }
            }
        } else if active.exists() {
            let dis = disabled_path(&active);
            if !dis.exists() {
                plan.push((active, dis));
            }
        }
    }

    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in &plan {
        match fs::rename(from, to) {
            Ok(()) => done.push((from.clone(), to.clone())),
            Err(e) => {
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

    cfg.options[idx].enabled = enable;
    let renamed = done.len();
    let _ = write_config(addon_dir, &cfg);
    Ok(ToggleResult {
        option: cfg.options.swap_remove(idx),
        renamed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_orbx_katl_note() {
        let note = "\
* Remove Static Cars: placement_full_car_flatten.bgl -> placement_full_car_flatten.bgl.off\n\
* Remove Static Ramp Personnel: placement_full_gse_rampguy_static_flatten.bgl -> placement_full_gse_rampguy_static_flatten.bgl.off\n\
* Remove 3D Passengers: placement_full_passengers_flatten.bgl -> placement_full_passengers_flatten.bgl.off\n\
* Remove Animated Ramp Agents: placement_rampguy_walking_flatten.bgl -> placement_rampguy_walking_flatten.bgl.off\n";
        let opts = parse_note_options(note);
        assert_eq!(opts.len(), 4, "esperaba 4 opciones");
        for o in &opts {
            assert_eq!(o.files.len(), 1, "{}", o.label);
            assert!(o.files[0].ends_with(".bgl"));
            assert!(!o.files[0].contains(".off"));
        }
        assert_eq!(opts[0].label, "Static Cars");
    }

    #[test]
    fn parses_aerosoft_eddf_note_dot_off() {
        // Aerosoft usa `X.bgl -> X.off` y nombres sin ruta.
        let note = "\
Optional Configuration: To switch to simple interiors and improve performance, rename:\n\
EDDF_Placements_interiors_complex.bgl -> EDDF_Placements_interiors_complex.off\n\
EDDF_Placements_interiors_simple.off -> EDDF_Placements_interiors_simple.bgl\n\
Optional Configuration: To remove various objects, rename:\n\
* Remove Extra Clutter (models, streetlights, fences): EDDF_Placements_extra.bgl -> EDDF_Placements_extra.off\n\
* Remove Train Station Interior: EDDF_Placements_interiors_trainstation.bgl -> EDDF_Placements_interiors_trainstation.off\n\
* Remove Terminal Passengers: EDDF_Placements_passengers.bgl -> EDDF_Placements_passengers.off\n\
* Remove Road Traffic: EDDF_Placements_roadtraffic.bgl -> EDDF_Placements_roadtraffic.off\n\
* Remove Service Vehicle Traffic: EDDF_Placements_servicetraffic.bgl -> EDDF_Placements_servicetraffic.off\n\
* Remove Static Aircraft: EDDF_Placements_staticAC.bgl -> EDDF_Placements_staticAC.off\n\
* Remove Aerosoft VDGS Boxes (for use with GSX): EDDF_Placements_VDGS.bgl -> EDDF_Placements_VDGS.off\n";
        let opts = parse_note_options(note);
        // 7 líneas "Remove …" (los 2 de interiores son swap, sin "Remove").
        assert_eq!(opts.len(), 7, "esperaba 7 remociones, hubo {}", opts.len());
        let labels: Vec<&str> = opts.iter().map(|o| o.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.starts_with("Extra Clutter")));
        assert!(labels.contains(&"Terminal Passengers"));
        assert!(labels.contains(&"Road Traffic"));
        // Todos resuelven a 1 basename activo .bgl (sin ruta, sin .off).
        for o in &opts {
            assert_eq!(o.files.len(), 1, "{}", o.label);
            assert!(o.files[0].ends_with(".bgl") && !o.files[0].contains(".off"));
        }
    }

    #[test]
    fn categorizes_known_patterns() {
        assert_eq!(categorize("placement_full_car_flatten.bgl"), Some("static_cars"));
        assert_eq!(categorize("eddf_placements_passengers.bgl"), Some("passengers"));
        assert_eq!(categorize("ymml_airside_gse.bgl"), Some("gse"));
        assert_eq!(categorize("placement_rampguy_walking_flatten.bgl"), Some("animated"));
        assert_eq!(categorize("eddf_placements_roadtraffic.bgl"), Some("road_traffic"));
        assert_eq!(categorize("eddf_placements_staticac.bgl"), Some("static_aircraft"));
        assert_eq!(categorize("eddf_placements_vdgs.bgl"), Some("vdgs"));
        assert_eq!(categorize("ymml_terminal.bgl"), None);
        assert_eq!(categorize("cargo_building.bgl"), None);
    }

    #[test]
    fn to_active_name_handles_both_conventions() {
        assert_eq!(to_active_name("X.bgl"), "X.bgl");
        assert_eq!(to_active_name("X.bgl.off"), "X.bgl");
        assert_eq!(to_active_name("EDDF_Placements_extra.off"), "EDDF_Placements_extra.bgl");
    }
}
