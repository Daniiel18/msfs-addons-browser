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
    /// (v4.25.0) Nombres de paquete declarados en
    /// `manifest.dependencies[].name`. Siembran las aristas 'auto' del
    /// Link Map (livery → su aircraft base) sin que el usuario tenga
    /// que enlazarlos a mano.
    pub dependency_names: Vec<String>,
    /// (v4.25.0) Estado físico del paquete en el simulador. MSFS solo
    /// carga paquetes con `layout.json` presente — nuestro toggle lo
    /// renombra a `layout.json.disabled` para apagarlos sin mover la
    /// carpeta. El scanner refleja ese estado acá.
    pub enabled: bool,
    /// (v4.26.0) Nombres de las carpetas bajo `SimObjects/Airplanes`
    /// (y Rotorcraft) del paquete. Un aircraft base "posee" su
    /// contenedor (p.ej. `FNX_320`); las liveries lo referencian.
    pub simobject_dirs: Vec<String>,
    /// (v4.26.0) Valores de `base_container` extraídos de los
    /// aircraft.cfg del paquete (último componente del path, p.ej.
    /// `FNX_320`). Es la referencia REAL livery → aircraft base que
    /// usa MSFS — mucho más fiable que adivinar por folder name.
    pub base_containers: Vec<String>,
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

        // (v4.25.0) Estado enabled/disabled del paquete. Disabled =
        // nuestro toggle renombró `layout.json` → `layout.json.disabled`
        // (MSFS ignora paquetes sin layout). Si no hay NINGUNO de los
        // dos lo tratamos como enabled — paquetes raros sin layout no
        // deben aparecer como "apagados por SimFleet".
        let enabled = path.join("layout.json").is_file()
            || !path.join("layout.json.disabled").is_file();

        // (v4.26.0) Contenedores SimObjects propios + base_container
        // referenciados — alimentan el auto-link del Link Map.
        let (simobject_dirs, base_containers) = scan_simobjects(&path);

        match parse_manifest(&manifest_path) {
            Ok(raw) => {
                packages.push(materialize(
                    &path,
                    &folder_name,
                    raw,
                    enabled,
                    simobject_dirs,
                    base_containers,
                ));
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
                packages.push(materialize_fallback(
                    &path,
                    &folder_name,
                    enabled,
                    simobject_dirs,
                    base_containers,
                ));
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

/// (v4.26.0) Inspecciona `SimObjects/{Airplanes,Rotorcraft}` del
/// paquete: nombres de contenedores propios + los `base_container`
/// que referencian sus aircraft.cfg. Barato: lista 1-2 dirs y lee
/// los primeros KB de cada aircraft.cfg.
fn scan_simobjects(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut dirs: Vec<String> = Vec::new();
    let mut bases: Vec<String> = Vec::new();
    for kind in ["Airplanes", "Rotorcraft"] {
        let so = root.join("SimObjects").join(kind);
        let Ok(entries) = std::fs::read_dir(&so) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                dirs.push(name.to_string());
            }
            let cfg = path.join("aircraft.cfg");
            if let Ok(raw) = std::fs::read_to_string(&cfg) {
                // `base_container = "..\Asobo_A320_NEO"` (o con más ..\).
                // Tomamos el último componente del path entre comillas.
                for line in raw.lines().take(200) {
                    let lower = line.trim_start();
                    if !lower.to_ascii_lowercase().starts_with("base_container") {
                        continue;
                    }
                    if let Some(value) = line.split('=').nth(1) {
                        let value = value.trim().trim_matches('"');
                        let last = value
                            .rsplit(['\\', '/'])
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !last.is_empty() && !last.starts_with('.') {
                            bases.push(last);
                        }
                    }
                    break;
                }
            }
        }
    }
    bases.sort();
    bases.dedup();
    (dirs, bases)
}

fn parse_manifest(path: &Path) -> anyhow::Result<RawManifest> {
    let raw = std::fs::read_to_string(path)?;
    // Algunos manifests de la comunidad traen comentarios o trailing
    // commas — `serde_json::from_str` los rechaza. Sobre el universo
    // típico funciona, y los rotos caen al fallback.
    let parsed: RawManifest = serde_json::from_str(&raw)?;
    Ok(parsed)
}

fn materialize(
    path: &Path,
    folder_name: &str,
    raw: RawManifest,
    enabled: bool,
    simobject_dirs: Vec<String>,
    base_containers: Vec<String>,
) -> ScannedPackage {
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

    // (v4.25.0) Nombres de las dependencias del manifest. Cada entry
    // suele ser `{ "name": "fnx-aircraft-320", "package_version": … }`
    // — el name ES el folder name del paquete base, lo que nos permite
    // auto-enlazar livery → aircraft en el Link Map.
    let dependency_names: Vec<String> = raw
        .dependencies
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

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
        dependency_names,
        enabled,
        simobject_dirs,
        base_containers,
    }
}

fn materialize_fallback(
    path: &Path,
    folder_name: &str,
    enabled: bool,
    simobject_dirs: Vec<String>,
    base_containers: Vec<String>,
) -> ScannedPackage {
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
        dependency_names: Vec::new(),
        enabled,
        simobject_dirs,
        base_containers,
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
                folder_modified_at, dependencies_count, scanned_at, enabled
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), ?12)
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
                scanned_at = datetime('now'),
                enabled = excluded.enabled
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
        .bind(pkg.enabled)
        .execute(&mut *tx)
        .await?;
    }

    // (v4.25.0) Mantenimiento del grafo de enlaces (Link Map):
    //   1. Borra aristas y posiciones cuyos extremos ya no existen
    //      (paquete desinstalado fuera de la app).
    //   2. Re-siembra las aristas 'auto' desde manifest.dependencies —
    //      cada livery/soundpack declara su paquete base por folder
    //      name. Las aristas 'manual' del usuario nunca se tocan.
    sqlx::query(
        r#"
        DELETE FROM addon_links
        WHERE source_folder NOT IN (SELECT folder_name FROM community_packages)
           OR target_folder NOT IN (SELECT folder_name FROM community_packages)
        "#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        DELETE FROM addon_link_positions
        WHERE folder_name NOT IN (SELECT folder_name FROM community_packages)
        "#,
    )
    .execute(&mut *tx)
    .await?;
    {
        // (v4.26.0) Auto-link v2 — tres niveles de evidencia, en orden
        // de fiabilidad. Solo entre ADDONS (no SCENERY): el Link Map
        // es de aviones/liveries/sonidos, y los aeropuertos comparten
        // prefijos de vendor que generarían falsos enlaces.
        //   1. manifest.dependencies[].name == folder instalado.
        //   2. base_container del aircraft.cfg de la livery apunta al
        //      contenedor SimObjects de OTRO paquete (la referencia
        //      REAL que usa MSFS — cubre liveries cuyo folder no se
        //      parece en nada al avión, p.ej. "Etihad A6-API").
        //   3. Prefijo de folder: `fnx-aircraft-320-DLHDAIQSAL-CFM`
        //      empieza con `fnx-aircraft-320` (≥6 chars, el más largo
        //      gana).
        let is_addon =
            |p: &ScannedPackage| !matches_scenery(&p.content_type);
        let by_lower: std::collections::HashMap<String, &str> = report
            .packages
            .iter()
            .filter(|p| is_addon(p))
            .map(|p| (p.folder_name.to_lowercase(), p.folder_name.as_str()))
            .collect();
        // Dueño de cada contenedor SimObjects (en minúsculas).
        let mut owner_of_container: std::collections::HashMap<String, &str> =
            std::collections::HashMap::new();
        for p in report.packages.iter().filter(|p| is_addon(p)) {
            for d in &p.simobject_dirs {
                owner_of_container
                    .entry(d.to_lowercase())
                    .or_insert(p.folder_name.as_str());
            }
        }
        let mut seeded = 0usize;
        for pkg in report.packages.iter().filter(|p| is_addon(p)) {
            let mut sources: Vec<&str> = Vec::new();
            // Nivel 1 — dependencias del manifest.
            for dep in &pkg.dependency_names {
                if let Some(s) = by_lower.get(&dep.to_lowercase()) {
                    sources.push(s);
                }
            }
            // Nivel 2 — base_container del aircraft.cfg.
            for bc in &pkg.base_containers {
                if let Some(s) = owner_of_container.get(&bc.to_lowercase()) {
                    sources.push(s);
                }
            }
            // Nivel 3 — prefijo de folder (solo si 1 y 2 no dieron nada).
            if sources
                .iter()
                .all(|s| s.eq_ignore_ascii_case(&pkg.folder_name))
            {
                let pkg_lower = pkg.folder_name.to_lowercase();
                let mut best: Option<&str> = None;
                for (cand_lower, cand) in &by_lower {
                    if cand_lower.len() >= 6
                        && *cand_lower != pkg_lower
                        && pkg_lower.starts_with(cand_lower.as_str())
                        && best.map_or(true, |b| cand.len() > b.len())
                    {
                        best = Some(cand);
                    }
                }
                if let Some(b) = best {
                    sources.push(b);
                }
            }
            for source in sources {
                if source.eq_ignore_ascii_case(&pkg.folder_name) {
                    continue; // self-loop (el avión referencia su propio contenedor)
                }
                let r = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO addon_links
                        (source_folder, target_folder, origin)
                    VALUES (?1, ?2, 'auto')
                    "#,
                )
                .bind(source)
                .bind(&pkg.folder_name)
                .execute(&mut *tx)
                .await?;
                seeded += r.rows_affected() as usize;
            }
        }
        if seeded > 0 {
            tracing::info!(target: "scan", "auto-link: {} enlaces nuevos sembrados", seeded);
        }
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
        // (v4.24.1) Los packs librería/complemento NO se rescatan: el
        // rescue v4.24.0 promovía pseudo-ICAOs reales escondidos en
        // palabras comunes (ENHA en "enhanced", SING en "singapore",
        // VIGR en "navigraph") y llenó el mapa de night-lights/AIRAC
        // como si fueran aeropuertos (verificado en la DB del usuario:
        // 58 packs "Night Enhanced" rescatados por error).
        if is_library_pack(title.as_deref().unwrap_or(""), &folder) {
            continue;
        }
        let mut rescued: Option<String> = None;
        // (v4.27.0) PRIMERO buscar por NOMBRE del aeropuerto en
        // `airports`. "gakki-taiyuanwushu" tenía sufijo "USHU" (Uray)
        // pero name='Taiyuan Wusu International Airport' resuelve a
        // ZBYN — el dato real. Solo si no hay match por nombre caemos
        // al rescue por sufijo/prefijo.
        for text in [Some(folder.as_str()), title.as_deref()].into_iter().flatten() {
            if let Some(icao) = rescue_by_airport_name(pool, text).await {
                rescued = Some(icao);
                break;
            }
        }
        if rescued.is_none() {
            for text in [Some(folder.as_str()), title.as_deref()].into_iter().flatten() {
                if let Some(icao) = rescue_embedded_icao(pool, text).await {
                    rescued = Some(icao);
                    break;
                }
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

/// (v4.24.1) ¿El paquete es una LIBRERÍA / complemento y no un
/// aeropuerto? Única fuente de verdad para el mapa Y el dashboard: el
/// flag viaja en CommunityPackageRow (`isLibraryPack`) y el conteo de
/// AIRPORTS del dashboard lo usa igual — así ambos números coinciden
/// SIEMPRE y los packs de AIRAC/night lights/enhancements/excludes no
/// aparecen como aeropuertos (pedido del usuario).
pub fn is_library_pack(title: &str, folder_name: &str) -> bool {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?ix)\b(
              librar(?:y|ies) | developers?\s*pack | object\s*pack | asset\s*pack |
              jetways? | vehicles? | vehicle\s*pack | vegetation | trees? | grass |
              autogen | landmark\s*packs? | asobo\s*objects? | simobjects? | sdk | placeholder |
              airac | navdata | navigraph |
              night\s*-?\s*lights? | nightlights? | lights?\s*pack |
              enhancements? | enhanced |
              excludes? | excluder | merge | aerials? | ortho | mesh | photogrammetry |
              city\s*pack | citypack | static\s*aircraft |
              # v4.26.0 - GSX y sus packs de reemplazo NO son aeropuertos.
              # El rescue de prefijos validaba FSDR -fsdreamteam- contra
              # Desroches Airport y GSX Pro aparecia en el mapa; los
              # z-fsdreamteam-gsx-x de HAECO/CASL/catering igual.
              fsdreamteam-gsx | gsx[\s-]*pro | gsx\s*world
            )\b",
        )
        .unwrap()
    });
    let hay = format!("{} {}", title, folder_name);
    RE.is_match(&hay)
}

/// (v4.27.0) Resuelve ICAO por NOMBRE del aeropuerto. Tokeniza el
/// folder/title (incluyendo prefijos progresivos para "taiyuanwushu"
/// sin separador), busca cada token >=5 chars contra `airports.name`
/// (`LIKE '%token%'`), y devuelve el ICAO **sólo si todas las
/// coincidencias apuntan al mismo aeropuerto** (resultado único).
/// Caso real verificado: gakki-taiyuanwushu → "taiyuan" matchea
/// "Taiyuan Wusu International Airport" → ZBYN.
async fn rescue_by_airport_name(pool: &SqlitePool, text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut tokens: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in lower.split(|c: char| !c.is_ascii_alphabetic()) {
        if raw.len() >= 5 {
            tokens.insert(raw.to_string());
        }
        // Sin separador (`taiyuanwushu`) → probamos prefijos progresivos
        // de 5..=11 caracteres. Si "taiyuan" matchea único y nada más
        // contradice, ganamos.
        if raw.len() > 11 {
            for n in 5..=11 {
                tokens.insert(raw[..n].to_string());
            }
        }
    }
    if tokens.is_empty() {
        return None;
    }
    let mut winners: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for tok in &tokens {
        let pattern = format!("%{}%", tok);
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT icao FROM airports WHERE LOWER(name) LIKE ?1 LIMIT 2",
        )
        .bind(&pattern)
        .fetch_all(pool)
        .await
        .ok()?;
        // Tokens que matchean MUCHOS aeropuertos no son útiles
        // ("inter"/"airport" matchearían cientos) — los descartamos.
        if rows.len() == 1 {
            winners.insert(rows[0].clone());
        }
    }
    if winners.len() == 1 {
        return winners.into_iter().next();
    }
    None
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
    // (v4.24.1) SOLO sufijo o prefijo exactos. La regla anterior de
    // "único match interior" promovía pseudo-ICAOs escondidos en
    // palabras comunes (ENHA en "enhanced", SING en "singapore",
    // VIGR en "navigraph") y llenaba el mapa de basura.
    // 1. Sufijo exacto (`montevideosumu` → SUMU).
    if let Some((_, w)) = hits.iter().find(|(i, _)| i + 4 == bytes.len()) {
        return Some(w.clone());
    }
    // 2. Prefijo exacto (`sumu-montevideo` → SUMU).
    if let Some((_, w)) = hits.iter().find(|(i, _)| *i == 0) {
        return Some(w.clone());
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
