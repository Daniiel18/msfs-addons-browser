//! Escáner de la carpeta Community de MSFS.
//!
//! Recorre cada subcarpeta inmediata, lee su `manifest.json`, y
//! produce un `ScannedPackage` por cada paquete válido. La salida
//! se persiste en `community_packages` (migración 005) — esa tabla
//! es la fuente de verdad para la vista de mapa y el detector de
//! actualizaciones, en vez de `installed_addons` (que sólo registra
//! lo que se descargó a través de esta app).
//!
//! El escaneo es **sincrónico y bloqueante** porque:
//!   · La carpeta Community vive en SSD local — leer 200 manifests
//!     son ~200ms.
//!   · Lo invocamos desde un `spawn_blocking` o desde una tarea
//!     async normal — `std::fs` no requiere ceremonia adicional.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedPackage {
    pub folder_name: String,
    pub install_path: String,
    pub title: String,
    pub creator: Option<String>,
    pub content_type: Option<String>,
    pub package_version: Option<String>,
    pub minimum_game_version: Option<String>,
    pub icao: Option<String>,
    pub size_bytes: Option<u64>,
    pub folder_modified_at: Option<String>,
    /// Cantidad de entries en `manifest.dependencies`. Usamos esto
    /// para detectar liveries: en MSFS las liveries se declaran como
    /// `content_type=AIRCRAFT` con dependencia al aircraft base, así
    /// que `AIRCRAFT + deps>0` distingue livery de aircraft completo.
    pub dependencies_count: usize,
}

/// Estructura literal del `manifest.json` de MSFS. Sólo
/// deserializamos los campos que conocemos; el resto se ignora
/// vía `#[serde(default)]` y comportamiento por defecto de serde.
///
/// El manifest puede traer `manufacturer` y/o `creator` — mostramos
/// `creator` cuando exista (es lo que ven los usuarios en el sim);
/// si no, caemos a `manufacturer`.
#[derive(Debug, Deserialize)]
struct RawManifest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    package_version: Option<String>,
    #[serde(default)]
    minimum_game_version: Option<String>,
    /// Array genérico — no nos importa la forma exacta, sólo cuántos
    /// hay. Usar `serde_json::Value` evita modelar la estructura
    /// (que varía entre paquetes).
    #[serde(default)]
    dependencies: Vec<serde_json::Value>,
}

/// Resultado del escaneo: paquetes encontrados + recuento de
/// errores no fatales (manifests rotos, carpetas sin manifest…).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub packages: Vec<ScannedPackage>,
    pub skipped_no_manifest: usize,
    pub skipped_invalid_manifest: usize,
    pub community_path: String,
}

/// Escanea Community recursivamente al primer nivel. No descendemos
/// porque los paquetes de MSFS son siempre `Community/<paquete>/...`
/// — `<paquete>/<sub>/manifest.json` no es válido.
pub fn scan(community_path: &Path) -> anyhow::Result<ScanReport> {
    let mut packages = Vec::new();
    let mut skipped_no_manifest = 0usize;
    let mut skipped_invalid_manifest = 0usize;

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

        let manifest_path = path.join("manifest.json");
        if !manifest_path.is_file() {
            skipped_no_manifest += 1;
            continue;
        }

        match parse_manifest(&manifest_path) {
            Ok(raw) => {
                packages.push(materialize(&path, &folder_name, raw));
            }
            Err(e) => {
                tracing::debug!(
                    "scanner: manifest inválido en {} ({:#})",
                    manifest_path.display(),
                    e
                );
                // Fallback: aún sin manifest válido, registramos el
                // paquete con info derivada del folder name. Eso
                // permite que el usuario lo vea en el mapa con un
                // título plausible y nuestra heurística de ICAO.
                skipped_invalid_manifest += 1;
                packages.push(materialize_fallback(&path, &folder_name));
            }
        }
    }

    Ok(ScanReport {
        packages,
        skipped_no_manifest,
        skipped_invalid_manifest,
        community_path: community_path.to_string_lossy().into_owned(),
    })
}

fn parse_manifest(path: &Path) -> anyhow::Result<RawManifest> {
    let raw = std::fs::read_to_string(path)?;
    // Algunos manifests de la comunidad traen comentarios o trailing
    // commas — `serde_json::from_str` los rechaza. Sobre el universo
    // típico funciona, y los rotos caen al fallback.
    let parsed: RawManifest = serde_json::from_str(&raw)?;
    Ok(parsed)
}

fn materialize(path: &Path, folder_name: &str, raw: RawManifest) -> ScannedPackage {
    let title = raw
        .title
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| pretty_folder_name(folder_name));

    let creator = raw
        .creator
        .or(raw.manufacturer)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let content_type = raw
        .content_type
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // ICAO sólo tiene sentido para SCENERY. Aplicar la heurística
    // a AIRCRAFT/INSTRUMENT/MISC produce falsos positivos como
    // "PMDG" → matching contra el aeropuerto Palmar Sur — y
    // dispara notificaciones de update que el usuario reporta como
    // "no tienen sentido". Para todo lo que no sea SCENERY el
    // ICAO se queda en None de forma estricta.
    //
    // (v3.1.3) ORDEN INVERTIDO: probamos folder name PRIMERO, title
    // como fallback. Los folder names siguen convenciones rígidas
    // (`{vendor}-airport[s]-{ICAO}-{nombre}`), los titles son
    // free-form y dan falsos positivos:
    //   · "Paris Orly LFPO" → `ORLY` se pillaba antes que `LFPO`
    //   · "Saint Martin & Grand Case" → `CASE` se pillaba (y el
    //     título no contiene TNCM en absoluto). El folder
    //     `awdesigns-airports-tncm-tffg-saint-martin` SÍ lo tiene.
    let icao = if matches_scenery(&content_type) {
        extract_icao(folder_name).or_else(|| extract_icao(&title))
    } else {
        None
    };

    let (size_bytes, folder_modified_at) = folder_metadata(path);

    ScannedPackage {
        folder_name: folder_name.to_string(),
        install_path: path.to_string_lossy().into_owned(),
        title,
        creator,
        content_type,
        package_version: raw.package_version.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        minimum_game_version: raw
            .minimum_game_version
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        icao,
        size_bytes,
        folder_modified_at,
        dependencies_count: raw.dependencies.len(),
    }
}

fn materialize_fallback(path: &Path, folder_name: &str) -> ScannedPackage {
    let (size_bytes, folder_modified_at) = folder_metadata(path);
    let title = pretty_folder_name(folder_name);
    // Sin manifest válido no podemos saber si es SCENERY. Conservador:
    // no extraemos ICAO para evitar falsos positivos cuando alguien
    // tiene un livery o sound pack con folder name que parezca ICAO.
    ScannedPackage {
        folder_name: folder_name.to_string(),
        install_path: path.to_string_lossy().into_owned(),
        title,
        creator: None,
        content_type: None,
        package_version: None,
        minimum_game_version: None,
        icao: None,
        size_bytes,
        folder_modified_at,
        dependencies_count: 0,
    }
}

#[inline]
fn matches_scenery(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .map(|s| s.trim().eq_ignore_ascii_case("SCENERY"))
        .unwrap_or(false)
}

fn folder_metadata(path: &Path) -> (Option<u64>, Option<String>) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return (None, None),
    };
    let size = directory_size(path).ok();
    // (v3.1.0) "Recientemente añadido" — antes usábamos sólo
    // `modified()` que en Windows refleja la última escritura DENTRO
    // de la carpeta. Si el usuario instala manualmente un addon viejo
    // (descomprimiendo un .zip que preservaba mtimes originales) el
    // valor reflejaba la fecha del .zip, no la del install — y por
    // eso el dashboard sólo veía los drag-drop nuestros (que SÍ
    // generan mtime fresco al copiar).
    //
    // Fix: tomamos el MÁXIMO entre `created()` (birth time, en Windows
    // siempre presente y refleja la creación del folder en este disco
    // = momento del install) y `modified()` (último update). El que
    // sea más reciente gana, lo que cubre ambos casos: install nuevo
    // (created reciente) y edit reciente (modified más nuevo que
    // created).
    let to_secs = |t: std::time::SystemTime| -> Option<i64> {
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() as i64)
    };
    let created_secs = meta.created().ok().and_then(to_secs);
    let modified_secs = meta.modified().ok().and_then(to_secs);
    let chosen = match (created_secs, modified_secs) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let modified = chosen
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
    (size, modified)
}

/// Suma el tamaño de los archivos del folder. Se consulta on-demand
/// (no cacheada) — a 200 paquetes y miles de ficheros, sigue siendo
/// inferior a 1s en SSD. El cache caería en `community_packages`
/// y se invalidaría en cada scan.
fn directory_size(path: &Path) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        for entry in std::fs::read_dir(&p)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
    }
    Ok(total)
}

/// Convierte `bravoairspace-airport-mdsd-las-americas` →
/// `Bravoairspace Airport Mdsd Las Americas`. Heurística para
/// fallback cuando no hay título en el manifest.
fn pretty_folder_name(folder: &str) -> String {
    folder
        .split(|c: char| c == '-' || c == '_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extrae un ICAO (4 letras ASCII consecutivas) con frontera de
/// palabra. La validación contra airports reales se hace en el caller.
///
/// **(v1.1.4) Heurística "airport- wins"**: si hay múltiples 4-letras
/// candidates en el texto y al menos uno está inmediatamente precedido
/// por "airport-" / "airport_" / "airport " (típico de los packs de
/// scenery: `dev-airport-ICAO-name`), preferimos ése sobre los demás.
///
/// Bug que esto arregla: para `kaze-airport-mhtg-toncontin`, la
/// versión anterior elegía `KAZE` (developer name, casualmente ICAO
/// real de Hazlehurst GA) en vez de `MHTG` (Toncontin Honduras). El
/// resultado: Toncontin aparecía en EE.UU. en el mapa.
fn extract_icao(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }

    // Recolecta TODOS los candidatos con word boundary.
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let candidate = &bytes[i..i + 4];
        if !candidate.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if before_ok && after_ok {
            if let Ok(s) = String::from_utf8(candidate.to_vec()) {
                candidates.push((i, s));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates.into_iter().next().unwrap().1);
    }

    // Prioridad 1: candidato precedido inmediatamente por "AIRPORT"
    // o "AIRPORTS" + un separador (`-`, `_`, ` `). Los packs de
    // scenery siguen la convención `developer-airport-ICAO-nombre`,
    // y algunos vendors usan plural `developer-airports-ICAO-…`
    // (ej. `awdesigns-airports-tncm-tffg-saint-martin`).
    for (i, cand) in &candidates {
        // Match "AIRPORT-" / "AIRPORT_" / "AIRPORT " — 8 chars antes.
        if *i >= 8 {
            let prefix = &bytes[*i - 8..*i];
            if prefix.starts_with(b"AIRPORT")
                && matches!(prefix[7], b'-' | b'_' | b' ')
            {
                return Some(cand.clone());
            }
        }
        // (v3.1.3) Match "AIRPORTS-" plural — 9 chars antes.
        if *i >= 9 {
            let prefix = &bytes[*i - 9..*i];
            if prefix.starts_with(b"AIRPORTS")
                && matches!(prefix[8], b'-' | b'_' | b' ')
            {
                return Some(cand.clone());
            }
        }
    }

    // Prioridad 2: el primer match (comportamiento original). Usado
    // cuando ningún candidato tiene "airport-" delante — la mayoría
    // de folders fuera de la convención usan dev como prefijo opcional.
    Some(candidates.into_iter().next().unwrap().1)
}

/// Sincroniza los resultados del scan con la base de datos. Borra
/// entradas que ya no existen físicamente (paquete desinstalado
/// fuera de la app) y hace upsert del resto.
pub async fn sync_to_db(pool: &SqlitePool, report: &ScanReport) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;

    // Borra los que ya no están — por exclusión del set actual.
    if report.packages.is_empty() {
        sqlx::query("DELETE FROM community_packages")
            .execute(&mut *tx)
            .await?;
    } else {
        // SQLite no soporta `DELETE … WHERE x NOT IN (?,?,…)` con
        // `bind` plural genérico; armamos placeholders manualmente.
        let mut sql = String::from("DELETE FROM community_packages WHERE folder_name NOT IN (");
        let mut first = true;
        for _ in &report.packages {
            if !first {
                sql.push(',');
            }
            sql.push('?');
            first = false;
        }
        sql.push(')');
        let mut q = sqlx::query(&sql);
        for p in &report.packages {
            q = q.bind(&p.folder_name);
        }
        q.execute(&mut *tx).await?;
    }

    // Upsert por folder_name. Reemplazamos todos los campos en cada
    // scan — más simple que diff incremental y suficientemente rápido.
    for pkg in &report.packages {
        sqlx::query(
            r#"
            INSERT INTO community_packages (
                folder_name, install_path, title, creator, content_type,
                package_version, minimum_game_version, icao, size_bytes,
                folder_modified_at, dependencies_count, scanned_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'))
            ON CONFLICT(folder_name) DO UPDATE SET
                install_path = excluded.install_path,
                title = excluded.title,
                creator = excluded.creator,
                content_type = excluded.content_type,
                package_version = excluded.package_version,
                minimum_game_version = excluded.minimum_game_version,
                icao = excluded.icao,
                size_bytes = excluded.size_bytes,
                folder_modified_at = excluded.folder_modified_at,
                dependencies_count = excluded.dependencies_count,
                scanned_at = datetime('now')
            "#,
        )
        .bind(&pkg.folder_name)
        .bind(&pkg.install_path)
        .bind(&pkg.title)
        .bind(&pkg.creator)
        .bind(&pkg.content_type)
        .bind(&pkg.package_version)
        .bind(&pkg.minimum_game_version)
        .bind(&pkg.icao)
        .bind(pkg.size_bytes.map(|n| n as i64))
        .bind(&pkg.folder_modified_at)
        .bind(pkg.dependencies_count as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // (v4.24.0) RESCATE de ICAOs embebidos SIN separador. `extract_icao`
    // exige word boundaries, así que folders tipo `montevideosumu` (el
    // ICAO pegado al final) quedaban con icao NULL y el aeropuerto no
    // aparecía en el mapa (bug reportado con SUMU). Acá probamos TODAS
    // las ventanas de 4 letras del folder/título VALIDÁNDOLAS contra la
    // tabla `airports` (OurAirports) — solo se acepta un match real, con
    // preferencia por sufijo/prefijo para evitar falsos positivos.
    let unresolved: Vec<(String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT folder_name, title FROM community_packages
        WHERE (icao IS NULL OR icao = '')
          AND UPPER(COALESCE(content_type, '')) = 'SCENERY'
        "#,
    )
    .fetch_all(pool)
    .await?;
    for (folder, title) in unresolved {
        let mut rescued: Option<String> = None;
        for text in [Some(folder.as_str()), title.as_deref()].into_iter().flatten() {
            if let Some(icao) = rescue_embedded_icao(pool, text).await {
                rescued = Some(icao);
                break;
            }
        }
        if let Some(icao) = rescued {
            sqlx::query("UPDATE community_packages SET icao = ?1 WHERE folder_name = ?2")
                .bind(&icao)
                .bind(&folder)
                .execute(pool)
                .await?;
            tracing::info!(
                target: "scan",
                "icao rescatado para '{}': {} (substring validado contra airports)",
                folder, icao
            );
        }
    }

    Ok(report.packages.len())
}

/// (v4.24.0) Busca un ICAO embebido en `text` sin word boundaries: prueba
/// cada ventana de 4 letras contra la tabla `airports`. Selección:
///   1. ventana al FINAL del string (`montevideosumu` → SUMU),
///   2. ventana al INICIO,
///   3. único match distinto en el interior.
/// Varios matches interiores distintos → ambiguo → None.
async fn rescue_embedded_icao(pool: &SqlitePool, text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let mut windows: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let w = &bytes[i..i + 4];
        if w.iter().all(|b| b.is_ascii_alphabetic()) {
            if let Ok(s) = String::from_utf8(w.to_vec()) {
                windows.push((i, s));
            }
        }
    }
    if windows.is_empty() {
        return None;
    }
    // Una sola query con IN (...) para validar todas las ventanas.
    let mut sql = String::from("SELECT icao FROM airports WHERE icao IN (");
    for (k, _) in windows.iter().enumerate() {
        if k > 0 {
            sql.push(',');
        }
        sql.push('?');
    }
    sql.push(')');
    let mut q = sqlx::query_scalar::<_, String>(&sql);
    for (_, w) in &windows {
        q = q.bind(w.clone());
    }
    let valid: std::collections::HashSet<String> =
        q.fetch_all(pool).await.ok()?.into_iter().collect();
    if valid.is_empty() {
        return None;
    }
    let hits: Vec<&(usize, String)> = windows
        .iter()
        .filter(|(_, w)| valid.contains(w))
        .collect();
    // 1. Sufijo exacto.
    if let Some((_, w)) = hits.iter().find(|(i, _)| i + 4 == bytes.len()) {
        return Some(w.clone());
    }
    // 2. Prefijo exacto.
    if let Some((_, w)) = hits.iter().find(|(i, _)| *i == 0) {
        return Some(w.clone());
    }
    // 3. Único ICAO distinto.
    let distinct: std::collections::HashSet<&str> =
        hits.iter().map(|(_, w)| w.as_str()).collect();
    if distinct.len() == 1 {
        return Some(hits[0].1.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_folder_name_handles_dashes_and_underscores() {
        assert_eq!(
            pretty_folder_name("bravoairspace-airport-mdsd-las-americas"),
            "Bravoairspace Airport Mdsd Las Americas"
        );
        assert_eq!(
            pretty_folder_name("foo_bar-baz"),
            "Foo Bar Baz"
        );
        assert_eq!(pretty_folder_name(""), "");
    }

    #[test]
    fn icao_extraction_word_boundary() {
        assert_eq!(extract_icao("MDSD Las Americas"), Some("MDSD".to_string()));
        assert_eq!(extract_icao("bravoairspace-mdsd"), Some("MDSD".to_string()));
        // 5 letras consecutivas no machean ICAO de 4
        assert_eq!(extract_icao("MDSDX International"), None);
        assert_eq!(extract_icao("a320 livery"), None);
    }
}
