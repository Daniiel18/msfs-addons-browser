pub mod airports;
pub mod commands;
pub mod community;
pub mod community_scanner;
pub mod db;
pub mod download;
pub mod flight_log;
pub mod gsx;
pub mod install;
pub mod logger;
pub mod package_ops;
pub mod parser;
pub mod simbrief;
pub mod simconnect_ffi;
pub mod simconnect_watcher;
pub mod sources;
pub mod updater;
pub mod updates;

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
}

impl AppState {
    pub fn source(&self, id: &str) -> Option<Arc<dyn Source>> {
        self.sources.iter().find(|s| s.id() == id).cloned()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            commands::install::diagnose_pmdg_oc,
            commands::gsx::gsx_lookup,
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
            commands::changelog::fetch_changelog,
            commands::simbrief::get_simbrief_pilot_id,
            commands::simbrief::set_simbrief_pilot_id,
            commands::simbrief::refresh_simbrief,
            commands::simbrief::list_simbrief_flights,
            commands::simbrief::delete_simbrief_flight,
            commands::stats::get_dashboard_stats,
            commands::flight_log::list_flight_log,
            commands::flight_log::delete_flight_log_entry,
            commands::flight_log::debug_seed_flight_log,
            commands::flight_log::get_flight_status,
            commands::settings::get_app_settings,
            commands::settings::set_app_setting,
            commands::settings::set_autostart,
            commands::settings::clear_caches,
            commands::settings::reset_settings,
            commands::backup::backup_community,
            commands::backup::export_addons,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}

async fn init_state(app: &tauri::AppHandle) -> anyhow::Result<AppState> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;

    logger::init(&app_data_dir)?;
    tracing::info!("app starting; data dir = {}", app_data_dir.display());

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
        .user_agent("MSFSAddonsBrowser/0.1 (+https://github.com/n0xful)")
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
    })
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
        .tooltip("MSFS Addons Browser")
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
