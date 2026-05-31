pub mod airports;
pub mod cloud_sync;
pub mod commands;
pub mod community;
pub mod community_scanner;
pub mod db;
pub mod drop_install;
pub mod download;
pub mod flight_log;
pub mod gsx;
pub mod gsx_parking;
pub mod install;
pub mod logger;
pub mod msfs_logbook;
pub mod package_ops;
pub mod parser;
pub mod pmdg_liveries;
pub mod scoring;
pub mod simbrief;
pub mod simconnect_ffi;
pub mod simconnect_watcher;
pub mod sources;
pub mod updater;
pub mod updates;
pub mod vas_acars;
pub mod vas_summary;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use download::manager::DownloadManager;
use gsx::GsxClient;
use sources::Source;
use tauri::Manager;

pub struct AppState {
    pub sources: Vec<Arc<dyn Source>>,
    pub db: sqlx::SqlitePool,
    pub downloads: DownloadManager,
    /// Cliente cacheado para consultas a flightsim.to (perfiles GSX Pro).
    /// Comparte el mismo `reqwest::Client` que las fuentes principales.
    pub gsx: GsxClient,
    /// HTTP client compartido — lo usa el comando de updater para
    /// hablar con la API de GitHub. Se instancia una sola vez al
    /// arrancar para reutilizar el connection pool.
    pub http: reqwest::Client,
    /// Flag compartido entre el setter de `pref_minimize_to_tray` y
    /// el handler de cierre de ventana. Cuando es `true`, cerrar la
    /// ventana la oculta en lugar de salir.
    pub minimize_to_tray: Arc<AtomicBool>,
    /// (v2.1.0) Sesiones de drag-and-drop en curso. Cada entrada es
    /// un tempdir + lista de items inspeccionados, esperando el
    /// commit del modal de selección.
    pub drop_sessions: drop_install::DropSessions,
}

impl AppState {
    pub fn source(&self, id: &str) -> Option<Arc<dyn Source>> {
        self.sources.iter().find(|s| s.id() == id).cloned()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // (v2.2.0) Single instance — si la app ya está corriendo,
        // abrir el .exe de nuevo le manda los args a la instancia
        // existente y trae su ventana al frente. NO arrancar un
        // segundo proceso.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        // Diálogo nativo de selección de archivos — sostiene el flujo
        // «Instalar desde archivo…» del header. El plugin se registra
        // antes que `setup` porque la inicialización del state no lo
        // necesita, pero los comandos sí (resolved at runtime).
        .plugin(tauri_plugin_dialog::init())
        // Autostart con Windows. Pasamos `vec![]` como args extra —
        // el ejecutable arrancará con sus defaults. El plugin registra
        // / borra `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`
        // según `manager.enable()` / `manager.disable()`.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle_for_init = app.handle().clone();
            let handle_for_tray = app.handle().clone();
            let state = tauri::async_runtime::block_on(async move {
                init_state(&handle_for_init).await
            })?;
            // Tray icon — menú con "Mostrar" y "Salir". El flag
            // `minimize_to_tray` decide si el cierre de la ventana
            // oculta o cierra; el icono de bandeja sigue funcionando
            // como interruptor para mostrar/ocultar.
            init_tray(&handle_for_tray)?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Si está activado «minimizar a la bandeja», ocultamos
            // la ventana en lugar de cerrar la app. El usuario la
            // recupera desde el tray. Sin el flag activado, el cierre
            // funciona normal (sale del proceso).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    if state.minimize_to_tray.load(Ordering::Relaxed) {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search::search,
            commands::search::list_sources,
            commands::search::browse_source,
            commands::addons::list_installed,
            commands::downloads::list_downloads,
            commands::downloads::start_download,
            commands::downloads::cancel_download,
            commands::downloads::pause_download,
            commands::downloads::resume_download,
            commands::downloads::clear_download,
            commands::install::community_folder,
            commands::install::install_archive,
            commands::install::forget_install,
            commands::gsx::gsx_lookup,
            commands::gsx::gsx_list_installed_icaos,
            commands::gsx::gsx_install_profile,
            commands::gsx::read_text_file,
            commands::updater::check_for_update,
            commands::updater::install_update,
            commands::airports::list_addons_on_map,
            commands::airports::refresh_airports_dataset,
            commands::community::scan_community,
            commands::community::list_community_packages,
            commands::community::list_available_updates,
            commands::community::refresh_updates_for_installed,
            commands::community::uninstall_community_package,
            commands::community::diagnose_update_for_package,
            commands::community::dismiss_update,
            commands::community::dismiss_all_updates,
            commands::community::clear_dismissed_updates,
            commands::community::package_thumbnail,
            commands::community::list_pmdg_liveries,
            commands::changelog::fetch_changelog,
            commands::simbrief::get_simbrief_pilot_id,
            commands::simbrief::set_simbrief_pilot_id,
            commands::simbrief::refresh_simbrief,
            commands::simbrief::list_simbrief_flights,
            commands::simbrief::delete_simbrief_flight,
            commands::stats::get_dashboard_stats,
            commands::flight_log::list_flight_log,
            commands::flight_log::delete_flight_log_entry,
            commands::flight_log::force_close_flight_log_entry,
            commands::flight_log::debug_seed_flight_log,
            commands::flight_log::get_flight_status,
            commands::flight_log::get_flight_track,
            commands::flight_log::update_flight_log_entry,
            commands::flight_log::msfs_logbook_detect,
            commands::flight_log::msfs_logbook_import,
            commands::flight_log::vas_acars_detect,
            commands::flight_log::vas_acars_import,
            commands::flight_log::delete_flights_by_source,
            commands::settings::get_app_settings,
            commands::settings::set_app_setting,
            commands::settings::set_autostart,
            commands::settings::clear_caches,
            commands::settings::reset_settings,
            commands::backup::backup_community,
            commands::backup::export_addons,
            commands::cloud::cloud_get_config,
            commands::cloud::cloud_set_credentials,
            commands::cloud::cloud_start_oauth,
            commands::cloud::cloud_disconnect,
            commands::cloud::cloud_sync_now,
            commands::cloud::cloud_upload_all,
            commands::cloud::cloud_download_missing,
            commands::cloud::cloud_test_connection,
            commands::cloud::cloud_purge,
            commands::cloud::folder_sync_get_config,
            commands::cloud::folder_sync_save,
            commands::cloud::folder_sync_load,
            commands::cloud::folder_sync_clear,
            commands::drop::drop_inspect,
            commands::drop::drop_commit,
            commands::drop::drop_cancel,
            // (v3.6.0 Phase H) Scoring + Airlines
            commands::scoring::score_flight,
            commands::scoring::score_get_report,
            commands::scoring::list_airlines,
            commands::scoring::airline_kpis,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}

/// (v3.2.0) Resuelve el data dir PORTABLE. Estrategia:
///   1. Carpeta `data/` al lado del ejecutable (`exe_parent/data/`).
///      Si es escribible la usamos — toda la app vive dentro de la
///      carpeta de instalación (Program Files/SimFleet o donde sea
///      que el usuario instaló).
///   2. Si el dir del ejecutable NO es escribible (Program Files sin
///      admin), caemos a `%LOCALAPPDATA%\SimFleet\data` como fallback
///      — sigue siendo "user-scoped" y no contamina Roaming/Temp.
///
/// Antes (v3.1.x y previo) usábamos `%APPDATA%\org.n0xful.msfsaddonsbrowser`
/// que el usuario reportó como invasivo. La nueva arquitectura
/// portable migra esos datos automáticamente al primer arranque.
fn resolve_portable_data_dir() -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent()
        .ok_or_else(|| anyhow::anyhow!("current_exe sin parent"))?
        .to_path_buf();
    let candidate = exe_dir.join("data");
    // Test de escritura: intentamos crear el dir + un archivo temporal.
    if std::fs::create_dir_all(&candidate).is_ok() {
        let probe = candidate.join(".write-probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Ok(candidate);
        }
    }
    // Fallback: %LOCALAPPDATA%\SimFleet\data
    let local = std::env::var("LOCALAPPDATA")
        .map_err(|_| anyhow::anyhow!("LOCALAPPDATA no disponible"))?;
    let fallback = std::path::PathBuf::from(local)
        .join("SimFleet")
        .join("data");
    std::fs::create_dir_all(&fallback)?;
    Ok(fallback)
}

/// (v3.2.0) Migra datos del `%APPDATA%\org.n0xful.msfsaddonsbrowser`
/// viejo al data dir portable nuevo. Sólo corre una vez — al detectar
/// que el nuevo está vacío (sin DB) Y el viejo existe con datos.
/// Best-effort: cualquier fallo se loguea y la app sigue.
fn migrate_legacy_appdata(new_dir: &Path) -> anyhow::Result<()> {
    let new_db = new_dir.join("msfs-addons.db");
    if new_db.exists() {
        // Ya tenemos datos en la nueva ubicación — no sobrescribir.
        return Ok(());
    }
    let appdata = std::env::var("APPDATA").map_err(|_| anyhow::anyhow!("APPDATA?"))?;
    let legacy = std::path::PathBuf::from(appdata)
        .join("org.n0xful.msfsaddonsbrowser");
    let legacy_db = legacy.join("msfs-addons.db");
    if !legacy_db.exists() {
        return Ok(()); // nada que migrar
    }
    eprintln!(
        "[migration] copiando datos viejos desde {} → {}",
        legacy.display(), new_dir.display()
    );
    copy_dir_recursive(&legacy, new_dir)?;
    eprintln!("[migration] OK. El directorio viejo se conserva — puedes borrarlo a mano si quieres.");
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let src_p = entry.path();
        let dst_p = dst.join(entry.file_name());
        if kind.is_dir() {
            copy_dir_recursive(&src_p, &dst_p)?;
        } else if kind.is_file() {
            // Si ya existe en destino (raro), saltamos para no
            // pisotear cambios nuevos.
            if !dst_p.exists() {
                let _ = std::fs::copy(&src_p, &dst_p);
            }
        }
    }
    Ok(())
}

async fn init_state(app: &tauri::AppHandle) -> anyhow::Result<AppState> {
    // (v3.2.0) Data dir PORTABLE al lado del ejecutable. Todo lo que
    // genere la app (DB, logs, OFPs, descargas, temp del drag-drop)
    // vive aquí dentro. Si existe la instalación vieja en
    // %APPDATA%\org.n0xful.msfsaddonsbrowser, migramos antes.
    let app_data_dir = match resolve_portable_data_dir() {
        Ok(p) => p,
        Err(e) => {
            // Si todo falla, último fallback: app_data_dir del SO
            // para que la app al menos arranque (no es ideal pero
            // mejor que crashear).
            eprintln!("[init] resolve_portable_data_dir falló: {e:#}; usando app_data_dir del SO");
            app.path().app_data_dir()?
        }
    };
    std::fs::create_dir_all(&app_data_dir)?;
    if let Err(e) = migrate_legacy_appdata(&app_data_dir) {
        eprintln!("[init] migrate_legacy_appdata falló (no fatal): {e:#}");
    }

    // (v3.2.0) Redirigir el TEMP del proceso al data dir portable.
    // Cubre TODOS los `tempfile::tempdir()` (drag-drop inspect,
    // extracción de .zip/.rar/.7z, GSX install). El usuario pidió
    // explícitamente que los temp del drag-drop NO contaminen
    // `%TEMP%` del sistema operativo.
    {
        let temp_dir = app_data_dir.join("temp");
        let _ = std::fs::create_dir_all(&temp_dir);
        std::env::set_var("TMP", &temp_dir);
        std::env::set_var("TEMP", &temp_dir);
    }

    logger::init(&app_data_dir)?;
    tracing::info!("app starting; data dir = {} (portable)", app_data_dir.display());

    // (v3.0.0) Cleanup de instalación vieja "MSFS Addons Browser" tras
    // el rebrand a SimFleet. Best-effort: si la app actual está corriendo
    // desde `%LOCALAPPDATA%\Programs\SimFleet\…` (o equivalente per-machine)
    // pero todavía existe la carpeta vieja `%LOCALAPPDATA%\Programs\MSFS Addons Browser\`,
    // la borramos. Esto resuelve el "limpia todo rastro de carpetas e
    // instancias viejas del sistema" del usuario sin romper los datos
    // (que viven en AppData\Roaming\org.n0xful.msfsaddonsbrowser\,
    // intencionalmente conservado para retrocompat).
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let old_install =
                std::path::PathBuf::from(&local).join("Programs").join("MSFS Addons Browser");
            // Sólo intentamos borrar si no es donde corre el exe actual
            // (defensa contra borrar nuestra propia instalación).
            let current_exe = std::env::current_exe().ok();
            let current_dir = current_exe.as_ref().and_then(|p| p.parent());
            let safe_to_delete = match current_dir {
                Some(d) => d != old_install.as_path(),
                None => true,
            };
            if safe_to_delete && old_install.is_dir() {
                tracing::info!(
                    "rebrand cleanup: borrando instalación vieja en {}",
                    old_install.display()
                );
                match std::fs::remove_dir_all(&old_install) {
                    Ok(()) => tracing::info!("rebrand cleanup: OK"),
                    Err(e) => tracing::warn!(
                        "rebrand cleanup: falló borrar {}: {} — el usuario puede limpiar manualmente",
                        old_install.display(),
                        e
                    ),
                }
            }
        }
    }

    // Log what we know about the Community folder up-front — the UI
    // will re-query this, but having it in the log helps troubleshoot
    // install failures reported by users without us seeing their screen.
    match community::detect_community_folder() {
        Ok(Some(info)) => tracing::info!(
            "community folder detected ({}): {} (exists={})",
            info.variant, info.path, info.exists
        ),
        Ok(None) => tracing::warn!("community folder not detected automatically"),
        Err(e) => tracing::warn!("community folder detection failed: {}", e),
    }

    let http = reqwest::Client::builder()
        .user_agent("SimFleet/3.0 (+https://github.com/n0xful)")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let sources: Vec<Arc<dyn Source>> = vec![
        Arc::new(sources::sceneryaddons::SceneryAddonsSource::new(http.clone())),
        Arc::new(sources::simplaza::SimplazaSource::new(http.clone())),
    ];

    let db = db::init(&app_data_dir).await?;

    // Torrent output dir sits under app data so it's easy to clean and
    // survives app restarts only as long as the job itself is active.
    // Each job gets a `job-{uuid}` subfolder under here, wiped after
    // install regardless of outcome.
    let torrent_data_dir = app_data_dir.join("torrents");
    // El manager necesita el pool para registrar instalaciones en
    // `installed_addons` cuando termina un torrent — clonamos el handle
    // (es barato, internamente es un Arc).
    let downloads = DownloadManager::new(app.clone(), torrent_data_dir, db.clone());
    // Resume de descargas tras un cierre/crash de la app. Carga los
    // jobs persistidos en `download_jobs` y re-lanza los torrents
    // activos — librqbit detecta los ficheros parciales en
    // `<torrent_data_dir>/job-{id}/` y continúa desde donde se quedó.
    // Best-effort: si la DB está corrupta o vacía no rompemos el
    // arranque de la app, sólo logueamos.
    match downloads.restore_persisted().await {
        Ok(n) if n > 0 => tracing::info!("downloads: {n} job(s) restaurado(s) tras restart"),
        Ok(_) => tracing::debug!("downloads: sin jobs persistidos para restaurar"),
        Err(e) => tracing::warn!("downloads: restore_persisted falló: {e:#}"),
    }

    // (v1.1.1) Cleanup de "vuelos fantasma" — los creaba el watcher
    // en versiones viejas cuando MSFS reportaba el avión en world
    // origin (0, 0) durante el splash/menú. Best-effort.
    match flight_log::delete_junk_flights(&db).await {
        Ok(n) if n > 0 => tracing::info!(
            "flight_log: limpiados {n} vuelo(s) fantasma con origen no válido"
        ),
        Ok(_) => tracing::debug!("flight_log: sin vuelos fantasma para limpiar"),
        Err(e) => tracing::warn!("flight_log: delete_junk_flights falló: {e:#}"),
    }
    // (v2.0.3) Cierre automático de vuelos huérfanos — la app o el sim
    // se cerraron mid-flight y nunca se marcó el IN event. Si el último
    // tick de posición tiene más de 24 h, los cerramos con
    // `ended_at = last_position_at` (o `started_at` si nunca hubo
    // tick) para que no queden en estado "En vuelo ahora" eternamente.
    // El watcher seguirá restaurando los recientes (<24h) al conectar
    // SimConnect — el usuario que pausó la noche entera no pierde
    // nada.
    const STALE_FLIGHT_SECONDS: i64 = 24 * 60 * 60;
    match flight_log::close_stale_open_flights(&db, STALE_FLIGHT_SECONDS).await {
        Ok(n) if n > 0 => tracing::info!(
            "flight_log: auto-cerrados {n} vuelo(s) abandonados (>24h sin update de posición)"
        ),
        Ok(_) => tracing::debug!("flight_log: sin vuelos abandonados que cerrar"),
        Err(e) => tracing::warn!("flight_log: close_stale_open_flights falló: {e:#}"),
    }
    // Cliente GSX comparte el `reqwest::Client` con las fuentes — mismo
    // pool de conexiones, mismo timeout, misma resolución DNS.
    let gsx = GsxClient::new(http.clone());

    // Disparamos la sincronización del dataset de aeropuertos en
    // background. Estrategia "offline-first":
    //   1. Intenta cargar el CSV bundleado en `resources/` (incluido
    //      en el setup.exe — la app funciona sin internet).
    //   2. Si no hay recurso o la lectura falla, descarga del
    //      mirror público de OurAirports.
    // De todas formas no bloqueamos el splash — son varios segundos
    // de parsing/insert que el usuario no necesita ver.
    {
        let bg_pool = db.clone();
        let bg_http = http.clone();
        let bg_app = app.clone();
        tokio::spawn(async move {
            if let Err(e) =
                airports::ensure_dataset_with_app(&bg_pool, &bg_http, Some(&bg_app)).await
            {
                tracing::warn!("airports: background sync failed: {e:#}");
            }
        });
    }

    // El scan de Community + refresh de updates se mueve al
    // bootstrap del frontend para que la splash screen los muestre
    // como pasos visibles. Antes hacíamos un spawn aquí que duplicaba
    // trabajo con `useCommunityStore.bootstrap` y dejaba al usuario
    // esperando sin feedback.

    // Watcher de "vuelo en curso" — detecta el proceso MSFS y lo
    // cruza con la última OFP fresca de SimBrief para responder a
    // "¿qué estoy volando ahora?". Emite `flight://current` al
    // frontend cuando cambia el estado. Lo añadimos al state de
    // Tauri para que el comando `get_flight_status` pueda leerlo.
    let watcher_state = simconnect_watcher::spawn(db.clone(), app.clone());
    app.manage(watcher_state);

    // (v3.1.0) Auto-sync con la nube — silencioso, una vez al día.
    // Sólo dispara si:
    //   · Hay refresh_token guardado (usuario ya autorizó OAuth).
    //   · Han pasado >24h desde el último sync (settings.google_last_sync_at).
    // En cualquier caso es best-effort — si falla la conexión, no
    // molestamos al usuario, sólo logueamos.
    {
        let bg_pool = db.clone();
        let bg_http = http.clone();
        tokio::spawn(async move {
            // Pequeño delay para no competir con el splash/scan al
            // arrancar — 15s da tiempo al usuario a ver el dashboard.
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            match should_auto_sync(&bg_pool).await {
                Ok(true) => {
                    tracing::info!(target: "cloud", "auto-sync diario: arrancando");
                    match cloud_sync::sync_now(&bg_pool, &bg_http).await {
                        Ok(report) => tracing::info!(
                            target: "cloud",
                            "auto-sync OK: ↑{}↓{} vuelos · ↑{}↓{} tracks · ↑{}↓{} settings",
                            report.uploaded_flights, report.downloaded_flights,
                            report.uploaded_tracks, report.downloaded_tracks,
                            report.uploaded_settings, report.downloaded_settings,
                        ),
                        Err(e) => tracing::warn!(
                            target: "cloud",
                            "auto-sync falló (best-effort, app sigue): {e:#}"
                        ),
                    }
                }
                Ok(false) => {
                    tracing::debug!(
                        target: "cloud",
                        "auto-sync: skip (último <24h o sin token)"
                    );
                }
                Err(e) => tracing::debug!(
                    target: "cloud",
                    "auto-sync should-check falló: {e:#}"
                ),
            }
        });
    }

    // Lee el setting de minimize-to-tray para inicializar el flag
    // atómico que el handler de cierre de ventana consulta.
    let minimize_to_tray = Arc::new(AtomicBool::new(false));
    {
        let pool = db.clone();
        let flag = minimize_to_tray.clone();
        // No bloqueamos init_state esperando esto — si la lectura
        // falla, queda en false (default) hasta que el usuario lo
        // active desde el panel de settings.
        tokio::spawn(async move {
            if let Ok(Some((value,))) = sqlx::query_as::<_, (String,)>(
                "SELECT value FROM settings WHERE key = 'pref_minimize_to_tray'",
            )
            .fetch_optional(&pool)
            .await
            {
                let on = matches!(value.as_str(), "1" | "true" | "yes");
                flag.store(on, Ordering::Relaxed);
            }
        });
    }

    Ok(AppState {
        sources,
        db,
        downloads,
        gsx,
        http,
        minimize_to_tray,
        drop_sessions: drop_install::DropSessions::default(),
    })
}

/// (v3.1.0) Decide si toca disparar el auto-sync diario:
///   · Hay refresh_token (usuario autorizó OAuth).
///   · No hay last_sync_at, o han pasado >=24h desde el último.
async fn should_auto_sync(pool: &sqlx::SqlitePool) -> anyhow::Result<bool> {
    let refresh: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'google_refresh_token'")
            .fetch_optional(pool)
            .await?;
    if refresh.as_ref().map(|r| r.0.is_empty()).unwrap_or(true) {
        return Ok(false);
    }
    let last: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'google_last_sync_at'")
            .fetch_optional(pool)
            .await?;
    let last_at = match last {
        Some((s,)) if !s.is_empty() => s,
        _ => return Ok(true), // nunca sincronizado
    };
    // Formato guardado en sync_now: "%Y-%m-%dT%H:%M:%SZ"
    let parsed = chrono::DateTime::parse_from_rfc3339(&last_at)
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&last_at, "%Y-%m-%dT%H:%M:%SZ")
            .map(|n| n.and_utc().into()));
    let last_dt = match parsed {
        Ok(d) => d.with_timezone(&chrono::Utc),
        Err(_) => return Ok(true), // parse falló → mejor sincronizar
    };
    let elapsed = chrono::Utc::now()
        .signed_duration_since(last_dt)
        .num_hours();
    Ok(elapsed >= 24)
}

/// Construye el icono de bandeja con menú "Mostrar / Salir". En el
/// click izquierdo, alterna la visibilidad de la ventana principal —
/// patrón estándar de apps Windows que usan tray.
fn init_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};

    let show_item = MenuItem::with_id(app, "tray_show", "Mostrar / Ocultar", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "tray_quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let toggle_window = |handle: &tauri::AppHandle| {
        if let Some(window) = handle.get_webview_window("main") {
            match window.is_visible() {
                Ok(true) => {
                    let _ = window.hide();
                }
                _ => {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.unminimize();
                }
            }
        }
    };

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("SimFleet")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "tray_show" => toggle_window(app),
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
