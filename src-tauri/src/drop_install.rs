//! Drag & Drop universal (v2.1.0).
//!
//! Reemplaza el flujo anterior de `install_archive`-only por uno que
//! soporta:
//!
//!   · **Perfiles GSX** (`.ini` / `.py` sueltos o dentro de `.zip`/`.rar`).
//!   · **Paquetes Community** (carpetas con `manifest.json` dentro de
//!     `.zip`/`.rar` o `.7z`).
//!   · **Mixtos** — un archivo puede tener perfiles GSX + escenarios.
//!
//! Flujo:
//!
//!   1. **Inspect** — el frontend llama `drop_inspect(path)`. Extraemos
//!      a un tempdir persistente (vive en `AppState::drop_sessions`) y
//!      clasificamos cada item. Devolvemos `DropInspection` con la
//!      lista de items + `session_id` para referenciar el tempdir.
//!   2. **Commit** — el frontend muestra modal de selección si hay
//!      >1 item. Después llama `drop_commit(session_id, selected_ids)`
//!      que copia cada item a su destino real (GSX folder / Community
//!      folder).
//!   3. **Cleanup** — al cerrar el modal el frontend llama
//!      `drop_cancel(session_id)` para liberar el tempdir.
//!      Auto-cleanup tras 10 min sin commit como red de seguridad.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tempfile::TempDir;

/// Sesión de drop pendiente. Mantenemos el `TempDir` vivo hasta el
/// commit/cancel — al dropear el TempDir, sus archivos se borran.
pub struct DropSession {
    pub _tempdir: Option<TempDir>,
    pub archive_path: String,
    pub items: Vec<DropItem>,
    pub created_at: std::time::Instant,
}

/// Holder en AppState. Mutex sync porque sólo se toca desde comandos
/// tauri sync (cortos), no se hace I/O dentro del lock.
#[derive(Default)]
pub struct DropSessions(pub Mutex<HashMap<String, DropSession>>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropItem {
    /// "gsx_profile" | "community_package" | "installer_exe" | "unknown"
    pub kind: String,
    /// Texto humano enriquecido: "EDDF · Aerosoft (VDGS · python handler)".
    pub label: String,
    /// ICAO detectado (sólo para gsx_profile y community_package).
    pub icao: Option<String>,
    /// (v2.1.1) Variantes detectadas: ["vdgs", "novdgs", "handler",
    /// "es", "v2", ...]. Sirve para que la UI muestre chips diferenciados
    /// cuando un .zip trae varias copias del mismo perfil.
    pub variants: Vec<String>,
    /// Path absoluto del archivo extraído (o el original si no se
    /// extrajo). Lo usa el commit.
    pub source_path: String,
    /// Path relativo dentro del archivo original — para mostrar al
    /// usuario, no para abrir.
    pub relative_path: String,
    /// Tamaño en bytes — para mostrar al usuario.
    pub size_bytes: u64,
    /// (v2.1.1) Primeras líneas relevantes del .ini (comentarios,
    /// secciones) — útil para distinguir variantes que sólo difieren
    /// en el contenido pero comparten nombre.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropInspection {
    pub session_id: String,
    /// Path original del archivo dropeado.
    pub archive_path: String,
    /// Lista de items detectados. Vacía → el archivo no es válido.
    pub items: Vec<DropItem>,
    /// True si el archivo era un .ini/.py suelto. El frontend puede
    /// skip el modal e instalar directo.
    pub is_single: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropCommitReport {
    pub installed_gsx: Vec<String>,
    pub installed_packages: Vec<String>,
    pub errors: Vec<String>,
}

/// Inspecciona el archivo dropeado y devuelve la lista de items
/// instalables dentro. Crea una sesión que mantiene el tempdir.
pub fn inspect(
    archive_path: &Path,
    sessions: &DropSessions,
) -> anyhow::Result<DropInspection> {
    tracing::info!(target: "drop", "inspect: archive={}", archive_path.display());
    if !archive_path.is_file() {
        anyhow::bail!("No existe el archivo {}", archive_path.display());
    }
    let ext = archive_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    tracing::info!(target: "drop", "extension detectada: .{}", ext);

    let session_id = uuid::Uuid::new_v4().to_string();
    let archive_str = archive_path.to_string_lossy().into_owned();

    // .ini / .py sueltos → un solo item, sin extracción.
    if ext == "ini" || ext == "py" {
        // Para archivos sueltos no hay extract_root real — usamos el
        // padre del archivo como referencia para calcular relative.
        let root = archive_path.parent().unwrap_or(Path::new(""));
        let item = classify_gsx_file(archive_path, root)?;
        let session = DropSession {
            _tempdir: None,
            archive_path: archive_str.clone(),
            items: vec![item.clone()],
            created_at: std::time::Instant::now(),
        };
        sessions
            .0
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex envenenado: {e}"))?
            .insert(session_id.clone(), session);
        tracing::info!(
            target: "drop",
            "sesión {} creada con 1 item (GSX suelto): {}",
            session_id, item.label
        );
        return Ok(DropInspection {
            session_id,
            archive_path: archive_str,
            items: vec![item],
            is_single: true,
        });
    }

    // .zip / .rar / .7z → extraer y enumerar.
    if !matches!(ext.as_str(), "zip" | "rar" | "7z") {
        anyhow::bail!(
            "Extensión no soportada: .{} (acepta .ini, .py, .zip, .rar, .7z)",
            ext
        );
    }

    let tempdir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("no se pudo crear tempdir: {e}"))?;
    let temp_path = tempdir.path().to_path_buf();
    tracing::info!(
        target: "drop",
        "extrayendo .{} a {}",
        ext, temp_path.display()
    );

    match ext.as_str() {
        "zip" => extract_zip(archive_path, &temp_path)?,
        "rar" => extract_rar(archive_path, &temp_path)?,
        "7z" => extract_7z(archive_path, &temp_path)?,
        _ => unreachable!(),
    }
    tracing::info!(target: "drop", "extracción completada");

    let items = scan_extracted(&temp_path)?;
    tracing::info!(
        target: "drop",
        "scan: {} items detectados ({} GSX + {} Community + {} otros)",
        items.len(),
        items.iter().filter(|i| i.kind == "gsx_profile").count(),
        items.iter().filter(|i| i.kind == "community_package").count(),
        items
            .iter()
            .filter(|i| !matches!(i.kind.as_str(), "gsx_profile" | "community_package"))
            .count(),
    );

    let session = DropSession {
        _tempdir: Some(tempdir),
        archive_path: archive_str.clone(),
        items: items.clone(),
        created_at: std::time::Instant::now(),
    };
    sessions
        .0
        .lock()
        .map_err(|e| anyhow::anyhow!("Mutex envenenado: {e}"))?
        .insert(session_id.clone(), session);

    Ok(DropInspection {
        session_id,
        archive_path: archive_str,
        items,
        is_single: false,
    })
}

/// Instala los items seleccionados. `selected_paths` debe contener
/// los `source_path` (absolutos) de los items a instalar.
///
/// (v2.2.0) Al terminar (con o sin errores), ELIMINA la sesión y su
/// tempdir asociado. Antes el tempdir se quedaba colgado hasta el
/// timeout de 10min o un drop_cancel explícito, lo cual no siempre
/// pasaba si la UI hacía algo inesperado.
pub fn commit(
    session_id: &str,
    selected_paths: &[String],
    community_path: &Path,
    sessions: &DropSessions,
) -> anyhow::Result<DropCommitReport> {
    tracing::info!(
        target: "drop",
        "commit: session={} ({} items seleccionados)",
        session_id, selected_paths.len()
    );

    let items = {
        let map = sessions
            .0
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex envenenado: {e}"))?;
        match map.get(session_id) {
            Some(s) => s.items.clone(),
            None => anyhow::bail!("Sesión {} no existe o expiró", session_id),
        }
    };

    let mut report = DropCommitReport {
        installed_gsx: Vec::new(),
        installed_packages: Vec::new(),
        errors: Vec::new(),
    };

    for path_str in selected_paths {
        let Some(item) = items.iter().find(|i| i.source_path == *path_str) else {
            report
                .errors
                .push(format!("Item no encontrado en sesión: {}", path_str));
            tracing::warn!(target: "drop", "item no en sesión: {}", path_str);
            continue;
        };
        match item.kind.as_str() {
            "gsx_profile" => match install_gsx(&PathBuf::from(&item.source_path)) {
                Ok(dest) => {
                    tracing::info!(
                        target: "drop",
                        "GSX instalado: {} → {}",
                        item.label, dest
                    );
                    report.installed_gsx.push(dest);
                }
                Err(e) => {
                    tracing::error!(target: "drop", "GSX falló ({}): {}", item.label, e);
                    report.errors.push(format!("{}: {}", item.label, e));
                }
            },
            "community_package" => {
                let src_dir = PathBuf::from(&item.source_path);
                match install_community_package(&src_dir, community_path) {
                    Ok(dest) => {
                        tracing::info!(
                            target: "drop",
                            "Community instalado: {} → {}",
                            item.label, dest
                        );
                        report.installed_packages.push(dest);
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "drop",
                            "Community falló ({}): {}",
                            item.label, e
                        );
                        report.errors.push(format!("{}: {}", item.label, e));
                    }
                }
            }
            other => {
                tracing::warn!(target: "drop", "tipo no instalable: {}", other);
                report
                    .errors
                    .push(format!("Tipo no instalable: {}", item.kind));
            }
        }
    }

    // (v2.2.0) Cleanup explícito del tempdir + sesión tras commit.
    // El TempDir se elimina del filesystem cuando hacemos drop() de él
    // (lo trae `tempfile::TempDir`). Quitarlo del HashMap libera la
    // memoria + dispara el cleanup en cascada.
    if let Ok(mut map) = sessions.0.lock() {
        if let Some(removed) = map.remove(session_id) {
            tracing::info!(
                target: "drop",
                "cleanup: tempdir + sesión {} eliminados ({} archivos extraídos liberados)",
                session_id, removed.items.len()
            );
        }
    }

    Ok(report)
}

pub fn cancel(session_id: &str, sessions: &DropSessions) {
    tracing::info!(target: "drop", "cancel: session={}", session_id);
    if let Ok(mut map) = sessions.0.lock() {
        map.remove(session_id);
    }
}

/// Limpia sesiones que llevan > 10 min sin commit/cancel. Llamado
/// periódicamente desde un timer en lib.rs (best-effort, no crítico).
#[allow(dead_code)]
pub fn cleanup_stale(sessions: &DropSessions) {
    if let Ok(mut map) = sessions.0.lock() {
        let now = std::time::Instant::now();
        map.retain(|_, s| now.duration_since(s.created_at).as_secs() < 600);
    }
}

// =============================================================================
// Extracción
// =============================================================================

fn extract_zip(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(src)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(dest)?;
    Ok(())
}

fn extract_rar(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let archive = unrar::Archive::new(src)
        .open_for_processing()
        .map_err(|e| anyhow::anyhow!("rar open: {e}"))?;
    let mut iter = archive;
    loop {
        let header = match iter.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => anyhow::bail!("rar read header: {e}"),
        };
        iter = header
            .extract_with_base(dest)
            .map_err(|e| anyhow::anyhow!("rar extract: {e}"))?;
    }
    Ok(())
}

fn extract_7z(src: &Path, dest: &Path) -> anyhow::Result<()> {
    sevenz_rust2::decompress_file(src, dest)
        .map_err(|e| anyhow::anyhow!("7z extract: {e}"))?;
    Ok(())
}

// =============================================================================
// Scan + classify
// =============================================================================

fn scan_extracted(root: &Path) -> anyhow::Result<Vec<DropItem>> {
    let mut items = Vec::new();
    // Paso 1 — encontrar TODOS los manifest.json de Community.
    let mut community_dirs: Vec<PathBuf> = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name() != "manifest.json" {
            continue;
        }
        if let Some(parent) = entry.path().parent() {
            community_dirs.push(parent.to_path_buf());
        }
    }
    tracing::debug!(
        target: "drop",
        "scan: {} manifest.json encontrados",
        community_dirs.len()
    );

    for dir in &community_dirs {
        match classify_community_dir(dir, root) {
            Ok(item) => items.push(item),
            Err(e) => {
                tracing::warn!(
                    target: "drop",
                    "manifest inválido en {}: {}",
                    dir.display(), e
                );
            }
        }
    }

    // Paso 2 — .ini/.py de GSX. Excluimos los que estén DENTRO de un
    // community_dir.
    for entry in walkdir::WalkDir::new(root) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "ini" && ext != "py" {
            continue;
        }
        if community_dirs.iter().any(|d| entry.path().starts_with(d)) {
            continue;
        }
        if let Ok(item) = classify_gsx_file(entry.path(), root) {
            items.push(item);
        }
    }

    // (v2.1.1) Ordenar items por relative_path para que las variantes
    // del mismo perfil queden agrupadas visualmente.
    items.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(items)
}

/// (v2.1.1) Construye un label detallado para un archivo GSX.
/// Detecta variantes (VDGS / noVDGS / handler / locale) a partir de:
///   1. Filename suffixes (`-vdgs`, `_novdgs`, `_handler`, `-es`, etc.)
///   2. Subcarpeta (`novdgs/EDDF.ini` → variant "noVDGS folder")
///   3. Comentarios al inicio del .ini (`; Variant: VDGS`, etc.)
fn classify_gsx_file(file_path: &Path, extract_root: &Path) -> anyhow::Result<DropItem> {
    let file_name = file_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("archivo")
        .to_string();
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&file_name)
        .to_string();
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let icao = leading_icao_from_filename(&stem);
    let size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);

    // (v2.1.1) FIX del bug del relative_path — antes hacía strip_prefix
    // sobre `archive_path.parent()` que está en otro directorio (Downloads
    // vs Temp), y siempre fallaba → fallback a file_name solo. Ahora
    // strip_prefix sobre el extract_root (donde realmente vive el archivo)
    // y devolvemos el path COMPLETO dentro del archive (e.g.
    // "novdgs/EDDF-Aerosoft.ini").
    let rel_path: PathBuf = file_path
        .strip_prefix(extract_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(&file_name));
    let rel = rel_path.to_string_lossy().replace('\\', "/");

    // ─── Variant detection ──────────────────────────────────────────
    let mut variants: Vec<String> = Vec::new();
    let stem_lower = stem.to_ascii_lowercase();

    // Por extensión.
    if ext == "py" {
        if stem_lower.contains("handler") {
            variants.push("python handler".to_string());
        } else {
            variants.push("python script".to_string());
        }
    }

    // Por filename suffix.
    if stem_lower.contains("novdgs") || stem_lower.contains("no_vdgs") || stem_lower.contains("no-vdgs") {
        variants.push("sin VDGS".to_string());
    } else if stem_lower.contains("vdgs") {
        variants.push("con VDGS".to_string());
    }
    if stem_lower.contains("safedock") {
        variants.push("Safedock".to_string());
    }
    // Locale codes: -es, -en, -de, -fr, -it, -pt
    let locale_re = ["-es", "_es", "-en", "_en", "-de", "_de", "-fr", "_fr"];
    for suffix in locale_re {
        if stem_lower.ends_with(suffix) || stem_lower.contains(&format!("{}-", suffix)) {
            let code = suffix.trim_start_matches(['-', '_']);
            variants.push(format!("idioma: {}", code.to_ascii_uppercase()));
            break;
        }
    }
    // Versión inline: -v1, _v2, etc.
    if let Some(cap) = regex::Regex::new(r"[-_]v(\d+(?:\.\d+)*)")
        .ok()
        .and_then(|r| r.captures(&stem_lower))
    {
        if let Some(v) = cap.get(1) {
            variants.push(format!("v{}", v.as_str()));
        }
    }

    // Por subfolder en el archive.
    if let Some(parent_rel) = rel_path.parent() {
        let parent_str = parent_rel.to_string_lossy().to_ascii_lowercase();
        if !parent_str.is_empty() && parent_str != "/" && parent_str != "." {
            // Si el subfolder contiene VDGS/noVDGS/etc, ya lo añadimos
            // arriba via filename — evitamos duplicar.
            let normalized_parent = parent_str.replace(['/', '_'], "-");
            if !variants.iter().any(|v| normalized_parent.contains(&v.to_ascii_lowercase())) {
                // Mostramos el subfolder como variant si tiene info útil.
                if normalized_parent.contains("vdgs")
                    || normalized_parent.contains("novdgs")
                    || normalized_parent.contains("safedock")
                    || normalized_parent.contains("aerosoft")
                    || normalized_parent.contains("mkstudios")
                    || normalized_parent.contains("flytampa")
                    || normalized_parent.len() <= 30
                {
                    variants.push(format!("carpeta: {}", parent_str));
                }
            }
        }
    }

    // ─── Inspect file content (sólo .ini, primeras ~2KB) ────────────
    let description = if ext == "ini" {
        read_ini_header(file_path)
    } else {
        None
    };

    // Si el contenido aporta "Variant: X" lo metemos como variant.
    if let Some(ref desc) = description {
        for line in desc.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix(";").and_then(|s| s.trim_start().strip_prefix("variant")).or_else(|| {
                lower.strip_prefix("#").and_then(|s| s.trim_start().strip_prefix("variant"))
            }) {
                let cleaned: String = rest
                    .trim_start_matches([':', '=', ' '])
                    .chars()
                    .take(40)
                    .collect();
                if !cleaned.is_empty() {
                    variants.push(format!("variant: {}", cleaned));
                }
            }
        }
    }

    // Deduplicate.
    variants.dedup();

    // ─── Build label ─────────────────────────────────────────────────
    let base_label = match &icao {
        Some(i) => format!("{} · {}", i, file_name),
        None => file_name.clone(),
    };
    let label = if variants.is_empty() {
        base_label
    } else {
        format!("{}  —  {}", base_label, variants.join(" · "))
    };

    Ok(DropItem {
        kind: "gsx_profile".to_string(),
        label,
        icao,
        variants,
        source_path: file_path.to_string_lossy().into_owned(),
        relative_path: rel,
        size_bytes: size,
        description,
    })
}

/// Lee los primeros bytes del .ini y devuelve hasta 8 líneas no vacías
/// como descripción para mostrar al usuario.
fn read_ini_header(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines().take(40) {
        let line = line.ok()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(trimmed.to_string());
        if lines.len() >= 8 {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn classify_community_dir(dir: &Path, root: &Path) -> anyhow::Result<DropItem> {
    let folder_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("paquete");
    let manifest_path = dir.join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)?;
    let title = manifest
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(folder_name)
        .to_string();
    let content_type = manifest
        .get("content_type")
        .and_then(|v| v.as_str())
        .map(String::from);
    let icao = extract_icao(folder_name).or_else(|| extract_icao(&title));
    let size = directory_size(dir).unwrap_or(0);
    let rel = dir
        .strip_prefix(root)
        .ok()
        .and_then(|p| p.to_str())
        .map(String::from)
        .unwrap_or_else(|| folder_name.to_string());
    let label = format!(
        "{} ({})",
        title,
        content_type.as_deref().unwrap_or("paquete")
    );
    Ok(DropItem {
        kind: "community_package".to_string(),
        label,
        icao,
        variants: Vec::new(),
        source_path: dir.to_string_lossy().into_owned(),
        relative_path: rel,
        size_bytes: size,
        description: None,
    })
}

// =============================================================================
// Install actions
// =============================================================================

fn install_gsx(src: &Path) -> anyhow::Result<String> {
    let folder = gsx_profiles_folder().ok_or_else(|| {
        anyhow::anyhow!("No se pudo resolver %APPDATA%\\Virtuali\\GSX\\MSFS")
    })?;
    std::fs::create_dir_all(&folder)?;
    let file_name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("archivo sin nombre"))?;
    let dest = folder.join(file_name);
    tracing::info!(
        target: "drop",
        "copiando GSX {} → {}",
        src.display(), dest.display()
    );
    std::fs::copy(src, &dest)?;
    Ok(dest.to_string_lossy().into_owned())
}

fn install_community_package(src_dir: &Path, community_path: &Path) -> anyhow::Result<String> {
    let folder_name = src_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("carpeta sin nombre"))?;
    let dest = community_path.join(folder_name);
    tracing::info!(
        target: "drop",
        "copiando Community {} → {}",
        src_dir.display(), dest.display()
    );
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    copy_dir_recursive(src_dir, &dest)?;
    Ok(dest.to_string_lossy().into_owned())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

fn directory_size(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
        if entry.file_type().is_file() {
            if let Ok(m) = entry.metadata() {
                total = total.saturating_add(m.len());
            }
        }
    }
    Ok(total)
}

// =============================================================================
// Helpers
// =============================================================================

pub fn gsx_profiles_folder() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Virtuali").join("GSX").join("MSFS"))
}

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

fn extract_icao(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let cand = &bytes[i..i + 4];
        if !cand.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if before_ok && after_ok {
            if let Ok(s) = String::from_utf8(cand.to_vec()) {
                candidates.push((i, s));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    // Prefer matches preceded by "AIRPORT-".
    for (i, c) in &candidates {
        if *i >= 8 {
            let prefix = &bytes[i - 8..*i];
            if prefix.starts_with(b"AIRPORT") && matches!(prefix[7], b'-' | b'_' | b' ') {
                return Some(c.clone());
            }
        }
    }
    candidates.into_iter().next().map(|(_, c)| c)
}
