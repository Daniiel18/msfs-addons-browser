//! Operaciones sobre paquetes ya instalados en Community.
//!
//! Las dos primitivas que el usuario invoca desde el modal de
//! detalle son:
//!   · **Desinstalar** — borra el folder del paquete en Community.
//!   · **Abrir carpeta** — manejado por el frontend vía `openShell`.
//!
//! # Desinstalación total (v0.1.19)
//!
//! Antes: borrábamos sólo `Community/<folder>/` del path guardado en
//! `community_packages`. Eso funciona si el usuario tiene una sola
//! versión de MSFS — pero si conviven Steam + MS Store, o MSFS 2020
//! + MSFS 2024, el paquete podía quedar duplicado en la otra
//! Community y "resucitar" al siguiente scan.
//!
//! Ahora: detectamos **todas** las Community folders disponibles
//! (4 variantes en Windows — MSFS 2020/2024 × Steam/MS Store) y
//! borramos el folder en cada una donde exista. Además miramos las
//! ubicaciones "extras" que algunos instaladores .exe usan:
//!   · `Documents\MSFS Sceneries\<folder>` (Aerosoft, Drzewiecki…)
//!   · `Documents\MSFS 2024 Sceneries\<folder>`
//!
//! No tocamos la rolling-cache: MSFS la regenera al arranque, y
//! parsearla bien implica entender el formato binario propietario.
//!
//! Hacemos varios sanity checks antes del borrado para evitar
//! desastres si el `install_path` viene corrupto o si una entrada
//! del registro apunta a una carpeta inesperada (ver `sanity_check`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::community;
use crate::db::repo;

/// Errores propios del módulo. Mantenerlos tipados nos deja
/// devolver mensajes específicos al usuario ("path inválido", "no
/// existe", "fuera de Community"…) en vez de un string opaco.
#[derive(Debug, thiserror::Error)]
pub enum PackageOpError {
    #[error("paquete '{0}' no encontrado en la base de datos")]
    UnknownPackage(String),
    #[error("no se encontró ninguna ubicación instalada para '{0}'")]
    NothingToRemove(String),
    #[error(transparent)]
    Db(#[from] anyhow::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl PackageOpError {
    /// Mensaje localizado para el usuario final. `thiserror` sigue dando
    /// el `Display` en español para los logs; esto es lo que se usa en el
    /// boundary de comandos para lo que ve el frontend.
    pub fn localized(&self) -> String {
        match self {
            PackageOpError::UnknownPackage(name) => {
                crate::tr!("packageOps.unknownPackage", name = name)
            }
            PackageOpError::NothingToRemove(name) => {
                crate::tr!("packageOps.nothingToRemove", name = name)
            }
            // Transparentes: reenvían el mensaje del error subyacente.
            PackageOpError::Db(e) => e.to_string(),
            PackageOpError::Sqlx(e) => e.to_string(),
        }
    }
}

/// Reporte del trabajo hecho — qué paths fueron borrados con éxito,
/// cuáles fallaron, y cuántas filas de DB se limpiaron. El frontend
/// usa esto para enseñar al usuario "se borró de N ubicaciones" en
/// lugar del void anterior, que dejaba dudas sobre si la operación
/// realmente había sido total.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UninstallReport {
    /// Folder name del paquete (el mismo que entró por parámetro).
    pub folder_name: String,
    /// Paths que se borraron correctamente. Cada entrada es la ruta
    /// absoluta que se eliminó (`Community/<folder>` o
    /// `Documents\MSFS Sceneries\<folder>`, etc.).
    pub removed_paths: Vec<String>,
    /// Paths que se intentaron borrar pero fallaron. Cada entrada es
    /// `(path, error)` — el frontend los muestra para que el usuario
    /// pueda intervenir manualmente si MSFS está corriendo y bloquea
    /// el delete por handles abiertos.
    pub failed_paths: Vec<RemovalFailure>,
    /// (05/11) Carpetas reales de los enlaces quitados, que siguen en disco.
    pub kept_targets: Vec<String>,
    /// Cuántas filas se borraron en `community_packages` +
    /// `installed_addons`.
    pub db_rows_cleared: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalFailure {
    pub path: String,
    pub error: String,
}

/// Borra el folder de un paquete instalado en **todas** las
/// ubicaciones donde aparezca + limpia DB. Idempotente: ejecutarlo
/// dos veces no es error (la segunda vez no encontrará nada).
///
/// El folder name proviene de `community_packages` (la fila escaneada
/// del FS) o, si no existe esa fila, se borra usando el folder name
/// tal cual — útil para "fantasmas" donde la DB se desincronizó.
///
/// (05/11) Si el paquete es un enlace, borrarlo quita solo el enlace. Con
/// `delete_target` se borra además la carpeta real, salvo que sea de un
/// instalador; lo que se quede se cuenta en `kept_targets`.
pub async fn uninstall_by_folder(
    pool: &sqlx::SqlitePool,
    folder_name: &str,
    delete_target: bool,
) -> Result<UninstallReport, PackageOpError> {
    let mut report = UninstallReport {
        folder_name: folder_name.to_string(),
        ..Default::default()
    };

    // Candidato 1: el `install_path` exacto de la DB. Es el más
    // fiable porque fue el que escaneamos — si el usuario configuró
    // una Community en un disco que `detect_all_community_folders`
    // no descubre (raro pero posible), seguimos pudiendo borrarlo.
    let pkgs = repo::list_community_packages(pool).await?;
    let db_target = pkgs.iter().find(|p| p.folder_name == folder_name);

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(t) = db_target {
        candidates.push(PathBuf::from(&t.install_path));
    }

    // Candidatos 2-N: una entrada por cada Community detectada
    // automáticamente. Esto cubre el caso del usuario que tiene
    // Steam + MS Store o MSFS 2020 + 2024 — el paquete puede vivir
    // en varias y queremos eliminarlo de todas.
    match community::detect_all_community_folders() {
        Ok(folders) => {
            for folder in folders {
                if !folder.exists {
                    continue;
                }
                let candidate = PathBuf::from(&folder.path).join(folder_name);
                if !already_listed(&candidates, &candidate) {
                    candidates.push(candidate);
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "install",
                "uninstall: detect_all_community_folders falló ({:#}); sigo con candidatos de DB",
                e
            );
        }
    }

    // (05/11) Community2024 de MSFS 2024.
    for folder in community::secondary_community_folders() {
        let candidate = folder.join(folder_name);
        if !already_listed(&candidates, &candidate) {
            candidates.push(candidate);
        }
    }

    // Candidatos extras: `Documents\MSFS Sceneries\<folder>` y
    // `Documents\MSFS 2024 Sceneries\<folder>`. Algunos instaladores
    // .exe ponen ahí los assets — si están, hay que borrarlos también.
    for (label, base) in community::extra_install_locations() {
        let candidate = base.join(folder_name);
        if !already_listed(&candidates, &candidate) {
            tracing::debug!(
                target: "install",
                "uninstall: añadiendo candidato extra ({}): {}",
                label,
                candidate.display()
            );
            candidates.push(candidate);
        }
    }

    // Detectar si la entrada principal (DB) es un enlace. Si lo es,
    // NO borrar carpetas reales de otras Community — podrían ser la
    // fuente del enlace u otra instalación independiente.
    let primary_is_link = db_target
        .map(|t| {
            std::fs::symlink_metadata(Path::new(&t.install_path))
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    // Intentar borrar cada candidato. Sanity check antes de cada
    // delete — un path mal formado en DB no debe poder borrar `/`
    // ni `Community/`. Los que no existen se omiten silenciosamente
    // (eso es lo que queremos para idempotencia).
    let primary_path = db_target.map(|t| PathBuf::from(&t.install_path));
    for candidate in &candidates {
        // El nombre `display` choca con un token reservado por las
        // macros de `tracing` (lo interpreta como helper formatter
        // en vez de variable local), por eso usamos `path_str`.
        let path_str = candidate.to_string_lossy().to_string();
        // (fix) `exists()` SIGUE el reparse point de un junction cross-link a su
        // target; si el target ya se borró (la carpeta REAL va primero), da
        // false y el junction quedaba HUÉRFANO. `symlink_metadata` no lo sigue →
        // detecta el junction colgante para poder borrarlo.
        let Ok(link_meta) = std::fs::symlink_metadata(candidate) else {
            continue;
        };
        let is_junction = link_meta.file_type().is_symlink();
        // (audit) Si la entrada primaria es un enlace, no borrar carpetas
        // reales de otras Community: podrían ser el target u otra instalación.
        let is_primary = primary_path.as_ref().is_some_and(|p| paths_equal(p, candidate));
        if primary_is_link && !is_junction && !is_primary {
            tracing::info!(
                target: "install",
                "uninstall: omitido {} (carpeta real en otra Community; la primaria es enlace)",
                candidate.display()
            );
            continue;
        }
        if let Err(reason) = sanity_check(candidate, folder_name) {
            tracing::warn!(
                target: "install",
                path = %path_str,
                reason = %reason,
                "uninstall: sanity_check rechazó path"
            );
            report.failed_paths.push(RemovalFailure {
                path: path_str.clone(),
                error: reason,
            });
            continue;
        }
        let link = crate::folder_link::read(candidate).filter(|link| !link.broken);
        // Un junction/symlink se borra con remove_dir (quita SÓLO el reparse
        // point, nunca su target); una carpeta real con remove_dir_all.
        let removal = if is_junction {
            std::fs::remove_dir(candidate).or_else(|_| std::fs::remove_file(candidate))
        } else {
            std::fs::remove_dir_all(candidate)
        };
        match removal {
            Ok(()) => {
                tracing::info!(
                    target: "install",
                    path = %path_str,
                    "uninstall: borrado"
                );
                report.removed_paths.push(path_str);
                if let Some(link) = link {
                    let target = link.target.to_string_lossy().into_owned();
                    if delete_target && !link.managed {
                        match remove_link_target(&link.target, folder_name) {
                            Ok(()) => report.removed_paths.push(target),
                            Err(e) => report.failed_paths.push(RemovalFailure {
                                path: target,
                                error: e.to_string(),
                            }),
                        }
                    } else {
                        report.kept_targets.push(target);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "install",
                    path = %path_str,
                    error = %e,
                    "uninstall: fallo borrando"
                );
                report.failed_paths.push(RemovalFailure {
                    path: path_str,
                    error: e.to_string(),
                });
            }
        }
    }

    // Otra Community pudo tener la carpeta real: entonces ya no se queda.
    let removed = report.removed_paths.clone();
    report
        .kept_targets
        .retain(|kept| !removed.iter().any(|r| paths_equal(Path::new(r), Path::new(kept))));

    // Limpiamos DB siempre que algo se haya borrado, O cuando no
    // existía nada en disco pero sí filas en DB (fantasma). Si
    // todo falló, dejamos las filas para que el usuario pueda
    // reintentar tras cerrar MSFS.
    let any_success = !report.removed_paths.is_empty();
    let all_missing = report.removed_paths.is_empty() && report.failed_paths.is_empty();
    if any_success || all_missing {
        report.db_rows_cleared = cleanup_db(pool, folder_name).await?;
    }

    if report.removed_paths.is_empty() && report.failed_paths.is_empty() && db_target.is_none() {
        return Err(PackageOpError::NothingToRemove(folder_name.to_string()));
    }

    Ok(report)
}

/// (05/11) Borra la carpeta real de un enlace. Solo si conserva el nombre del
/// paquete y es una carpeta de verdad, para no llevarse nada que no sea el addon.
fn remove_link_target(target: &Path, folder_name: &str) -> std::io::Result<()> {
    let invalid = || std::io::Error::other(crate::tr!("packageOps.linkTargetInvalid"));
    let same_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(folder_name));
    if !same_name || target.components().count() < 3 {
        return Err(invalid());
    }
    if std::fs::symlink_metadata(target)?.file_type().is_symlink() {
        return Err(invalid());
    }
    // Con Community de pruebas, nada fuera de su carpeta se toca.
    if let Some(root) = crate::test_env::community_override() {
        if !target.starts_with(root.parent().unwrap_or(&root)) {
            return Err(invalid());
        }
    }
    std::fs::remove_dir_all(target)
}

/// (04/11) Qué carpeta hay que quitar después de instalar lo que sustituye a
/// `replaces`. Si lo nuevo llegó en esa misma carpeta, ya la reemplazó la
/// instalación; si no se instaló ningún paquete (un instalador .exe, un perfil
/// de GSX), lo que había se queda. `installed`: nombres o rutas de carpeta.
pub fn folder_to_remove<'a>(replaces: Option<&'a str>, installed: &[String]) -> Option<&'a str> {
    let old = replaces?.trim();
    if old.is_empty() || installed.is_empty() {
        return None;
    }
    let same_folder = installed.iter().any(|p| {
        let name = Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or(p);
        // Windows no distingue mayúsculas: sería borrar lo recién instalado.
        name.eq_ignore_ascii_case(old)
    });
    (!same_folder).then_some(old)
}

fn already_listed(list: &[PathBuf], candidate: &Path) -> bool {
    list.iter().any(|p| paths_equal(p, candidate))
}

/// Comparación de paths case-insensitive en Windows (donde un mismo
/// archivo puede escribirse como `C:\Foo` o `c:\foo`). En el resto
/// de plataformas hacemos comparación literal — no aplicamos
/// canonicalize porque eso requiere que el path exista físicamente y
/// puede fallar para candidatos válidos no instalados.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let sa = a.to_string_lossy().to_lowercase();
    let sb = b.to_string_lossy().to_lowercase();
    sa.trim_end_matches(['/', '\\']) == sb.trim_end_matches(['/', '\\'])
}

/// Verifica que el path a borrar es razonable. Devuelve `Err(razón)`
/// con un mensaje legible para logs si rechaza el path.
///
/// Reglas:
///   · El nombre del último componente debe coincidir con
///     `expected_folder` (case-insensitive). Esto bloquea cualquier
///     manipulación que intente borrar la propia carpeta Community
///     o un directorio padre.
///   · El path debe contener "community" o "sceneries" en alguna
///     parte (case-insensitive). Filtro defensivo contra valores
///     corruptos en DB que apunten a `C:\Windows` o similares.
fn sanity_check(path: &Path, expected_folder: &str) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !name.eq_ignore_ascii_case(expected_folder) {
        return Err(crate::tr!(
            "packageOps.sanityNameMismatch",
            name = name,
            expected = expected_folder
        ));
    }
    let lower = path.to_string_lossy().to_lowercase();
    if !(lower.contains("community") || lower.contains("sceneries")) {
        return Err(crate::tr!("packageOps.sanityNotCommunity"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// (v4.25.0) Enable / Disable de paquetes + cascada por el Link Map
// ---------------------------------------------------------------------------

/// Reporte del toggle. `changed` lista los folders cuyo estado cambió
/// (incluye los alcanzados por cascada) para que la UI anime los
/// switches en cadena; `failed` los que no se pudieron tocar (p.ej.
/// archivo bloqueado) con su error.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToggleReport {
    pub enabled: bool,
    pub changed: Vec<String>,
    pub failed: Vec<RemovalFailure>,
}

/// Renombra `layout.json` ⇄ `layout.json.disabled` dentro del paquete.
/// MSFS solo carga paquetes con `layout.json` presente, así que esto
/// los apaga/enciende sin mover la carpeta (instantáneo aun con addons
/// de varios GB, reversible al 100%). Devuelve `true` si el FS cambió.
fn toggle_layout_file(install_path: &Path, enabled: bool) -> std::io::Result<bool> {
    // Con Community de pruebas, nada fuera de ella se toca.
    if let Some(root) = crate::test_env::community_override() {
        let inside = install_path.starts_with(&root)
            || community::secondary_community_folders().iter().any(|r| install_path.starts_with(r));
        if !inside {
            return Err(std::io::Error::other("fuera de la Community de pruebas"));
        }
    }
    let is_link = std::fs::symlink_metadata(install_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_link {
        return toggle_junction(install_path, enabled);
    }
    // (04/11) Renombrar mueve la fecha de la carpeta, y esa fecha dice «el addon
    // cambió»: decide si se recalcula el tamaño y si un hotfix ya se instaló.
    // Encender y apagar no cambia el addon, así que se deja como estaba.
    let folder_time = std::fs::metadata(install_path).and_then(|m| m.modified()).ok();
    let changed = rename_layout(install_path, enabled)?;
    if let (true, Some(time)) = (changed, folder_time) {
        if let Err(e) = set_folder_time(install_path, time) {
            tracing::debug!(target: "install", "toggle: fecha de {}: {e}", install_path.display());
        }
    }
    Ok(changed)
}

#[cfg(windows)]
fn set_folder_time(dir: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    // Sin este permiso Windows no deja abrir una carpeta.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)?
        .set_modified(time)
}

#[cfg(not(windows))]
fn set_folder_time(_dir: &Path, _time: std::time::SystemTime) -> std::io::Result<()> {
    Ok(())
}

fn toggle_junction(install_path: &Path, enabled: bool) -> std::io::Result<bool> {
    let disabled = install_path.with_extension("disabled");
    if enabled {
        if install_path.exists() || std::fs::symlink_metadata(install_path).is_ok() {
            return Ok(false);
        }
        if std::fs::symlink_metadata(&disabled).is_ok() {
            std::fs::rename(&disabled, install_path)?;
            return Ok(true);
        }
        Ok(false)
    } else {
        if std::fs::symlink_metadata(install_path).is_err() {
            return Ok(false);
        }
        if std::fs::symlink_metadata(&disabled).is_ok() {
            let _ = std::fs::remove_dir(&disabled);
        }
        std::fs::rename(install_path, &disabled)?;
        Ok(true)
    }
}

fn rename_layout(install_path: &Path, enabled: bool) -> std::io::Result<bool> {
    let on = install_path.join("layout.json");
    let off = install_path.join("layout.json.disabled");
    if enabled {
        if on.exists() {
            return Ok(false); // ya activo
        }
        if off.exists() {
            std::fs::rename(&off, &on)?;
            return Ok(true);
        }
        Ok(false) // paquete sin layout — nada que restaurar
    } else {
        if on.exists() {
            // Si quedó un .disabled huérfano de un toggle anterior lo
            // pisamos: rename falla en Windows si el destino existe.
            if off.exists() {
                std::fs::remove_file(&off)?;
            }
            std::fs::rename(&on, &off)?;
            return Ok(true);
        }
        Ok(false) // ya inactivo (o sin layout)
    }
}

/// Calcula el cierre transitivo hacia ABAJO del grafo de links
/// (source → target) partiendo de `root`. BFS con set de visitados —
/// cycle-safe por construcción (un nodo nunca se encola dos veces) y
/// asimétrico (solo seguimos aristas salientes: encender una livery
/// NO enciende su aircraft; encender el aircraft SÍ enciende sus
/// liveries/sonidos a cualquier profundidad).
fn transitive_targets(
    root: &str,
    links: &[(String, String)],
) -> Vec<String> {
    use std::collections::{HashSet, VecDeque};
    let mut visited: HashSet<String> = HashSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    visited.insert(root.to_lowercase());
    queue.push_back(root.to_string());
    while let Some(current) = queue.pop_front() {
        order.push(current.clone());
        for (source, target) in links {
            if source.eq_ignore_ascii_case(&current)
                && visited.insert(target.to_lowercase())
            {
                queue.push_back(target.clone());
            }
        }
    }
    order
}

/// Enlaces que sigue un interruptor. Los que tienen condición son de un vuelo,
/// y aquí no hay vuelo.
fn cascade_links(links: Vec<repo::AddonLinkRow>) -> Vec<(String, String)> {
    links
        .into_iter()
        .filter(|l| l.conditions.is_none())
        .map(|l| (l.source_folder, l.target_folder))
        .collect()
}

/// Enciende/apaga un paquete y, con `cascade`, todos sus dependientes
/// enlazados (transitivo, cycle-safe). Actualiza el FS primero y la
/// columna `enabled` después — la DB es espejo, el FS manda.
pub async fn set_package_enabled(
    pool: &sqlx::SqlitePool,
    folder_name: &str,
    enabled: bool,
    cascade: bool,
) -> Result<ToggleReport, PackageOpError> {
    let pkgs = repo::list_community_packages(pool).await?;
    let by_folder: std::collections::HashMap<String, &repo::CommunityPackageRow> = pkgs
        .iter()
        .map(|p| (p.folder_name.to_lowercase(), p))
        .collect();
    if !by_folder.contains_key(&folder_name.to_lowercase()) {
        return Err(PackageOpError::UnknownPackage(folder_name.to_string()));
    }

    let affected: Vec<String> = if cascade {
        transitive_targets(folder_name, &cascade_links(repo::list_addon_links(pool).await?))
    } else {
        vec![folder_name.to_string()]
    };

    let mut report = ToggleReport {
        enabled,
        ..Default::default()
    };
    let extra_folders = community::detect_all_community_folders().unwrap_or_default();
    for folder in &affected {
        let Some(pkg) = by_folder.get(&folder.to_lowercase()) else {
            continue;
        };
        match toggle_layout_file(Path::new(&pkg.install_path), enabled) {
            Ok(_) => {
                sqlx::query("UPDATE community_packages SET enabled = ?1 WHERE folder_name = ?2")
                    .bind(enabled)
                    .bind(&pkg.folder_name)
                    .execute(pool)
                    .await?;
                report.changed.push(pkg.folder_name.clone());
                tracing::info!(
                    target: "install",
                    "toggle: '{}' → {}",
                    pkg.folder_name,
                    if enabled { "enabled" } else { "disabled" }
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "install",
                    "toggle: '{}' falló: {}",
                    pkg.folder_name,
                    e
                );
                report.failed.push(RemovalFailure {
                    path: pkg.install_path.clone(),
                    error: e.to_string(),
                });
            }
        }
        for cf in &extra_folders {
            if !cf.exists { continue; }
            let extra = PathBuf::from(&cf.path).join(&pkg.folder_name);
            if extra == Path::new(&pkg.install_path) { continue; }
            if std::fs::symlink_metadata(&extra).is_ok() {
                let _ = toggle_layout_file(&extra, enabled);
            }
        }
    }
    Ok(report)
}

/// Toggle masivo (Enable All / Disable All). El frontend pasa la lista
/// explícita de folders en scope (p.ej. solo los addons no-escenario
/// visibles) — el backend no decide el alcance. Sin cascada: la lista
/// ya es exhaustiva.
pub async fn set_packages_enabled(
    pool: &sqlx::SqlitePool,
    folder_names: &[String],
    enabled: bool,
) -> Result<ToggleReport, PackageOpError> {
    let pkgs = repo::list_community_packages(pool).await?;
    let by_folder: std::collections::HashMap<String, &repo::CommunityPackageRow> = pkgs
        .iter()
        .map(|p| (p.folder_name.to_lowercase(), p))
        .collect();
    let mut report = ToggleReport {
        enabled,
        ..Default::default()
    };
    for folder in folder_names {
        let Some(pkg) = by_folder.get(&folder.to_lowercase()) else {
            continue;
        };
        match toggle_layout_file(Path::new(&pkg.install_path), enabled) {
            Ok(_) => {
                sqlx::query("UPDATE community_packages SET enabled = ?1 WHERE folder_name = ?2")
                    .bind(enabled)
                    .bind(&pkg.folder_name)
                    .execute(pool)
                    .await?;
                report.changed.push(pkg.folder_name.clone());
            }
            Err(e) => {
                report.failed.push(RemovalFailure {
                    path: pkg.install_path.clone(),
                    error: e.to_string(),
                });
            }
        }
    }
    tracing::info!(
        target: "install",
        "toggle masivo → {}: {} cambiados, {} fallaron",
        if enabled { "enabled" } else { "disabled" },
        report.changed.len(),
        report.failed.len()
    );
    Ok(report)
}

async fn cleanup_db(pool: &sqlx::SqlitePool, folder_name: &str) -> Result<u64, PackageOpError> {
    // Quitar de community_packages — el próximo scan lo confirmaría
    // de todas formas, pero hacerlo aquí mantiene la UI consistente
    // sin esperar al refresh.
    let r1 = sqlx::query("DELETE FROM community_packages WHERE folder_name = ?1")
        .bind(folder_name)
        .execute(pool)
        .await?;
    // Quitar de installed_addons — buscamos por nombre. Manuales
    // tienen `name = folder` o el archivo, así que un LIKE razonable.
    // (fix) Antes: `install_path LIKE '%folder%'` — borraba filas de OTROS
    // addons cuyo path CONTUVIERA este folder como subcadena, y `_`/`%` (muy
    // comunes en nombres MSFS: FNX_320) actuaban como comodines LIKE sin
    // escapar. Ahora: escapamos los comodines y anclamos al SEPARADOR de path
    // final (`\folder`), así sólo casa el paquete exacto, no `FNX_320_liveries`.
    let esc = folder_name
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    let r2 = sqlx::query(
        r#"
        DELETE FROM installed_addons
        WHERE name = ?1 OR install_path LIKE ?2 ESCAPE '!'
        "#,
    )
    .bind(folder_name)
    .bind(format!("%\\{}", esc))
    .execute(pool)
    .await?;
    // (v6.2.63) Quitar el tracking de updates de flightsim.to de esta carpeta:
    // si el usuario desinstaló el addon, NO deben salirle updates de algo que
    // ya no tiene instalado (ni en el badge ni en la campana).
    let r3 = sqlx::query("DELETE FROM flightsim_tracked_files WHERE folder_name = ?1")
        .bind(folder_name)
        .execute(pool)
        .await?;
    let r4 = sqlx::query("DELETE FROM catalog_installs WHERE folder_name = ?1")
        .bind(folder_name)
        .execute(pool)
        .await?;
    Ok(r1.rows_affected() + r2.rows_affected() + r3.rows_affected() + r4.rows_affected())
}

#[cfg(all(test, windows))]
mod link_tests {
    use super::*;
    use crate::folder_link::tests::junction;

    async fn pool_with(folder: &str, install_path: &Path) -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO community_packages (folder_name, install_path, title) VALUES (?1, ?2, ?1)")
            .bind(folder)
            .bind(install_path.to_string_lossy().into_owned())
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    /// Community con un enlace al addon de una carpeta aparte.
    fn linked(tmp: &Path, folder: &str, real: &Path) -> PathBuf {
        let community = tmp.join("Community");
        std::fs::create_dir_all(&community).unwrap();
        std::fs::create_dir_all(real).unwrap();
        std::fs::write(real.join("manifest.json"), "{}").unwrap();
        let link = community.join(folder);
        junction(&link, real);
        link
    }

    #[tokio::test]
    async fn a_condition_stops_the_cascade_and_only_manual_links_can_have_one() {
        let pool = pool_with("a", Path::new("C:/no-existe/a")).await;
        for (source, target) in [("a", "b"), ("b", "c"), ("a", "d")] {
            repo::add_addon_link(&pool, source, target).await.unwrap();
        }
        sqlx::query("INSERT INTO addon_links (source_folder, target_folder, origin) VALUES ('a', 'e', 'auto')")
            .execute(&pool)
            .await
            .unwrap();
        let brazil = crate::link_conditions::LinkConditions { country: vec!["BR".into()], ..Default::default() };

        assert!(repo::set_addon_link_conditions(&pool, "a", "b", Some(&brazil)).await.unwrap());
        assert!(!repo::set_addon_link_conditions(&pool, "a", "e", Some(&brazil)).await.unwrap(), "automático");

        let links = repo::list_addon_links(&pool).await.unwrap();
        let conditional = links.iter().find(|l| l.target_folder == "b").unwrap();
        assert_eq!(conditional.conditions.as_ref(), Some(&brazil));
        assert_eq!(transitive_targets("a", &cascade_links(links)), ["a", "d", "e"], "b y lo que cuelga de b quedan fuera");

        assert!(repo::set_addon_link_conditions(&pool, "a", "b", None).await.unwrap());
        let links = repo::list_addon_links(&pool).await.unwrap();
        assert!(links.iter().all(|l| l.conditions.is_none()));
        assert!(transitive_targets("a", &cascade_links(links)).contains(&"c".to_string()));
    }

    #[tokio::test]
    async fn by_default_only_the_link_goes_and_the_report_says_where_the_files_are() {
        let folder = "simfleet-test-enlace-solo";
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Addons").join(folder);
        let link = linked(tmp.path(), folder, &real);
        let pool = pool_with(folder, &link).await;

        let report = uninstall_by_folder(&pool, folder, false).await.unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err(), "el enlace se fue");
        assert!(real.join("manifest.json").is_file(), "los archivos siguen");
        assert_eq!(report.kept_targets.len(), 1);
        assert!(report.kept_targets[0].ends_with(folder));
        assert!(report.failed_paths.is_empty());
    }

    #[tokio::test]
    async fn the_real_folder_goes_too_when_asked() {
        let folder = "simfleet-test-enlace-todo";
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Addons").join(folder);
        let link = linked(tmp.path(), folder, &real);
        let pool = pool_with(folder, &link).await;

        let report = uninstall_by_folder(&pool, folder, true).await.unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(!real.exists(), "se borró la carpeta real");
        assert!(report.kept_targets.is_empty());
        assert_eq!(report.removed_paths.len(), 2);
    }

    #[tokio::test]
    async fn an_installers_folder_is_never_deleted() {
        let folder = "simfleet-test-enlace-instalador";
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("App").join("efb");
        let link = linked(tmp.path(), folder, &real);
        let pool = pool_with(folder, &link).await;

        let report = uninstall_by_folder(&pool, folder, true).await.unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(real.join("manifest.json").is_file(), "es del instalador");
        assert_eq!(report.kept_targets.len(), 1);
    }

    #[test]
    fn a_folder_that_is_not_the_addons_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tmp.path().join("Addons").join("otra-cosa");
        std::fs::create_dir_all(&other).unwrap();

        assert!(remove_link_target(&other, "acme-airport").is_err());
        assert!(other.is_dir());
    }
}

#[cfg(test)]
mod replace_tests {
    use super::folder_to_remove;

    fn folders(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_same_folder_is_never_removed() {
        let installed = folders(&[r"B:\Community\acme-airport"]);
        assert_eq!(folder_to_remove(Some("acme-airport"), &installed), None);
        assert_eq!(folder_to_remove(Some("Acme-Airport"), &installed), None, "mayúsculas");
        let names = folders(&["otro", "acme-airport"]);
        assert_eq!(folder_to_remove(Some("acme-airport"), &names), None, "archivo con varios");
    }

    #[test]
    fn a_new_folder_removes_the_old_one() {
        let installed = folders(&[r"B:\Community\acme-airport-2024"]);
        assert_eq!(folder_to_remove(Some("acme-airport"), &installed), Some("acme-airport"));
    }

    #[test]
    fn without_a_new_package_nothing_is_removed() {
        assert_eq!(folder_to_remove(Some("acme-airport"), &[]), None);
        assert_eq!(folder_to_remove(None, &folders(&["otro"])), None);
        assert_eq!(folder_to_remove(Some("  "), &folders(&["otro"])), None);
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::*;

    /// (04/11) Enfocar no debe parecer una actualización de cada addon.
    #[test]
    fn toggling_keeps_the_folder_date() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("addon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("layout.json"), "{}").unwrap();
        let date = |d: &Path| std::fs::metadata(d).unwrap().modified().unwrap();
        let before = date(&dir);
        std::thread::sleep(std::time::Duration::from_millis(50));

        assert!(toggle_layout_file(&dir, false).unwrap());
        assert!(dir.join("layout.json.disabled").is_file());
        assert_eq!(date(&dir), before);
        assert!(toggle_layout_file(&dir, true).unwrap());
        assert!(dir.join("layout.json").is_file());
        assert_eq!(date(&dir), before);
        // Sin cambios no se toca nada.
        assert!(!toggle_layout_file(&dir, true).unwrap());
    }
}
