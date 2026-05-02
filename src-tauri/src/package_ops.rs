//! Operaciones sobre paquetes ya instalados en Community.
//!
//! Las dos primitivas que el usuario invoca desde el modal de
//! detalle son:
//!   · **Desinstalar** — borra el folder del paquete en Community.
//!   · **Abrir carpeta** — manejado por el frontend vía `openShell`.
//!
//! ¿Qué hay que borrar para una desinstalación "limpia"?
//!
//! MSFS guarda los paquetes Community **sólo** en el directorio
//! `Community/<folder>/`. La rolling-cache, los oficiales y los
//! caches de objetos viven aparte y no son por-paquete. Al borrar
//! el folder Community, MSFS deja de cargarlo en el siguiente
//! arranque y limpia las referencias automáticamente.
//!
//! Otros candidatos que evalué y descarté:
//!   · `LocalState\packages\<pkg>` — sólo se popula en MS Store
//!     para paquetes oficiales/marketplace, no Community.
//!   · `LOCALAPPDATA\Microsoft Flight Simulator\Packages\` — no
//!     existe para Community.
//!   · `Documents\MSFS Sceneries\` — convención de algunos
//!     instaladores .exe; lo notificamos al usuario en
//!     `install/mod.rs` cuando el archivo no es un paquete válido.
//!
//! Hacemos varios sanity checks antes del borrado para evitar
//! desastres si el `install_path` viene corrupto.

use std::path::{Path, PathBuf};

use crate::db::repo;

/// Errores propios del módulo. Mantenerlos tipados nos deja
/// devolver mensajes específicos al usuario ("path inválido", "no
/// existe", "fuera de Community"…) en vez de un string opaco.
#[derive(Debug, thiserror::Error)]
pub enum PackageOpError {
    #[error("la ruta '{0}' no existe")]
    NotFound(String),
    #[error("la ruta '{0}' no es un directorio")]
    NotADirectory(String),
    #[error("rechazado por seguridad: la ruta '{0}' parece sospechosa")]
    UnsafePath(String),
    #[error("no se pudo borrar '{path}': {source}")]
    DeleteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("paquete '{0}' no encontrado en la base de datos")]
    UnknownPackage(String),
    #[error(transparent)]
    Db(#[from] anyhow::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Borra el folder de un paquete instalado en Community y limpia
/// las filas de DB asociadas. La fila de `installed_addons` se
/// borra también porque ya no representa nada en disco.
pub async fn uninstall_by_folder(
    pool: &sqlx::SqlitePool,
    folder_name: &str,
) -> Result<(), PackageOpError> {
    let pkgs = repo::list_community_packages(pool).await?;
    let target = pkgs
        .iter()
        .find(|p| p.folder_name == folder_name)
        .ok_or_else(|| PackageOpError::UnknownPackage(folder_name.to_string()))?;

    let path = PathBuf::from(&target.install_path);
    sanity_check(&path)?;
    delete_dir(&path)?;
    cleanup_db(pool, folder_name).await?;
    Ok(())
}

fn sanity_check(path: &Path) -> Result<(), PackageOpError> {
    let display = path.to_string_lossy().to_string();
    if !path.exists() {
        return Err(PackageOpError::NotFound(display));
    }
    if !path.is_dir() {
        return Err(PackageOpError::NotADirectory(display));
    }
    // Negativa razonable: no permitir borrar la propia carpeta
    // Community o algo encima de ella. El paquete real siempre es
    // <community>/<paquete>; si el path tiene <2 componentes en
    // su parent chain hasta "Community", es sospechoso.
    let lower = display.to_lowercase();
    if !lower.contains("community") {
        return Err(PackageOpError::UnsafePath(display));
    }
    // Heurística adicional: el folder a borrar no debe ser el
    // propio "Community" (chequeo nombre exacto, case-insensitive).
    if path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("community"))
        .unwrap_or(false)
    {
        return Err(PackageOpError::UnsafePath(display));
    }
    Ok(())
}

fn delete_dir(path: &Path) -> Result<(), PackageOpError> {
    std::fs::remove_dir_all(path).map_err(|e| PackageOpError::DeleteFailed {
        path: path.to_string_lossy().into_owned(),
        source: e,
    })
}

async fn cleanup_db(pool: &sqlx::SqlitePool, folder_name: &str) -> Result<(), PackageOpError> {
    // Quitar de community_packages — el próximo scan lo confirmaría
    // de todas formas, pero hacerlo aquí mantiene la UI consistente
    // sin esperar al refresh.
    sqlx::query("DELETE FROM community_packages WHERE folder_name = ?1")
        .bind(folder_name)
        .execute(pool)
        .await?;
    // Quitar de installed_addons — buscamos por nombre. Manuales
    // tienen `name = folder` o el archivo, así que un LIKE razonable.
    sqlx::query(
        r#"
        DELETE FROM installed_addons
        WHERE name = ?1 OR install_path LIKE ?2
        "#,
    )
    .bind(folder_name)
    .bind(format!("%{}%", folder_name))
    .execute(pool)
    .await?;
    Ok(())
}
