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
pub async fn uninstall_by_folder(
    pool: &sqlx::SqlitePool,
    folder_name: &str,
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

    // Intentar borrar cada candidato. Sanity check antes de cada
    // delete — un path mal formado en DB no debe poder borrar `/`
    // ni `Community/`. Los que no existen se omiten silenciosamente
    // (eso es lo que queremos para idempotencia).
    for candidate in &candidates {
        // El nombre `display` choca con un token reservado por las
        // macros de `tracing` (lo interpreta como helper formatter
        // en vez de variable local), por eso usamos `path_str`.
        let path_str = candidate.to_string_lossy().to_string();
        if !candidate.exists() {
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
        match std::fs::remove_dir_all(candidate) {
            Ok(()) => {
                tracing::info!(
                    target: "install",
                    path = %path_str,
                    "uninstall: borrado"
                );
                report.removed_paths.push(path_str);
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
        return Err(format!(
            "el último componente '{}' no coincide con el folder esperado '{}'",
            name, expected_folder
        ));
    }
    let lower = path.to_string_lossy().to_lowercase();
    if !(lower.contains("community") || lower.contains("sceneries")) {
        return Err(
            "el path no contiene 'community' ni 'sceneries' — sospechoso".to_string(),
        );
    }
    Ok(())
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
    let r2 = sqlx::query(
        r#"
        DELETE FROM installed_addons
        WHERE name = ?1 OR install_path LIKE ?2
        "#,
    )
    .bind(folder_name)
    .bind(format!("%{}%", folder_name))
    .execute(pool)
    .await?;
    Ok(r1.rows_affected() + r2.rows_affected())
}
