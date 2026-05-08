use std::path::PathBuf;

use crate::community::{self, CommunityInfo};
use crate::community_scanner::{self, ScanReport};
use crate::db::repo::{self, CommunityPackageRow};
use crate::package_ops;
use crate::updates::{self, AvailableUpdate, RefreshSummary};
use crate::AppState;

/// Escanea Community y persiste los paquetes encontrados. Acepta
/// un path opcional — si no viene, se intenta detección automática.
#[tauri::command]
pub async fn scan_community(
    community_path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ScanReport, String> {
    let path = resolve_path(community_path).await?;
    let path_for_task = path.clone();
    let report = tokio::task::spawn_blocking(move || community_scanner::scan(&path_for_task))
        .await
        .map_err(|e| format!("la tarea de escaneo falló: {e}"))?
        .map_err(|e| e.to_string())?;
    community_scanner::sync_to_db(&state.db, &report)
        .await
        .map_err(|e| e.to_string())?;
    Ok(report)
}

/// Lista los paquetes en Community tal como están en la DB. No
/// dispara escaneo — el frontend usa esto para pintar la sidebar
/// del mapa al instante.
#[tauri::command]
pub async fn list_community_packages(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CommunityPackageRow>, String> {
    repo::list_community_packages(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Devuelve las actualizaciones detectadas a partir de la cache
/// (no toca la red). Combinado con `refresh_updates_for_installed`
/// permite la UX "abre el panel y mira las que ya conocemos; pulsa
/// refrescar para ir a buscar más".
#[tauri::command]
pub async fn list_available_updates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AvailableUpdate>, String> {
    updates::compute_available(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Hace un barrido activo: por cada ICAO instalado con versión,
/// pregunta a cada fuente qué tiene. Los resultados se cachean en
/// `addons` y luego pueden detectarse via `list_available_updates`.
#[tauri::command]
pub async fn refresh_updates_for_installed(
    state: tauri::State<'_, AppState>,
) -> Result<RefreshSummary, String> {
    updates::refresh_for_installed(&state.db, &state.sources)
        .await
        .map_err(|e| e.to_string())
}

/// Desinstala un paquete por nombre de folder. Borra el directorio
/// en Community y limpia las filas de DB asociadas. Devuelve un
/// scan report fresco para que el frontend reconcilie la lista.
#[tauri::command]
pub async fn uninstall_community_package(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    package_ops::uninstall_by_folder(&state.db, &folder_name)
        .await
        .map_err(|e| e.to_string())
}

/// Diagnóstico exhaustivo del estado de detección de updates para
/// un folder concreto. Devuelve toda la cadena: paquete escaneado,
/// match de aeropuerto, entradas del catálogo, cache de chequeos y
/// la razón explícita por la que la update no aparece (si es el caso).
///
/// La UI lo invoca desde un botón en el modal de detalle, para que
/// el usuario vea qué eslabón rompe la cadena sin tener que mirar
/// logs ni editar SQL.
#[tauri::command]
pub async fn diagnose_update_for_package(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<repo::UpdateDiagnostic, String> {
    repo::diagnose_for_folder(&state.db, &folder_name)
        .await
        .map_err(|e| e.to_string())
}

/// Marca una update como vista — desaparece del panel hasta que
/// el usuario pulse "Recargar" o instale el paquete.
#[tauri::command]
pub async fn dismiss_update(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::dismiss_update(&state.db, &folder_name)
        .await
        .map_err(|e| e.to_string())
}

/// Marca todas las updates pendientes como vistas. La próxima
/// pulsación de "Recargar" las trae de vuelta si siguen siendo
/// válidas.
#[tauri::command]
pub async fn dismiss_all_updates(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let updates = updates::compute_available(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    for u in updates {
        if let Err(e) = repo::dismiss_update(&state.db, &u.folder_name).await {
            tracing::warn!("dismiss_all_updates: falló para {}: {e:#}", u.folder_name);
        }
    }
    Ok(())
}

/// Limpia todas las descartadas. Lo invoca el botón "Recargar"
/// del panel de notificaciones para garantizar que siempre se
/// vean las pendientes.
#[tauri::command]
pub async fn clear_dismissed_updates(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::clear_dismissed_updates(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Busca un thumbnail dentro del folder de un paquete instalado.
/// MSFS distribuye `thumbnail.jpg` / `Thumbnail.jpg` dentro de
/// cada `texture.*/` (liveries) o en el SimObjects raíz (aircraft
/// principales). También escenarios de pago a veces incluyen un
/// preview en la raíz. Devolvemos el path absoluto del primer
/// thumbnail encontrado (BFS hasta profundidad 4) — el frontend lo
/// convierte con `convertFileSrc()` para mostrarlo en el card.
#[tauri::command]
pub async fn package_thumbnail(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    use std::path::PathBuf;
    let pkgs = repo::list_community_packages(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let target = pkgs
        .iter()
        .find(|p| p.folder_name == folder_name)
        .ok_or_else(|| format!("paquete desconocido: {}", folder_name))?;
    let root = PathBuf::from(&target.install_path);
    Ok(find_thumbnail(&root, 0))
}

/// Dos pasadas:
///   1. Imágenes con keyword conocida (thumbnail/preview/icon/...).
///   2. Cualquier `.jpg`/`.png`/`.webp` >= 8 KB (skip iconos
///      diminutos que se ven feos como banner).
///
/// Cubre el caso de Aerosoft, Drzewiecki, FSDreamTeam — distribuyen
/// previews con nombres arbitrarios que el filtro estricto rechazaba.
fn find_thumbnail(dir: &std::path::Path, _depth: usize) -> Option<String> {
    if let Some(p) = find_image_named(dir, 0) {
        return Some(p);
    }
    find_any_image(dir, 0)
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "jpg" | "jpeg" | "png" | "webp")
}

fn looks_like_thumbnail(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "thumbnail",
        "preview",
        "icon",
        "marketing",
        "screenshot",
        "splash",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

fn find_image_named(dir: &std::path::Path, depth: usize) -> Option<String> {
    // Depth 6 cubre: <pkg>/SimObjects/Airplanes/<plane>/texture.<code>/thumbnail.jpg
    // (5 niveles bajo el root). PMDG, Fenix y similares ponen el
    // thumbnail ahí. Profundidad 4 anterior los dejaba fuera.
    if depth > 6 || !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_file() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if is_image_ext(&ext) && looks_like_thumbnail(&name_str) {
                return Some(path.to_string_lossy().into_owned());
            }
        } else if ft.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_image_named(&sub, depth + 1) {
            return Some(found);
        }
    }
    None
}

fn find_any_image(dir: &std::path::Path, depth: usize) -> Option<String> {
    // Depth 6 cubre: <pkg>/SimObjects/Airplanes/<plane>/texture.<code>/thumbnail.jpg
    // (5 niveles bajo el root). PMDG, Fenix y similares ponen el
    // thumbnail ahí. Profundidad 4 anterior los dejaba fuera.
    if depth > 6 || !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if is_image_ext(&ext) {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() >= 8 * 1024 {
                        return Some(path.to_string_lossy().into_owned());
                    }
                }
            }
        } else if ft.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_any_image(&sub, depth + 1) {
            return Some(found);
        }
    }
    None
}

async fn resolve_path(community_path: Option<String>) -> Result<PathBuf, String> {
    if let Some(p) = community_path {
        return Ok(PathBuf::from(p));
    }
    let detected: Option<CommunityInfo> =
        community::detect_community_folder().map_err(|e| e.to_string())?;
    let info = detected.ok_or_else(|| {
        "No se detectó la carpeta Community automáticamente — pásala manualmente."
            .to_string()
    })?;
    if !info.exists {
        return Err(format!("La carpeta Community detectada no existe: {}", info.path));
    }
    Ok(PathBuf::from(info.path))
}
