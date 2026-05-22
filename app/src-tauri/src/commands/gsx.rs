use std::path::{Path, PathBuf};

use crate::gsx::{self, GsxProfile};
use crate::AppState;

/// Devuelve los perfiles GSX Pro disponibles para un ICAO (vacío si no
/// hay coincidencias). El comando es idempotente y respeta el caché de
/// 24h en SQLite — una segunda llamada con el mismo ICAO no toca la
/// red. ICAOs vacíos o blancos se cortocircuitan a `[]`.
#[tauri::command]
pub async fn gsx_lookup(
    icao: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GsxProfile>, String> {
    let trimmed = icao.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    gsx::lookup_with_cache(&state.gsx, &state.db, trimmed)
        .await
        .map_err(|e| e.to_string())
}

/// Lista los ICAOs que tienen al menos un perfil GSX instalado
/// localmente en `%APPDATA%\Virtuali\GSX\MSFS`. Se usa para
/// mostrar un check en cada card de escenario cuando hay perfil
/// instalado.
///
/// Estrategia de detección:
///   1. **Nombre del archivo**: la mayoría siguen `ICAO-devname.ini`.
///   2. **`afcad_path` dentro del .ini**: si el nombre no empieza
///      por 4 letras + guión, abrimos el .ini y buscamos un
///      `MHTG.bgl` (cualquier ICAO seguido de `.bgl`).
#[tauri::command]
pub async fn gsx_list_installed_icaos() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(|| {
        let folder = gsx_profiles_folder().ok_or_else(|| {
            "No se pudo resolver %APPDATA%\\Virtuali\\GSX\\MSFS".to_string()
        })?;
        if !folder.is_dir() {
            return Ok::<Vec<String>, String>(Vec::new());
        }
        let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let dir = std::fs::read_dir(&folder).map_err(|e| e.to_string())?;
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case("ini") {
                continue;
            }

            // Camino 1 — prefijo del nombre. `MHTG-pdqiuo.ini` → MHTG.
            if let Some(icao) = leading_icao_from_filename(stem) {
                out.insert(icao);
                continue;
            }

            // Camino 2 — leer el contenido. Buscar ICAO.bgl.
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(icao) = icao_from_afcad_path(&content) {
                    out.insert(icao);
                }
            }
        }
        Ok(out.into_iter().collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// (v2.0.0) Resultado de instalar perfil(es) GSX desde un archivo.
/// Si la fuente era un solo .ini/.py, `installed_files.len() == 1`
/// y `archive_kind = "single"`. Si era .zip/.rar, refleja cuántos
/// `.ini`/`.py` válidos se extrajeron + copiaron.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GsxInstallReport {
    pub archive_kind: String,
    pub installed_files: Vec<String>,
    /// Lista de archivos del archivo que se IGNORARON (no eran .ini/.py).
    pub skipped_files: Vec<String>,
}

/// Instala perfil(es) GSX desde un archivo. Acepta:
///   · `.ini`/`.py` sueltos — se copian tal cual (1 perfil).
///   · `.zip` — extrae a temp, escanea recursivo y copia cada .ini/.py.
///   · `.rar` — idem usando `unrar`.
///
/// Reglas:
///   · El nombre original del archivo se conserva en destino — GSX
///     no es estricto con la extensión, sólo necesita que el ICAO
///     esté en el filename o en `afcad_path` del .ini.
///   · Si un archivo ya existe en destino, se sobreescribe
///     (re-instalación intencionada).
///   · Se ignoran subdirectorios al copiar — todos los .ini/.py van
///     directamente al folder raíz `Virtuali\GSX\MSFS\`. GSX no usa
///     subfolders.
///
/// Razón de empezar a soportar .zip/.rar (v2.0.0): el usuario reportó
/// "no soporta .rar cuando son perfiles de gsx comprimidos". Antes
/// fallaba con "sólo .ini y .py son válidos" — ahora explota el
/// archivo y copia lo que encuentre.
#[tauri::command]
pub async fn gsx_install_profile(
    source_path: String,
) -> Result<GsxInstallReport, String> {
    tokio::task::spawn_blocking(move || {
        let src = Path::new(&source_path);
        if !src.is_file() {
            return Err(format!("No existe el archivo {}", src.display()));
        }
        let ext = src
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let folder = gsx_profiles_folder().ok_or_else(|| {
            "No se pudo resolver %APPDATA%\\Virtuali\\GSX\\MSFS".to_string()
        })?;
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

        match ext.as_str() {
            "ini" | "py" => {
                let file_name = src
                    .file_name()
                    .ok_or_else(|| "archivo sin nombre".to_string())?;
                let dest = folder.join(file_name);
                std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
                Ok(GsxInstallReport {
                    archive_kind: "single".to_string(),
                    installed_files: vec![dest.to_string_lossy().into_owned()],
                    skipped_files: Vec::new(),
                })
            }
            "zip" => install_from_zip(src, &folder),
            "rar" => install_from_rar(src, &folder),
            other => Err(format!(
                "Extensión no soportada: .{} (acepta .ini, .py, .zip, .rar)",
                other
            )),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Extrae un .zip a un tempdir y copia todos los .ini/.py encontrados
/// (incluso en subfolders) al destino. Devuelve el reporte con la
/// lista de archivos instalados y los que ignoró.
fn install_from_zip(src: &Path, dest_folder: &Path) -> Result<GsxInstallReport, String> {
    let file = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    archive
        .extract(temp.path())
        .map_err(|e| format!("Error extrayendo .zip: {e}"))?;
    collect_and_copy_profiles(temp.path(), dest_folder, "zip")
}

/// Idem para .rar usando el crate `unrar`. La librería se hace cargo
/// de volúmenes múltiples (`foo.part1.rar` + `foo.part2.rar`).
fn install_from_rar(src: &Path, dest_folder: &Path) -> Result<GsxInstallReport, String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    // `unrar::Archive` extrae todo al directorio actual; cambiamos
    // a `temp.path()` via `process()` que acepta destination path.
    let archive = unrar::Archive::new(src)
        .open_for_processing()
        .map_err(|e| format!("Error abriendo .rar: {e}"))?;
    let mut iter = archive;
    loop {
        let header = match iter.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => return Err(format!("Error leyendo .rar: {e}")),
        };
        iter = header
            .extract_with_base(temp.path())
            .map_err(|e| format!("Error extrayendo .rar: {e}"))?;
    }
    collect_and_copy_profiles(temp.path(), dest_folder, "rar")
}

/// Walks recursive sobre `extracted_root` buscando .ini/.py y los
/// copia a `dest_folder` con su filename original. Subdirs se ignoran
/// (sólo nos interesa el filename final). Si un archivo aparece varias
/// veces con el mismo nombre en distintas subcarpetas, el último gana
/// (no es común en profile packs reales).
fn collect_and_copy_profiles(
    extracted_root: &Path,
    dest_folder: &Path,
    kind: &str,
) -> Result<GsxInstallReport, String> {
    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    for entry in walkdir::WalkDir::new(extracted_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if ext != "ini" && ext != "py" {
            if !name.is_empty() {
                skipped.push(name);
            }
            continue;
        }
        let dest = dest_folder.join(p.file_name().unwrap());
        std::fs::copy(p, &dest).map_err(|e| {
            format!("Error copiando {}: {}", p.display(), e)
        })?;
        installed.push(dest.to_string_lossy().into_owned());
    }
    if installed.is_empty() {
        return Err(format!(
            "El archivo .{} no contiene perfiles GSX (.ini/.py)",
            kind
        ));
    }
    Ok(GsxInstallReport {
        archive_kind: kind.to_string(),
        installed_files: installed,
        skipped_files: skipped,
    })
}

/// (v1.1.4) Lee un archivo de texto plano y lo devuelve como string.
/// Usado por el modal "Importar inventario" para leer los CSV/TXT/JSON
/// que el usuario seleccionó con el file picker — sin la dep de
/// tauri-plugin-fs (que añadiría +500KB al bundle por sólo esto).
///
/// Cap de seguridad: 10MB. Un export normal tiene <100KB. Cualquier
/// archivo mayor probablemente no es un export válido.
#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let p = Path::new(&path);
        if !p.is_file() {
            return Err(format!("No existe el archivo {}", p.display()));
        }
        let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
        if meta.len() > 10 * 1024 * 1024 {
            return Err(format!(
                "Archivo muy grande ({} bytes); cap 10MB",
                meta.len()
            ));
        }
        std::fs::read_to_string(p).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `%APPDATA%\Virtuali\GSX\MSFS` resuelto en runtime. Devuelve None
/// si `APPDATA` no existe (no debería pasar en Windows real).
fn gsx_profiles_folder() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Virtuali").join("GSX").join("MSFS"))
}

/// `MHTG-pdqiuo` → `Some("MHTG")`. Acepta 4 letras ASCII seguidas
/// de un separador no-alfanumérico (`-`, `_`, ` `, `.`).
fn leading_icao_from_filename(stem: &str) -> Option<String> {
    let bytes = stem.as_bytes();
    if bytes.len() < 5 {
        return None;
    }
    if !bytes[..4].iter().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    if !matches!(bytes[4], b'-' | b'_' | b' ' | b'.') {
        return None;
    }
    Some(stem[..4].to_ascii_uppercase())
}

/// Busca un `afcad_path = ...XXXX.bgl` dentro del contenido del .ini
/// y extrae XXXX. Fallback cuando el filename no expone el ICAO.
fn icao_from_afcad_path(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let key = "afcad_path";
    let idx = lower.find(key)?;
    let line_end = lower[idx..]
        .find('\n')
        .map(|n| idx + n)
        .unwrap_or(content.len());
    let line = &content[idx..line_end];
    // Busca cualquier secuencia "XXXX.bgl" (4 letras + .bgl) — el ICAO
    // típicamente aparece como nombre del BGL del AFCAD.
    let upper = line.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 8 {
        return None;
    }
    for i in 0..=bytes.len() - 8 {
        if &bytes[i + 4..i + 8] != b".BGL" {
            continue;
        }
        let icao = &bytes[i..i + 4];
        if !icao.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if before_ok {
            return Some(String::from_utf8(icao.to_vec()).ok()?);
        }
    }
    None
}
