//! (04/11) Etapa 9 — manuales, guías y notas que traen los addons.
//!
//! Un paquete de Community trae muchos archivos con pinta de documento que no
//! lo son: licencias de librerías, `placeholder.txt`, datos del EFB, cartas o
//! páginas internas. Aquí sólo entra lo que un piloto querría abrir: los PDF y
//! los readme, changelog y similares.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;
use walkdir::{DirEntry, WalkDir};

/// Recorrer los paquetes lee muchas carpetas: no se repite en cada apertura.
const TTL: Duration = Duration::from_secs(600);
/// Los manuales no van más hondo que `Data/Documentation/Aircraft/x.pdf`.
const MAX_DEPTH: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonDoc {
    pub folder: String,
    /// Nombre del archivo sin extensión.
    pub name: String,
    pub path: String,
    /// `pdf` | `text` | `web`.
    pub kind: String,
    pub size_bytes: u64,
}

/// Carpetas que nunca guardan documentación para el piloto.
fn skip_dir(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "simobjects", "scenery", "html_ui", "materiallibs", "modellib", "behaviors", "wasm", "license", "licenses",
        "legal", "navdatapro", "contentinfo", "cookedcomps", "ps", "printer",
    ];
    const PREFIX: &[&str] = &["model", "texture", "config", "sound", "effect", "panel"];
    EXACT.contains(&name) || PREFIX.iter().any(|p| name.starts_with(p))
}

fn keep_entry(e: &DirEntry) -> bool {
    e.depth() == 0 || !e.file_type().is_dir() || !skip_dir(&e.file_name().to_string_lossy().to_lowercase())
}

/// Tipo de documento, o `None` si el archivo no interesa.
fn doc_kind(file: &str) -> Option<&'static str> {
    const NOT_DOCS: &[&str] = &[
        "license", "licence", "eula", "placeholder", "aerohub", "third party", "thirdparty", "copyright", "notices",
    ];
    const TEXT_DOCS: &[&str] = &[
        "readme", "read me", "read-me", "changelog", "change log", "changes", "release note", "release_note",
        "manual", "guide", "instruction", "install", "handbook", "tutorial", "faq",
    ];
    let name = file.to_lowercase();
    if NOT_DOCS.iter().any(|w| name.contains(w)) {
        return None;
    }
    let (stem, ext) = name.rsplit_once('.')?;
    let named_as_doc = TEXT_DOCS.iter().any(|w| stem.contains(w));
    match ext {
        "pdf" => Some("pdf"),
        "txt" | "md" | "rtf" | "doc" | "docx" if named_as_doc => Some("text"),
        "html" | "htm" if named_as_doc => Some("web"),
        _ => None,
    }
}

/// Documentos de un paquete, ordenados por nombre.
pub fn docs_in(folder: &str, root: &Path) -> Vec<AddonDoc> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root).max_depth(MAX_DEPTH).into_iter().filter_entry(keep_entry).flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let file = entry.file_name().to_string_lossy();
        let Some(kind) = doc_kind(&file) else { continue };
        let name = Path::new(file.as_ref())
            .file_stem()
            .map(|s| s.to_string_lossy().trim().to_string())
            .unwrap_or_default();
        out.push(AddonDoc {
            folder: folder.to_string(),
            name,
            path: entry.path().to_string_lossy().into_owned(),
            kind: kind.to_string(),
            size_bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
        });
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

type Cache = Mutex<Option<(Instant, Vec<AddonDoc>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Tras un escaneo puede haber addons nuevos o actualizados.
pub fn invalidate() {
    *cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Documentos de todos los addons instalados.
pub async fn all(pool: &SqlitePool) -> Result<Vec<AddonDoc>> {
    if let Some((at, list)) = cache().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < TTL {
            return Ok(list.clone());
        }
    }
    let packs: Vec<(String, String)> = sqlx::query_as("SELECT folder_name, install_path FROM community_packages")
        .fetch_all(pool)
        .await?;
    let list = tokio::task::spawn_blocking(move || {
        packs.iter().flat_map(|(folder, path)| docs_in(folder, Path::new(path))).collect::<Vec<_>>()
    })
    .await?;
    *cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), list.clone()));
    Ok(list)
}

/// (07/11) Etapa F: hasta aquí se muestra un texto dentro de SimFleet.
const MAX_PREVIEW: u64 = 2 * 1024 * 1024;

/// Cómo se ve un documento dentro de SimFleet.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocPreview {
    /// `pdf` | `text` | `markdown` | `external` (se abre con su programa).
    pub kind: String,
    pub text: Option<String>,
}

impl DocPreview {
    fn outside() -> Self {
        Self { kind: "external".into(), text: None }
    }
}

/// Texto de un archivo de manual: UTF-8 (con BOM o sin él), UTF-16 con BOM o,
/// si no, ANSI, que es lo que dejan muchos readme antiguos.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], read: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| read([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

fn preview_of(doc: &AddonDoc) -> Result<DocPreview> {
    let path = Path::new(&doc.path);
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let kind = match ext.as_str() {
        "pdf" => return Ok(DocPreview { kind: "pdf".into(), text: None }),
        "txt" => "text",
        "md" => "markdown",
        _ => return Ok(DocPreview::outside()),
    };
    if std::fs::metadata(path)?.len() > MAX_PREVIEW {
        return Ok(DocPreview::outside());
    }
    Ok(DocPreview { kind: kind.into(), text: Some(decode(&std::fs::read(path)?)) })
}

/// Vista previa de un documento de la lista: nunca de un archivo cualquiera.
/// El visor de PDF lo carga desde el disco, así que ese archivo se autoriza.
pub async fn preview(app: &tauri::AppHandle, pool: &SqlitePool, path: &str) -> Result<DocPreview> {
    use tauri::Manager;
    let docs = all(pool).await?;
    let doc = docs.iter().find(|d| d.path == path).ok_or_else(|| anyhow::anyhow!("not_found"))?;
    let out = preview_of(doc)?;
    if out.kind == "pdf" {
        app.asset_protocol_scope().allow_file(Path::new(&doc.path))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manuals_are_read_in_their_own_encoding() {
        assert_eq!(decode("añade".as_bytes()), "añade");
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'h', b'i']), "hi");
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode(&[0xFE, 0xFF, 0, b'h', 0, b'i']), "hi");
        assert_eq!(decode(&[b'a', 0xF1, b'o']), "año", "ANSI");
    }

    #[test]
    fn each_document_opens_where_it_can() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = |file: &str, body: Vec<u8>| {
            let path = tmp.path().join(file);
            std::fs::write(&path, body).unwrap();
            let d = AddonDoc {
                folder: "x".into(),
                name: file.into(),
                path: path.to_string_lossy().into_owned(),
                kind: String::new(),
                size_bytes: 0,
            };
            preview_of(&d).unwrap()
        };
        assert_eq!(doc("readme.txt", b"hola".to_vec()), DocPreview { kind: "text".into(), text: Some("hola".into()) });
        assert_eq!(doc("readme.md", b"# hola".to_vec()).kind, "markdown");
        assert_eq!(doc("manual.pdf", b"%PDF".to_vec()), DocPreview { kind: "pdf".into(), text: None });
        assert_eq!(doc("manual.docx", b"x".to_vec()).kind, "external");
        assert_eq!(doc("guide.html", b"x".to_vec()).kind, "external");
        let big = vec![b'a'; MAX_PREVIEW as usize + 1];
        assert_eq!(doc("grande.txt", big).kind, "external", "demasiado para mostrarlo aquí");
    }

    fn found(files: &[&str]) -> Vec<String> {
        let tmp = tempfile::tempdir().unwrap();
        for f in files {
            let p = tmp.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let root = tmp.path().to_string_lossy().replace('\\', "/");
        let mut got: Vec<String> = docs_in("addon", tmp.path())
            .into_iter()
            .map(|d| d.path.replace('\\', "/").trim_start_matches(&format!("{root}/")).to_string())
            .collect();
        got.sort();
        got
    }

    /// Casos sacados de un Community real.
    #[test]
    fn only_what_a_pilot_would_open_is_listed() {
        let kept = [
            "Doc/WFSS Shanghai Pudong Intl.pdf",
            "RJAA Tokyo Narita MSFS Manual.pdf",
            "CUSTOMIZE/readme.md",
            "Data/Documentation/1.  MSFS 2024 User Guide.pdf",
            "Documentation/Manuals.pdf",
            "GSX Profile/GSX VDGS/README.txt",
            "README - IMPORTANT.txt",
            "Release Notes.txt",
            "changelog.md",
        ];
        let dropped = [
            "AeroHub Readme.txt",
            "LICENSE.txt",
            "License/json.txt",
            "Info/Readme_EULA.txt",
            "Config_738/EFB/EFB Database/BGLMapDef.txt",
            "Config_77W/Flightplans/placeholder.txt",
            "config/a32nx/a320-251n/README.md",
            "ContentInfo/navigraph-navdata/cycle_info.txt",
            "Data/Printer/ATIS_METAR.txt",
            "EffectLibs/contrail/VisualEffectLib/README.txt",
            "NavDataPro/EDDB/2558.pdf",
            "SimObjects/Airplanes/X/manual.pdf",
            "Data/v.txt",
            "ps/{14293394-0041-4A90-B239-83617B5E633F}.html",
            "version_baw-pmdg-737-msfs2024.txt",
        ];
        let all: Vec<&str> = kept.iter().chain(dropped.iter()).copied().collect();
        let mut expected: Vec<String> = kept.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(found(&all), expected);
    }

    #[test]
    fn names_and_kinds_are_ready_to_show() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Manual.PDF"), "abc").unwrap();
        std::fs::write(tmp.path().join("readme.html"), "x").unwrap();
        std::fs::write(tmp.path().join("page.html"), "x").unwrap();
        let docs = docs_in("a", tmp.path());
        let got: Vec<(&str, &str, u64)> = docs.iter().map(|d| (d.name.as_str(), d.kind.as_str(), d.size_bytes)).collect();
        assert_eq!(got, vec![("Manual", "pdf", 3), ("readme", "web", 1)]);
    }

    #[test]
    fn a_missing_folder_has_no_documents() {
        assert!(docs_in("a", Path::new(r"Z:\no\existe")).is_empty());
    }
}
