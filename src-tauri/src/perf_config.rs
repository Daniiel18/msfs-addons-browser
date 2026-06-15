//! (v5.0.0) Motor de rendimiento / optimización de FPS por aeropuerto.
//!
//! Muchos sceneries premium (Aerosoft, ORBX…) traen objetos pesados
//! OPCIONALES que se activan/desactivan renombrando archivos `.bgl`. La
//! receta viene en la nota "Optional Configuration" de la página del
//! addon en SceneryAddons.
//!
//! **Multiestado (v5).** Una opción NO siempre apaga archivos: a veces es
//! un *swap* (apaga uno y enciende otro). Por eso cada opción guarda un
//! arreglo de OPERACIONES (`ops`), una por archivo, con la dirección que
//! debe tomar cuando la opción está APLICADA:
//!   · `Disable Passengers completely` → _1.bgl→off, _2.bgl→off
//!   · `Switch from High to Medium Passenger Density` → _1.bgl→off, _2.off→bgl
//!
//! Dos convenciones de "desactivado": `X.bgl.off` (ORBX) y `X.off`
//! (Aerosoft). Detectamos ambas; al desactivar estandarizamos `.bgl.off`.
//!
//! La nota da SÓLO el nombre del archivo; el archivo real vive en
//! subcarpetas. Construimos un índice `basename → ruta real` y resolvemos.
//! El JSON `config/simfleet_perf.json` se escribe sólo con la nota real
//! de la página (o tras un toggle) — el badge refleja eso.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = "config";
const CONFIG_FILE: &str = "simfleet_perf.json";
const SCAN_DEPTH: usize = 8;

/// Una operación de archivo dentro de una opción.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfOp {
    /// Ruta RELATIVA al addon del archivo ACTIVO (`.bgl`). No cambia.
    pub path: String,
    /// Ruta RELATIVA de la forma DESACTIVADA, leída LITERALMENTE de la
    /// nota — no asumimos `.off`. El dev puede usar `.disabled`, `.bak`,
    /// `.HD`, `.SD`, etc. Ej. `…/RJAA_Static.BGL.disabled`.
    #[serde(default)]
    pub disabled_path: String,
    /// Cuando la opción está APLICADA, ¿este archivo debe quedar activo
    /// (`.bgl`)? `false` = desactivado. Para un swap, unos van `true` y
    /// otros `false`.
    pub active_when_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfOption {
    pub id: String,
    /// Para opciones de la NOTA: la etiqueta LITERAL del dev (de la
    /// página). Para el escaneo local: el nombre por categoría en inglés.
    pub label: String,
    pub description: String,
    pub fps_hint: String,
    pub category: String,
    /// true si vino de la nota "Optional Configuration" de la página.
    #[serde(default)]
    pub from_note: bool,
    /// Operaciones de archivo (multiestado).
    pub ops: Vec<PerfOp>,
    /// Estado actual: ¿la opción está APLICADA? (todos los `ops` en su
    /// estado aplicado).
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfConfig {
    pub icao: Option<String>,
    pub folder_name: String,
    /// "sceneryaddons" | "local-scan".
    pub source: String,
    pub options: Vec<PerfOption>,
}

// ---------------------------------------------------------------------------
// Categorización
// ---------------------------------------------------------------------------

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
    {
        return Some("service_traffic");
    }
    if text_lower.contains("ramptraffic")
        || (text_lower.contains("ramp") && text_lower.contains("traffic"))
    {
        return Some("ramp_traffic");
    }
    if text_lower.contains("hangardoor") || text_lower.contains("hangar door") {
        return Some("hangar_doors");
    }
    if text_lower.contains("windfarm") || text_lower.contains("wind farm") {
        return Some("wind_farm");
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
    if (text_lower.contains("_car") || text_lower.contains("car_") || text_lower.contains("cars"))
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
        "static_cars" => "Static cars",
        "ramp_personnel" => "Ramp personnel",
        "passengers" => "3D Passengers",
        "gse" => "Ground equipment (GSE)",
        "animated" => "Animated agents / vehicles",
        "static_aircraft" => "Static aircraft",
        "clutter" => "Clutter / extra objects",
        "road_traffic" => "Road traffic",
        "service_traffic" => "Service vehicle traffic",
        "ramp_traffic" => "Ramp traffic",
        "hangar_doors" => "Hangar doors",
        "wind_farm" => "Wind farms",
        "vdgs" => "VDGS boxes",
        _ => "Optional objects",
    }
}

fn description_for(category: &str) -> &'static str {
    match category {
        "static_cars" => "Removes static cars from parking and airport roads.",
        "ramp_personnel" => "Removes static ramp personnel (3D ground crew).",
        "passengers" => "Removes 3D passengers from terminals and boarding gates.",
        "gse" => "Removes static ground service equipment (tugs, belts, stairs, GPU).",
        "animated" => "Removes ANIMATED agents and vehicles (the heaviest on FPS).",
        "static_aircraft" => "Removes decorative static (parked AI) aircraft.",
        "clutter" => "Removes non-essential detail clutter (models, streetlights, fences).",
        "road_traffic" => "Removes road traffic around the airport.",
        "service_traffic" => "Removes moving service vehicles on the apron.",
        "ramp_traffic" => "Removes ramp traffic on the apron.",
        "hangar_doors" => "Switches animated hangar doors to static (lighter).",
        "wind_farm" => "Switches animated wind farms to static (lighter).",
        "vdgs" => "Disables VDGS info boxes (use with GSX).",
        _ => "Optional scenery objects you can disable to gain FPS.",
    }
}

fn fps_hint_for(category: &str) -> &'static str {
    match category {
        "animated" | "service_traffic" | "ramp_traffic" => "+3–6 FPS",
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
        "ramp_traffic" => 2,
        "passengers" => 3,
        "road_traffic" => 4,
        "static_aircraft" => 5,
        "gse" => 6,
        "static_cars" => 7,
        "clutter" => 8,
        "hangar_doors" => 9,
        "wind_farm" => 10,
        "ramp_personnel" => 11,
        "vdgs" => 12,
        _ => 20,
    }
}

// ---------------------------------------------------------------------------
// Parser de la nota "Optional Configuration"
// ---------------------------------------------------------------------------

// Token de archivo. Captura `x.bgl` con una posible extensión
// secundaria (`x.bgl.off`, `x.bgl.disabled`, `x.bgl.bak`…) O un archivo
// con extensión de desactivado de reemplazo (`x.off`, `x.disabled`…).
// NO asumimos `.off`: los devs usan .disabled/.bak/.HD/.SD/etc.
static FILE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([A-Za-z0-9_\-./\\]+?\.bgl(?:\.[A-Za-z0-9]{1,10})?|[A-Za-z0-9_\-./\\]+?\.(?:off|disabled|disable|bak|hd|sd|inactive))\b",
    )
    .unwrap()
});

/// Extensiones de "desactivado" de reemplazo (sustituyen a `.bgl`).
const DISABLED_EXTS: &[&str] = &[
    "off", "disabled", "disable", "bak", "hd", "sd", "inactive",
];

struct NoteOp {
    /// basename del archivo ACTIVO (`x.bgl`, minúsculas, sin ruta).
    active_basename: String,
    /// Nombre LITERAL de la forma desactivada (con su case original):
    /// `X.BGL.disabled`, `X.off`, `X.bak`… tal cual la nota.
    disabled_filename: String,
    active_when_applied: bool,
}
struct NoteOption {
    label: String,
    ops: Vec<NoteOp>,
}

fn basename_lower(s: &str) -> String {
    s.trim().rsplit(['/', '\\']).next().unwrap_or(s).to_lowercase()
}
fn basename_keepcase(s: &str) -> String {
    s.trim().rsplit(['/', '\\']).next().unwrap_or(s).to_string()
}

/// Nombre ACTIVO (con `.bgl`) en minúsculas, sea `.bgl`, `.bgl.off` o `.off`.
fn active_basename_of(name: &str) -> String {
    let n = basename_lower(name);
    if let Some(s) = n.strip_suffix(".bgl.off") {
        format!("{s}.bgl")
    } else if let Some(s) = n.strip_suffix(".off") {
        format!("{s}.bgl")
    } else {
        n
    }
}

/// ¿El token apunta a la forma ACTIVA (`.bgl`, no `.bgl.off`)?
fn is_active_token(name: &str) -> bool {
    let l = name.to_lowercase();
    l.ends_with(".bgl") && !l.ends_with(".bgl.off")
}

/// Primer token de archivo en un fragmento.
fn first_file(s: &str) -> Option<String> {
    FILE_RE.captures(s).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

/// Convierte un fragmento "A -> B" en una operación. Identifica cuál de
/// los dos tokens es el ACTIVO (`.bgl` exacto) y cuál el DESACTIVADO
/// (literal: `.disabled`, `.off`, `.bak`, `.HD`…). `active_when_applied`
/// = el token activo está a la DERECHA (es el destino del swap).
fn op_from_segment(seg: &str) -> Option<NoteOp> {
    let mut parts = seg.splitn(2, "->");
    let left = first_file(parts.next().unwrap_or(""));
    let right = first_file(parts.next().unwrap_or(""));
    let (active_tok, disabled_tok, active_on_right) = match (left, right) {
        (Some(l), Some(r)) => {
            if is_active_token(&l) {
                (l, r, false) // A.bgl -> A.disabled  (aplicar = desactivar)
            } else if is_active_token(&r) {
                (r, l, true) // A.off -> A.bgl       (aplicar = activar)
            } else {
                (l.clone(), r, false)
            }
        }
        (Some(l), None) => {
            // Token suelto sin "->": si es `.bgl`, desactivar a `.off`.
            if is_active_token(&l) {
                let dis = format!("{l}.off");
                (l, dis, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(NoteOp {
        active_basename: active_basename_of(&active_tok),
        disabled_filename: basename_keepcase(&disabled_tok),
        active_when_applied: active_on_right,
    })
}

/// Limpia el header a la etiqueta literal del dev (quita boilerplate
/// "Optional Configuration:" y el ":" final).
fn clean_label(header: &str) -> String {
    let mut s = header.trim();
    if s.to_lowercase().starts_with("optional configuration") {
        if let Some(pos) = s.find(':') {
            s = s[pos + 1..].trim();
        }
    }
    s.splitn(2, ':').next().unwrap_or(s).trim().to_string()
}

/// Parsea la nota. Maneja una línea (`• Remove X: a.bgl -> a.off`) y
/// multilínea (cabecera `Disable X:` + viñetas). Incluye swaps.
fn parse_note_options(text: &str) -> Vec<NoteOption> {
    let mut out: Vec<NoteOption> = Vec::new();
    let mut cur: Option<NoteOption> = None;

    macro_rules! flush {
        () => {
            if let Some(opt) = cur.take() {
                if !opt.ops.is_empty() {
                    out.push(opt);
                }
            }
        };
    }

    for raw in text.lines() {
        let stripped = raw.trim().trim_start_matches(['•', '*', '-', '·', ' ']).trim();
        if stripped.is_empty() {
            flush!();
            continue;
        }
        let lower = stripped.to_lowercase();
        let has_verb = lower.contains("remove")
            || lower.contains("disable")
            || lower.contains("delete")
            || lower.contains("switch");
        let has_file = lower.contains(".bgl") || lower.contains(".off");

        // Cabecera de opción: verbo + ":".
        if has_verb && stripped.contains(':') {
            flush!();
            let label = clean_label(stripped);
            let mut ops = Vec::new();
            // Caso de una línea: la op va tras el ":".
            if let Some(after) = stripped.splitn(2, ':').nth(1) {
                if let Some(op) = op_from_segment(after) {
                    ops.push(op);
                }
            }
            cur = Some(NoteOption { label, ops });
            continue;
        }

        // Viñeta de continuación bajo la cabecera actual.
        if has_file {
            if let Some(opt) = cur.as_mut() {
                if let Some(op) = op_from_segment(stripped) {
                    // dedup por basename
                    if !opt.ops.iter().any(|o| o.active_basename == op.active_basename) {
                        opt.ops.push(op);
                    }
                }
            }
        }
    }
    flush!();
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
// Índice de `.bgl` en disco + escaneo local
// ---------------------------------------------------------------------------

fn to_active_name(name: &str) -> String {
    let lower = name.to_lowercase();
    // Append-style: `X.bgl.<dis>` → `X.bgl` (preserva case; filenames ASCII).
    if let Some(pos) = lower.find(".bgl.") {
        return name[..pos + 4].to_string();
    }
    if lower.ends_with(".bgl") {
        return name.to_string();
    }
    // Replace-style: `X.<dis>` → `X.bgl` si <dis> es una ext de desactivado.
    if let Some(dot) = lower.rfind('.') {
        if DISABLED_EXTS.contains(&&lower[dot + 1..]) {
            return format!("{}.bgl", &name[..dot]);
        }
    }
    name.to_string()
}

/// ¿El archivo es un `.bgl` (activo) o una forma desactivada de uno?
fn is_bgl_related(name_lower: &str) -> bool {
    name_lower.contains(".bgl")
        || name_lower
            .rsplit('.')
            .next()
            .map(|ext| DISABLED_EXTS.contains(&ext))
            .unwrap_or(false)
}

fn normalize_rel(s: &str) -> String {
    s.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

/// Ruta relativa de un archivo HERMANO (misma carpeta) del activo.
fn sibling_rel(active_rel: &str, filename: &str) -> String {
    match active_rel.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{filename}"),
        None => filename.to_string(),
    }
}

/// `basename_activo(lower) → ruta relativa ACTIVA`.
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
        if !is_bgl_related(&lower) {
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
        if !is_bgl_related(&lower) {
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
// Archivos activo / desactivado
// ---------------------------------------------------------------------------

fn disabled_path(active: &Path) -> PathBuf {
    let mut s = active.as_os_str().to_owned();
    s.push(".off");
    PathBuf::from(s)
}
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
fn is_active(addon_dir: &Path, rel: &str) -> bool {
    addon_dir.join(rel).exists()
}
/// ¿Existe el archivo de la op en disco, sea activo o en su forma
/// desactivada (la explícita de la nota o las convenciones conocidas)?
fn op_exists(addon_dir: &Path, op: &PerfOp) -> bool {
    if is_active(addon_dir, &op.path) {
        return true;
    }
    if !op.disabled_path.is_empty() && addon_dir.join(&op.disabled_path).exists() {
        return true;
    }
    current_disabled(addon_dir, &op.path).is_some()
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

/// ¿La opción está APLICADA? (todos los ops en su estado aplicado).
fn compute_applied(addon_dir: &Path, ops: &[PerfOp]) -> bool {
    !ops.is_empty()
        && ops
            .iter()
            .all(|op| is_active(addon_dir, &op.path) == op.active_when_applied)
}

// ---------------------------------------------------------------------------
// Construcción del manifiesto
// ---------------------------------------------------------------------------

fn build_config(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    note_text: Option<&str>,
) -> Option<PerfConfig> {
    let index = index_bgls(addon_dir);
    let mut options: Vec<PerfOption> = Vec::new();
    let mut note_count = 0usize;

    // 1) Opciones de la NOTA (autoritativas, multiestado).
    if let Some(text) = note_text {
        for note in parse_note_options(text) {
            let mut ops: Vec<PerfOp> = Vec::new();
            for nop in &note.ops {
                if let Some(path) = index.get(&nop.active_basename) {
                    if !ops.iter().any(|o| &o.path == path) {
                        ops.push(PerfOp {
                            path: path.clone(),
                            disabled_path: sibling_rel(path, &nop.disabled_filename),
                            active_when_applied: nop.active_when_applied,
                        });
                    }
                }
            }
            if ops.is_empty() {
                continue;
            }
            note_count += 1;
            let category = categorize(&note.label.to_lowercase())
                .or_else(|| categorize(&note.ops[0].active_basename))
                .unwrap_or("clutter")
                .to_string();
            let id = {
                let base = slug(&note.label);
                if base.is_empty() {
                    format!("opt-{}", options.len())
                } else {
                    base
                }
            };
            let applied = compute_applied(addon_dir, &ops);
            options.push(PerfOption {
                id,
                label: note.label.clone(),
                description: description_for(&category).to_string(),
                fps_hint: fps_hint_for(&category).to_string(),
                category,
                from_note: true,
                ops,
                applied,
            });
        }
    }

    // 2) Escaneo LOCAL — SÓLO si la nota no aportó nada (sin nota, o nota
    //    sin opciones resolubles). Cuando hay nota real, ésta manda y NO
    //    añadimos extras locales inventados.
    if note_count == 0 {
        for raw in detect_local(addon_dir) {
            let ops: Vec<PerfOp> = raw
                .files
                .into_iter()
                .map(|path| PerfOp {
                    disabled_path: format!("{path}.off"), // local: convención .bgl.off
                    path,
                    active_when_applied: false, // local = "quitar"
                })
                .collect();
            if ops.is_empty() {
                continue;
            }
            let applied = compute_applied(addon_dir, &ops);
            options.push(PerfOption {
                id: raw.category.clone(),
                label: default_label(&raw.category).to_string(),
                description: description_for(&raw.category).to_string(),
                fps_hint: fps_hint_for(&raw.category).to_string(),
                category: raw.category,
                from_note: false,
                ops,
                applied,
            });
        }
    }

    if options.is_empty() {
        return None;
    }
    options.sort_by_key(|o| category_rank(&o.category));
    let source = if note_count > 0 { "sceneryaddons" } else { "local-scan" };
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
        opt.ops.retain(|op| op_exists(addon_dir, op));
        opt.applied = compute_applied(addon_dir, &opt.ops);
    }
    cfg.options.retain(|o| !o.ops.is_empty());
}

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Badge "optimizable": true sólo si hay config escrita (de la página o
/// tras un toggle) con ops sobre archivos reales.
pub fn has_optional_objects(addon_dir: &Path) -> bool {
    read_config_raw(addon_dir).is_some_and(|cfg| {
        cfg.options
            .iter()
            .any(|o| o.ops.iter().any(|op| op_exists(addon_dir, op)))
    })
}

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

/// Enriquece con la nota de la página + (si no hay nota) escaneo local, y
/// ESCRIBE. Para el botón del modal.
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

/// (Escáner masivo) Escribe SÓLO si la página trae nota real.
pub fn enrich_page_note_only(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    page_html: &str,
) -> Option<PerfConfig> {
    let desc = extract_description_from_html(page_html);
    let cfg = build_config(addon_dir, folder_name, icao, Some(&desc))?;
    if cfg.source == "local-scan" {
        return None;
    }
    if let Err(e) = write_config(addon_dir, &cfg) {
        tracing::warn!("perf: no se pudo escribir {CONFIG_FILE}: {e}");
    }
    Some(cfg)
}

pub fn generate_on_install(
    addon_dir: &Path,
    folder_name: &str,
    icao: Option<String>,
    description: Option<&str>,
) {
    let Some(desc) = description else {
        tracing::info!("perf: {folder_name} instalado sin nota — config la hará el escáner");
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

/// Aplica/revierte una opción multiestado de forma atómica con rollback.
/// `apply=true` pone cada archivo en su estado aplicado; `false` lo
/// revierte. Soporta `.bgl.off` y `.off`.
pub fn apply_toggle(
    addon_dir: &Path,
    option_id: &str,
    apply: bool,
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

    // Plan: por cada op, el destino activo según apply/dirección. Usa la
    // ruta DESACTIVADA explícita de la nota (`.disabled`, `.off`, `.bak`…),
    // no una asumida.
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    for op in &cfg.options[idx].ops {
        let active = addon_dir.join(&op.path);
        let explicit_dis = if op.disabled_path.is_empty() {
            disabled_path(&active)
        } else {
            addon_dir.join(&op.disabled_path)
        };
        let want_active = if apply {
            op.active_when_applied
        } else {
            !op.active_when_applied
        };
        if want_active {
            // queremos `.bgl`: si está desactivado, restaurarlo desde su
            // forma desactivada (explícita o, como respaldo, las conocidas).
            if !active.exists() {
                let from = if explicit_dis.exists() {
                    Some(explicit_dis)
                } else {
                    current_disabled(addon_dir, &op.path)
                };
                if let Some(from) = from {
                    plan.push((from, active));
                }
            }
        } else {
            // queremos desactivado: renombrar al destino LITERAL de la nota.
            if active.exists() && !explicit_dis.exists() {
                plan.push((active, explicit_dis));
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

    let renamed = done.len();
    cfg.options[idx].applied = apply;
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
    fn parses_orbx_katl_note_single_line() {
        let note = "\
* Remove Static Cars: placement_full_car_flatten.bgl -> placement_full_car_flatten.bgl.off\n\
* Remove 3D Passengers: placement_full_passengers_flatten.bgl -> placement_full_passengers_flatten.bgl.off\n";
        let opts = parse_note_options(note);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].label, "Remove Static Cars");
        assert_eq!(opts[0].ops.len(), 1);
        // Remoción pura: al aplicar, el archivo queda desactivado.
        assert!(!opts[0].ops[0].active_when_applied);
        assert_eq!(opts[0].ops[0].active_basename, "placement_full_car_flatten.bgl");
    }

    #[test]
    fn parses_aerosoft_engm_multistate() {
        let note = "\
Optional Configuration: To change various options, rename:\n\
\n\
Switch from High to Medium Passenger Density:\n\
• ENGM_Passengers_1.bgl -> ENGM_Passengers_1.off\n\
• ENGM_Passengers_2.off -> ENGM_Passengers_2.bgl\n\
\n\
Disable Passengers completely:\n\
• ENGM_Passengers_1.bgl -> ENGM_Passengers_1.off\n\
• ENGM_Passengers_2.bgl -> ENGM_Passengers_2.off\n\
\n\
Switch from Animated to Static Wind Farms:\n\
• ENGM_WindFarm_1.bgl -> ENGM_WindFarm_1.off\n\
• ENGM_WindFarm_2.off -> ENGM_WindFarm_2.bgl\n\
\n\
Disable Norse Static Aircraft on Taxiway U:\n\
• ENGM_StaticAircraft_Norse.bgl -> ENGM_StaticAircraft_Norse.off\n";
        let opts = parse_note_options(note);
        let labels: Vec<&str> = opts.iter().map(|o| o.label.as_str()).collect();
        // 4 opciones, INCLUIDOS los swaps (texto literal).
        assert_eq!(opts.len(), 4, "{labels:?}");
        assert!(labels.contains(&"Switch from High to Medium Passenger Density"));
        assert!(labels.contains(&"Disable Passengers completely"));
        assert!(labels.contains(&"Switch from Animated to Static Wind Farms"));

        // Swap de pasajeros: _1 apagado al aplicar, _2 encendido al aplicar.
        let sw = opts.iter().find(|o| o.label.starts_with("Switch from High")).unwrap();
        assert_eq!(sw.ops.len(), 2);
        let op1 = sw.ops.iter().find(|o| o.active_basename.contains("_1")).unwrap();
        let op2 = sw.ops.iter().find(|o| o.active_basename.contains("_2")).unwrap();
        assert!(!op1.active_when_applied, "_1 debe apagarse al aplicar");
        assert!(op2.active_when_applied, "_2 debe encenderse al aplicar");

        // Disable completely: ambos apagados al aplicar.
        let dis = opts.iter().find(|o| o.label == "Disable Passengers completely").unwrap();
        assert_eq!(dis.ops.len(), 2);
        assert!(dis.ops.iter().all(|o| !o.active_when_applied));
    }

    #[test]
    fn parses_explicit_disabled_extensions() {
        // RJAA usa `.disabled`; el destino NO se asume `.off`.
        let note = "\
• Remove Static Aircraft: RJAA_Static.BGL -> RJAA_Static.BGL.disabled\n\
• Remove 3D People: RJAA_People.BGL -> RJAA_People.BGL.disabled\n";
        let opts = parse_note_options(note);
        assert_eq!(opts.len(), 2);
        let o = &opts[0];
        assert_eq!(o.label, "Remove Static Aircraft");
        assert_eq!(o.ops.len(), 1);
        assert_eq!(o.ops[0].active_basename, "rjaa_static.bgl");
        assert_eq!(o.ops[0].disabled_filename, "RJAA_Static.BGL.disabled");
        assert!(!o.ops[0].active_when_applied);

        // JFK iniBuilds: swap con `.disabled` en ambas direcciones.
        let jfk = "\
Optional Configuration: To remove Terminals – Detailed interiors, rename:\n\
• kjfk-modellib-termPC.bgl -> kjfk-modellib-termPC.disabled\n\
• kjfk-modellib-termXB.disabled -> kjfk-modellib-termXB.bgl\n";
        let jopts = parse_note_options(jfk);
        assert_eq!(jopts.len(), 1);
        let j = &jopts[0];
        assert_eq!(j.ops.len(), 2);
        let pc = j.ops.iter().find(|o| o.active_basename.contains("termpc")).unwrap();
        assert_eq!(pc.disabled_filename, "kjfk-modellib-termPC.disabled");
        assert!(!pc.active_when_applied); // PC se desactiva al aplicar
        let xb = j.ops.iter().find(|o| o.active_basename.contains("termxb")).unwrap();
        assert!(xb.active_when_applied); // XB se activa al aplicar
    }

    #[test]
    fn categorizes_known_patterns() {
        assert_eq!(categorize("engm_passengers_1.bgl"), Some("passengers"));
        assert_eq!(categorize("engm_ramptraffic_1.bgl"), Some("ramp_traffic"));
        assert_eq!(categorize("engm_windfarm_1.bgl"), Some("wind_farm"));
        assert_eq!(categorize("engm_hangardoors_1.bgl"), Some("hangar_doors"));
        assert_eq!(categorize("engm_staticaircraft_norse.bgl"), Some("static_aircraft"));
        assert_eq!(categorize("ymml_terminal.bgl"), None);
    }

    #[test]
    fn to_active_name_handles_both_conventions() {
        assert_eq!(to_active_name("X.bgl"), "X.bgl");
        assert_eq!(to_active_name("X.bgl.off"), "X.bgl");
        assert_eq!(to_active_name("ENGM_Passengers_2.off"), "ENGM_Passengers_2.bgl");
    }
}
