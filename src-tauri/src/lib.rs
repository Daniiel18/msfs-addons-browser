pub mod airports;
pub mod commands;
pub mod community;
pub mod community_scanner;
pub mod db;
pub mod download;
pub mod gsx;
pub mod install;
pub mod logger;
pub mod package_ops;
pub mod parser;
pub mod simbrief;
pub mod sources;
pub mod updater;
pub mod updates;

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
        .setup(|app| {
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(async move { init_state(&handle).await })?;
            app.manage(state);
            Ok(())
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
            commands::updater::check_for_update,
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
    // background — son ~7MB que no queremos bloquear el splash. Si
    // está cacheado y dentro del TTL, esto sale en milisegundos.
    {
        let bg_pool = db.clone();
        let bg_http = http.clone();
        tokio::spawn(async move {
            if let Err(e) = airports::ensure_dataset(&bg_pool, &bg_http).await {
                tracing::warn!("airports: background sync failed: {e:#}");
            }
        });
    }

    // Scan de Community en background. Pipeline en cadena:
    //   1. Scan + persist en `community_packages`.
    //   2. Refresh activo de updates contra cada fuente — pone en
    //      cache las versiones del catálogo para los ICAOs instalados.
    // Resultado: al abrir la app por primera vez en una sesión, las
    // notificaciones aparecen pobladas sin acción del usuario.
    {
        let bg_pool = db.clone();
        let bg_sources = sources.clone();
        tokio::spawn(async move {
            let community_path = match community::detect_community_folder() {
                Ok(Some(info)) if info.exists => Some(std::path::PathBuf::from(info.path)),
                _ => None,
            };
            let Some(path) = community_path else {
                tracing::info!("community: scan saltado (no detectado)");
                return;
            };
            let scan_result =
                tokio::task::spawn_blocking(move || community_scanner::scan(&path)).await;
            let report = match scan_result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    tracing::warn!("community: scan falló: {e:#}");
                    return;
                }
                Err(e) => {
                    tracing::warn!("community: scan task join error: {e}");
                    return;
                }
            };
            tracing::info!(
                "community: {} paquetes vistos en {}",
                report.packages.len(),
                report.community_path
            );
            if let Err(e) = community_scanner::sync_to_db(&bg_pool, &report).await {
                tracing::warn!("community: persist falló: {e:#}");
                return;
            }
            // Una vez tenemos el inventario, vamos a buscar updates
            // contra las fuentes. Esto puede tardar (250ms por
            // query × N ICAOs × M fuentes), por eso es background.
            if let Err(e) = updates::refresh_for_installed(&bg_pool, &bg_sources).await {
                tracing::warn!("community: refresh de updates falló: {e:#}");
            }
        });
    }

    Ok(AppState {
        sources,
        db,
        downloads,
        gsx,
        http,
    })
}
