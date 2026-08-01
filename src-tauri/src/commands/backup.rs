//! Backup y export de la carpeta Community.
//!
//! · `backup_community` — comprime la carpeta Community en un .zip
//!   con timestamp en el nombre, en una ruta que el usuario elige.
//!   Útil antes de actualizar MSFS o instalar packs experimentales.
//!
//! · `export_addons_list` — exporta el inventario de
//!   `community_packages` a CSV / TXT / JSON. Para que el usuario
//!   pueda hacer share de "qué tengo instalado" o auditar lo que
//!   ocupa disco.
//!
//! Ambos comandos hacen el trabajo pesado en `spawn_blocking` para
//! no bloquear el reactor de Tauri durante el zip o la I/O.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub output_path: String,
    pub package_count: usize,
    pub total_bytes: u64,
    pub elapsed_ms: u128,
}

/// Comprime la carpeta Community en `dest_path`. Si `dest_path` es
/// un directorio, generamos un nombre `community-backup-YYYYMMDD-HHMMSS.zip`
/// dentro. Si es un archivo concreto (.zip), respetamos el nombre.
#[tauri::command]
pub async fn backup_community(
    dest_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<BackupResult, String> {
    let community = match crate::community::detect_community_folder()
        .map_err(|e| e.to_string())?
    {
        Some(info) if info.exists => PathBuf::from(info.path),
        _ => return Err("No se detectó la carpeta Community.".into()),
    };

    let dest = PathBuf::from(&dest_path);
    let final_path = if dest.is_dir() || !dest_path.to_lowercase().ends_with(".zip") {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let dir = if dest.is_dir() { dest.clone() } else { dest };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dir.join(format!("community-backup-{ts}.zip"))
    } else {
        dest
    };

    // El zip puede tardar minutos en una Community grande — fuera
    // del reactor.
    let started = std::time::Instant::now();
    let output_clone = final_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        zip_directory(&community, &output_clone)
    })
    .await
    .map_err(|e| format!("backup task join: {e}"))?
    .map_err(|e| e.to_string())?;
    let elapsed_ms = started.elapsed().as_millis();

    // Conteo de paquetes — por curiosidad para mostrar al usuario.
    let pkg_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM community_packages")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    Ok(BackupResult {
        output_path: final_path.to_string_lossy().into_owned(),
        package_count: pkg_count as usize,
        total_bytes: result,
        elapsed_ms,
    })
}

/// (v6.2.30 / R7) Escribe bytes (base64) a `path`. Lo usa el export de
/// las tarjetas compartibles y el Wrapped: el frontend rasteriza el SVG
/// a PNG en un canvas y nos manda el data-URL en base64. Genérico y
/// reutilizable para cualquier "guardar imagen/binario a disco".
#[tauri::command]
pub async fn save_binary_file(path: String, base64: String) -> Result<(), String> {
    use base64::Engine;
    // Aceptamos tanto un data-URL completo (`data:image/png;base64,AAAA`)
    // como el base64 pelado.
    let payload = base64
        .split_once("base64,")
        .map(|(_, b)| b)
        .unwrap_or(&base64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| format!("base64 inválido: {e}"))?;
    let p = PathBuf::from(&path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// Comprime recursivamente `src` en `dest`. Devuelve los bytes
/// totales escritos. Usa `zip` con compresión Deflate.
fn zip_directory(src: &Path, dest: &Path) -> anyhow::Result<u64> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    let file = std::fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut total: u64 = 0;
    // (fix) Backup ROBUSTO: un archivo bloqueado por MSFS/antivirus (sharing
    // violation) o sin permiso NO debe abortar el backup entero ni dejar un
    // .zip corrupto. Se saltan los que fallan (logueados) y se sigue. Justo el
    // momento del backup suele ser con MSFS abierto bloqueando scenery.
    let mut skipped: u64 = 0;
    // (v7) `follow_links(true)`: muchos usuarios enlazan addons a Community con
    // junctions/symlinks (el propio cross-link de la app). Sin esto walkdir los
    // ve como un "link" y NO desciende → el backup omitiría esos addons en
    // silencio. Siguiendo links, se respaldan sus archivos reales.
    for entry in walkdir::WalkDir::new(src).follow_links(true) {
        // Una entrada ilegible (permiso en una subcarpeta) → saltar, no abortar.
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(target: "backup", "entrada ilegible saltada: {e}");
                skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        let rel = match path.strip_prefix(src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if entry.file_type().is_file() {
            // Leemos ANTES de empezar la entrada del zip: si falla, saltamos
            // limpio sin dejar una entrada vacía/corrupta.
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        target: "backup",
                        "archivo bloqueado/ilegible saltado: {} ({e})",
                        path.display()
                    );
                    skipped += 1;
                    continue;
                }
            };
            let name = rel.to_string_lossy().replace('\\', "/");
            zip.start_file(name, opts)?;
            zip.write_all(&bytes)?;
            total += bytes.len() as u64;
        } else if entry.file_type().is_dir() {
            let name = format!("{}/", rel.to_string_lossy().replace('\\', "/"));
            zip.add_directory(name, opts)?;
        }
    }
    zip.finish()?;
    if skipped > 0 {
        tracing::warn!(
            target: "backup",
            "backup terminado con {skipped} archivo(s) omitido(s) por bloqueo/permiso"
        );
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Txt,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub output_path: String,
    pub row_count: usize,
}

/// Exporta el inventario de Community al fichero indicado, en el
/// formato indicado.
///
///   · CSV — abre limpio en Excel/Numbers/Sheets.
///   · TXT — formato tabular legible, columnas alineadas.
///   · JSON — para scripts / integraciones.
#[tauri::command]
pub async fn export_addons(
    dest_path: String,
    format: ExportFormat,
    state: tauri::State<'_, AppState>,
) -> Result<ExportResult, String> {
    let rows = fetch_export_rows(&state.db).await.map_err(|e| e.to_string())?;
    let path = PathBuf::from(&dest_path);
    let serialized = match format {
        ExportFormat::Csv => serialize_csv(&rows),
        ExportFormat::Txt => serialize_txt(&rows),
        ExportFormat::Json => serialize_json(&rows).map_err(|e| e.to_string())?,
    };
    std::fs::write(&path, serialized).map_err(|e| e.to_string())?;
    Ok(ExportResult {
        output_path: path.to_string_lossy().into_owned(),
        row_count: rows.len(),
    })
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExportRow {
    folder_name: String,
    title: String,
    creator: Option<String>,
    content_type: Option<String>,
    package_version: Option<String>,
    icao: Option<String>,
    size_bytes: Option<i64>,
    folder_modified_at: Option<String>,
}

async fn fetch_export_rows(pool: &SqlitePool) -> anyhow::Result<Vec<ExportRow>> {
    let rows = sqlx::query_as::<_, ExportRow>(
        r#"
        SELECT folder_name, title, creator, content_type, package_version,
               icao, size_bytes, folder_modified_at
        FROM community_packages
        ORDER BY title COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn serialize_csv(rows: &[ExportRow]) -> String {
    let mut out = String::new();
    out.push_str("folder_name,title,creator,content_type,version,icao,size_bytes,modified_at\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_escape(&r.folder_name),
            csv_escape(&r.title),
            csv_escape(r.creator.as_deref().unwrap_or("")),
            csv_escape(r.content_type.as_deref().unwrap_or("")),
            csv_escape(r.package_version.as_deref().unwrap_or("")),
            csv_escape(r.icao.as_deref().unwrap_or("")),
            r.size_bytes.unwrap_or(0),
            csv_escape(r.folder_modified_at.as_deref().unwrap_or("")),
        ));
    }
    out
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn serialize_txt(rows: &[ExportRow]) -> String {
    let mut out = String::new();
    out.push_str("SimFleet — inventario de Community\n");
    out.push_str(&format!(
        "Generado: {}\nTotal: {} paquetes\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        rows.len()
    ));
    out.push_str(
        "------------------------------------------------------------------------------\n",
    );
    for r in rows {
        out.push_str(&format!("· {}\n", r.title));
        out.push_str(&format!(
            "    Folder:   {}\n    Creator:  {}\n    Tipo:     {}\n    Versión:  {}\n    ICAO:     {}\n    Tamaño:   {}\n    Modific.: {}\n\n",
            r.folder_name,
            r.creator.as_deref().unwrap_or("(desconocido)"),
            r.content_type.as_deref().unwrap_or("—"),
            r.package_version.as_deref().unwrap_or("—"),
            r.icao.as_deref().unwrap_or("—"),
            human_bytes(r.size_bytes.unwrap_or(0)),
            r.folder_modified_at.as_deref().unwrap_or("—"),
        ));
    }
    out
}

fn serialize_json(rows: &[ExportRow]) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct ExportJson<'a> {
        folder_name: &'a str,
        title: &'a str,
        creator: Option<&'a str>,
        content_type: Option<&'a str>,
        package_version: Option<&'a str>,
        icao: Option<&'a str>,
        size_bytes: i64,
        folder_modified_at: Option<&'a str>,
    }
    let mapped: Vec<_> = rows
        .iter()
        .map(|r| ExportJson {
            folder_name: &r.folder_name,
            title: &r.title,
            creator: r.creator.as_deref(),
            content_type: r.content_type.as_deref(),
            package_version: r.package_version.as_deref(),
            icao: r.icao.as_deref(),
            size_bytes: r.size_bytes.unwrap_or(0),
            folder_modified_at: r.folder_modified_at.as_deref(),
        })
        .collect();
    Ok(serde_json::to_string_pretty(&mapped)?)
}

fn human_bytes(n: i64) -> String {
    let mut v = n as f64;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, units[i])
}
