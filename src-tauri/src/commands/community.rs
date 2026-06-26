use std::path::PathBuf;

use crate::community::{self, CommunityInfo};
use crate::community_scanner::{self, ScanReport};
use crate::db::repo::{self, CommunityPackageRow};
use crate::logger::CmdTimer;
use crate::package_ops;
use crate::pmdg_liveries::{self, PmdgLivery};
use crate::updates::{self, AvailableUpdate, RefreshSummary};
use crate::{cmd_log, AppState};

/// (v5.3.4) Versiones de MSFS instaladas en el sistema. El splash lo usa
/// para preguntar la versión SOLO cuando hay 2020 Y 2024 instalados.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSims {
    pub has2020: bool,
    pub has2024: bool,
}

#[tauri::command]
pub async fn get_installed_sims() -> Result<InstalledSims, String> {
    let (has2020, has2024) = community::detect_installed_sims();
    Ok(InstalledSims { has2020, has2024 })
}

/// (v6.1) Lista las liveries de un pack (parsea sus aircraft.cfg). La usa el
/// modal de liverypack en Addons y Link Map.
#[tauri::command]
pub async fn pack_liveries(
    install_path: String,
) -> Result<Vec<community_scanner::PackLivery>, String> {
    Ok(
        tokio::task::spawn_blocking(move || community_scanner::list_pack_liveries(&install_path))
            .await
            .map_err(|e| e.to_string())?,
    )
}

/// (v6) `true` si la app corre en MODO SEGURO (`SIMFLEET_SAFE_MODE`): 2ª
/// instancia de pruebas sin SimConnect ni auto-sync. El frontend muestra un
/// banner para no confundirla con la instancia principal.
#[tauri::command]
pub fn is_safe_mode() -> bool {
    std::env::var("SIMFLEET_SAFE_MODE").is_ok()
}

/// (v6) `true` si el watcher de SimConnect está activo. En MODO SEGURO suele
/// estar apagado, salvo que se fuerce con `SIMFLEET_ENABLE_WATCHER=1` para
/// probar Best Landings — el banner lo refleja para no confundir al usuario.
#[tauri::command]
pub fn is_watcher_active() -> bool {
    std::env::var("SIMFLEET_SAFE_MODE").is_err()
        || std::env::var("SIMFLEET_ENABLE_WATCHER").is_ok()
}

/// (v6 #1) Crea junctions NTFS de los paquetes recién instalados en la
/// Community de la OTRA versión de MSFS (cross-link 2020 ↔ 2024). Lo invoca
/// el modal post-instalación cuando el usuario acepta enlazar.
#[tauri::command]
pub fn cross_link_create(
    other_community: String,
    packages: Vec<crate::cross_link::CrossLinkPkg>,
) -> Result<crate::cross_link::CrossLinkResult, String> {
    cmd_log!(
        "cross_link_create",
        "dest={} n={}",
        other_community,
        packages.len()
    );
    Ok(crate::cross_link::create_junctions(
        &other_community,
        &packages,
    ))
}

/// Escanea Community y persiste los paquetes encontrados. Acepta
/// un path opcional — si no viene, se intenta detección automática.
#[tauri::command]
pub async fn scan_community(
    community_path: Option<String>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<ScanReport, String> {
    cmd_log!("scan_community", "path={:?}", community_path);
    let _t = CmdTimer::start("scan_community");
    let path = resolve_path(community_path).await?;
    tracing::info!(target: "scan", "scan_community: usando path {}", path.display());
    let path_for_task = path.clone();
    let report = tokio::task::spawn_blocking(move || community_scanner::scan(&path_for_task))
        .await
        .map_err(|e| {
            tracing::error!(target: "scan", "scan_community: tarea panic: {e}");
            format!("la tarea de escaneo falló: {e}")
        })?
        .map_err(|e| {
            tracing::error!(target: "scan", "scan_community: scan error: {e:#}");
            e.to_string()
        })?;
    tracing::info!(
        target: "scan",
        "scan_community: {} paquetes, {} skipped sin manifest, {} skipped manifest inválido",
        report.packages.len(),
        report.skipped_no_manifest,
        report.skipped_invalid_manifest
    );
    community_scanner::sync_to_db(&state.db, &report)
        .await
        .map_err(|e| {
            tracing::error!(target: "scan", "scan_community: sync_to_db error: {e:#}");
            e.to_string()
        })?;

    // (v4.26.0) Best-effort en background: bajar de Wikipedia las
    // imágenes de aeropuertos sin thumbnail (o con el placeholder del
    // SDK) y guardarlas en el paquete. No bloquea el scan; al
    // terminar emite thumbs://updated y el frontend refresca.
    {
        let pool = state.db.clone();
        let http = state.http.clone();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            match crate::airport_thumbs::fetch_missing(&pool, &http, &app2).await {
                Ok(n) if n > 0 => {
                    tracing::info!(target: "scan", "thumbs: {} imágenes bajadas de Wikipedia", n);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(target: "scan", "thumbs: fetch_missing falló: {e:#}");
                }
            }
        });
    }
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
    cmd_log!("refresh_updates_for_installed");
    let _t = CmdTimer::start("refresh_updates_for_installed");
    let result = updates::refresh_for_installed(&state.db, &state.sources)
        .await
        .map_err(|e| {
            tracing::error!(target: "updates", "refresh_for_installed error: {e:#}");
            e.to_string()
        })?;
    tracing::info!(
        target: "updates",
        "refresh_for_installed: {} ICAOs/keywords, {} queries run, {} skipped (cache), {} failed, {} addons vistos en {}ms",
        result.icaos_checked,
        result.queries_run,
        result.queries_skipped_cached,
        result.queries_failed,
        result.addons_seen,
        result.elapsed_ms
    );
    Ok(result)
}

/// Desinstala un paquete por nombre de folder.
///
/// Desde v0.1.19 la desinstalación es **total**: borra el folder en
/// CADA Community detectada (Steam 2020, MS Store 2020, Steam 2024,
/// MS Store 2024) y en las carpetas extras
/// (`Documents\MSFS Sceneries\<folder>`,
/// `Documents\MSFS 2024 Sceneries\<folder>`). Devuelve un
/// `UninstallReport` con los paths borrados + los que fallaron para
/// que el frontend pueda enseñar al usuario qué se hizo y dónde
/// queda residuo que requiera intervención manual (típicamente, MSFS
/// está corriendo y bloquea el handle del directorio).
#[tauri::command]
pub async fn uninstall_community_package(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<package_ops::UninstallReport, String> {
    cmd_log!("uninstall_community_package", "folder={}", folder_name);
    let _t = CmdTimer::start("uninstall_community_package");
    match package_ops::uninstall_by_folder(&state.db, &folder_name).await {
        Ok(report) => {
            tracing::info!(
                target: "install",
                "uninstall '{}' OK — {} paths borrados, {} fallaron, {} filas DB",
                folder_name,
                report.removed_paths.len(),
                report.failed_paths.len(),
                report.db_rows_cleared
            );
            Ok(report)
        }
        Err(e) => {
            tracing::error!(
                target: "install",
                "uninstall '{}' falló: {e:#}",
                folder_name
            );
            Err(e.to_string())
        }
    }
}

/// (v4.25.0) Enciende/apaga un paquete en el sim renombrando su
/// `layout.json` ⇄ `layout.json.disabled`. Con `cascade=true` propaga
/// el estado a todos los addons enlazados en el Link Map (transitivo,
/// asimétrico — solo hacia los dependientes — y cycle-safe vía BFS
/// con set de visitados).
#[tauri::command]
pub async fn set_package_enabled(
    folder_name: String,
    enabled: bool,
    cascade: bool,
    state: tauri::State<'_, AppState>,
) -> Result<package_ops::ToggleReport, String> {
    cmd_log!(
        "set_package_enabled",
        "folder={} enabled={} cascade={}",
        folder_name,
        enabled,
        cascade
    );
    package_ops::set_package_enabled(&state.db, &folder_name, enabled, cascade)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Toggle masivo — Enable All / Disable All del dashboard de
/// Addons. El frontend manda la lista explícita de folders en scope.
#[tauri::command]
pub async fn set_packages_enabled(
    folder_names: Vec<String>,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<package_ops::ToggleReport, String> {
    cmd_log!(
        "set_packages_enabled",
        "n={} enabled={}",
        folder_names.len(),
        enabled
    );
    let _t = CmdTimer::start("set_packages_enabled");
    package_ops::set_packages_enabled(&state.db, &folder_names, enabled)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Aristas del Link Map ('auto' del manifest + 'manual').
#[tauri::command]
pub async fn list_addon_links(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::AddonLinkRow>, String> {
    repo::list_addon_links(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Crea una arista manual source → target en el Link Map.
#[tauri::command]
pub async fn add_addon_link(
    source_folder: String,
    target_folder: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if source_folder.eq_ignore_ascii_case(&target_folder) {
        return Err("un addon no puede enlazarse consigo mismo".to_string());
    }
    cmd_log!("add_addon_link", "{} -> {}", source_folder, target_folder);
    repo::add_addon_link(&state.db, &source_folder, &target_folder)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Elimina una arista del Link Map (auto o manual).
#[tauri::command]
pub async fn remove_addon_link(
    source_folder: String,
    target_folder: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    cmd_log!("remove_addon_link", "{} -> {}", source_folder, target_folder);
    repo::remove_addon_link(&state.db, &source_folder, &target_folder)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Posiciones persistidas de los nodos del Link Map.
#[tauri::command]
pub async fn list_addon_node_positions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::AddonNodePosition>, String> {
    repo::list_addon_node_positions(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.25.0) Guarda posiciones de nodos tras un drag en el Link Map.
#[tauri::command]
pub async fn save_addon_node_positions(
    positions: Vec<repo::AddonNodePosition>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::save_addon_node_positions(&state.db, &positions)
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

/// Busca un thumbnail dentro del folder de un paquete instalado y
/// lo devuelve como **data URL en base64**.
///
/// Por qué base64 en vez de path: en Tauri 2 el asset protocol
/// requiere que cada path esté dentro de un `scope` declarado en
/// `tauri.conf.json`. La carpeta Community vive en cualquier disco
/// del usuario, así que un scope estático no la cubre. Antes
/// devolvíamos un path y `convertFileSrc()` lo formaba en una URL
/// `https://asset.localhost/...` — pero la mayoría de paths quedaba
/// fuera del scope y MSEdge silenciaba el load. Resultado: cards
/// sin imagen aunque el archivo existía.
///
/// Solución definitiva: leer el archivo en Rust y devolver
/// `data:image/<ext>;base64,...`. La imagen viaja por el canal IPC
/// (mismo que cualquier otro comando) y el `<img>` la muestra
/// directamente — sin scope, sin protocolo custom, sin sorpresas.
///
/// Coste: 33% extra por base64 vs binario crudo. Para thumbnails
/// típicos (50-300 KB) son 70-400 KB de string — barato.
///
/// `find_thumbnail` hace BFS hasta profundidad 6 buscando primero
/// nombres conocidos (thumbnail/preview/icon) y luego cualquier
/// imagen >= 8 KB.
#[tauri::command]
pub async fn package_thumbnail(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    // (v4.29.0) Lookup directo a `thumbnail_path` (resuelto durante el
    // scan). O(1) — sin recursión I/O en runtime. Si la ruta cacheada
    // ya no existe (paquete actualizado fuera de SimFleet), caemos a
    // un find_thumbnail puntual y bound (depth 8) como red de
    // seguridad — no pasa cada render porque la mayoría de paquetes
    // sí tienen el cache poblado.
    let row = sqlx::query_as::<_, (Option<String>, String)>(
        "SELECT thumbnail_path, install_path FROM community_packages WHERE folder_name = ?1",
    )
    .bind(&folder_name)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some((cached, install_path)) = row else {
        return Err(format!("paquete desconocido: {}", folder_name));
    };
    let folder_clone = folder_name.clone();
    let result = tokio::task::spawn_blocking(move || -> Option<String> {
        if let Some(path) = cached.as_deref() {
            let p = std::path::Path::new(path);
            if p.is_file() {
                return encode_image_as_data_url(p).ok();
            }
        }
        // Fallback puntual (paquete actualizado fuera del scan).
        let path_str = find_thumbnail(std::path::Path::new(&install_path), 0)?;
        encode_image_as_data_url(std::path::Path::new(&path_str))
            .inspect_err(|e| {
                tracing::debug!("thumbnail encode falló para {folder_clone}: {e:#}");
            })
            .ok()
    })
    .await
    .map_err(|e| format!("thumbnail task join error: {e}"))?;
    Ok(result)
}

/// Lee un archivo de imagen y devuelve `data:image/<mime>;base64,...`.
/// Devuelve error si el archivo no existe o no se puede leer; ese
/// error lo deglutimos en el caller (mejor mostrar el card sin
/// imagen que tirar el render por una imagen rota).
fn encode_image_as_data_url(path: &std::path::Path) -> anyhow::Result<String> {
    use base64::Engine;
    let bytes = std::fs::read(path)?;
    // Limit defensivo — un thumbnail de 5 MB ralentizaría el render
    // sin aportar nada visualmente. Si supera, lo descartamos y el
    // card cae al placeholder.
    if bytes.len() > 5 * 1024 * 1024 {
        anyhow::bail!("thumbnail demasiado grande ({} bytes)", bytes.len());
    }
    let mime = match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        // jpg / jpeg / desconocido → asumimos jpeg (los thumbnails de
        // MSFS son casi siempre JPEG).
        _ => "image/jpeg",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Tres pasadas:
///   0. (v4.26.0) `simfleet-thumbnail.{jpg,png}` en la raíz — la
///      imagen que NOSOTROS bajamos de Wikipedia (airport_thumbs).
///      Gana siempre: es la cura para los packs con arte
///      "PLACEHOLDER" del SDK.
///   1. Imágenes con keyword conocida (thumbnail/preview/icon/...).
///   2. Cualquier `.jpg`/`.png`/`.webp` >= 8 KB (skip iconos
///      diminutos que se ven feos como banner).
///
/// Cubre el caso de Aerosoft, Drzewiecki, FSDreamTeam — distribuyen
/// previews con nombres arbitrarios que el filtro estricto rechazaba.
/// `pub(crate)` porque airport_thumbs lo usa para decidir si un
/// paquete necesita imagen.
pub(crate) fn find_thumbnail(dir: &std::path::Path, _depth: usize) -> Option<String> {
    for ext in ["jpg", "jpeg", "png", "webp"] {
        let custom = dir.join(format!("simfleet-thumbnail.{ext}"));
        if custom.is_file() {
            return Some(custom.to_string_lossy().into_owned());
        }
    }
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
    // (v4.26.0) Los archivos llamados *placeholder* son el arte gris
    // del SDK — mostrarlos es peor que no mostrar nada (el card cae
    // al arte tipado del frontend o a la foto de Wikipedia).
    if lower.contains("placeholder") {
        return false;
    }
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
    if depth > 8 || !dir.is_dir() {
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
    if depth > 8 || !dir.is_dir() {
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
            // (v4.26.0) Mismo veto que en looks_like_thumbnail: el
            // arte *placeholder* del SDK no cuenta como imagen.
            let is_placeholder = path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.to_lowercase().contains("placeholder"))
                .unwrap_or(false);
            if is_image_ext(&ext) && !is_placeholder {
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

/// (v3.0.0) Escanea liveries de PMDG en la carpeta Community.
/// Detecta paquetes con prefijo `pmdg-aircraft-*-liveries`, parsea
/// el `aircraft.cfg` de cada uno y devuelve la lista de variantes
/// con tail number, airline, texture y thumbnail (si existe).
///
/// Es un comando independiente del scan general porque PMDG no
/// expone metadatos en `manifest.json` — el detalle vive en el
/// aircraft.cfg dentro de SimObjects/Airplanes/.
#[tauri::command]
pub async fn list_pmdg_liveries(
    community_path: Option<String>,
) -> Result<Vec<PmdgLivery>, String> {
    cmd_log!("list_pmdg_liveries", "path={:?}", community_path);
    let _t = CmdTimer::start("list_pmdg_liveries");
    let path = resolve_path(community_path).await?;
    let path_for_task = path.clone();
    let liveries = tokio::task::spawn_blocking(move || {
        pmdg_liveries::scan_pmdg_liveries(&path_for_task)
    })
    .await
    .map_err(|e| format!("la tarea de escaneo PMDG falló: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(liveries)
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
