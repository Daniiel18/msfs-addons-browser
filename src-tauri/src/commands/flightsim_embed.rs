//! (19/10) flightsim.to DENTRO de Explore.
//!
//! En vez de abrir la ventana aparte de liveries, Explore muestra la web de
//! flightsim.to como un webview HIJO de la ventana principal, colocado justo
//! encima del hueco que reserva el frontend (`FlightsimEmbed.tsx`). El
//! frontend nos manda ese rectángulo (px lógicos del área cliente) cada vez
//! que cambia, y lo ocultamos cuando se abre un panel/modal de SimFleet
//! (el webview nativo queda siempre por encima del HTML de la app).
//!
//! Comparte con la ventana de liveries el overlay de descarga, el driver de la
//! cuenta atrás, el bloqueo de anuncios, el progreso y el tracking de updates
//! (`livery::handle_flightsim_download`): lo que se baja aquí se instala igual.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

use super::livery;

const EMBED_LABEL: &str = "fsto-embed";
const HOME_URL: &str = "https://flightsim.to/";

/// Serializa la CREACIÓN del webview: si el hueco aparece, se oculta y vuelve
/// mientras se está creando, la segunda llamada espera y reutiliza el primero.
static CREATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Rectángulo del hueco de Explore, en px lógicos del área cliente.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct EmbedBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

fn apply_bounds(webview: &tauri::Webview, b: EmbedBounds) {
    let _ = webview.set_position(LogicalPosition::new(b.x, b.y));
    let _ = webview.set_size(LogicalSize::new(b.width.max(1.0), b.height.max(1.0)));
}

/// Muestra (o crea) flightsim.to en el rectángulo dado.
#[tauri::command]
pub async fn fsto_embed_show(
    app: tauri::AppHandle,
    bounds: EmbedBounds,
    url: Option<String>,
) -> Result<(), String> {
    if let Some(wv) = app.get_webview(EMBED_LABEL) {
        apply_bounds(&wv, bounds);
        if let Some(u) = url {
            let parsed: tauri::Url = u.parse().map_err(|e| format!("{e}"))?;
            let _ = wv.navigate(parsed);
        }
        let _ = wv.show();
        return Ok(());
    }

    let _guard = CREATE_LOCK.lock().await;
    if let Some(wv) = app.get_webview(EMBED_LABEL) {
        apply_bounds(&wv, bounds);
        let _ = wv.show();
        return Ok(());
    }

    let start = url.unwrap_or_else(|| HOME_URL.to_string());
    let parsed: tauri::Url = start.parse().map_err(|e| format!("{e}"))?;
    let main = app
        .get_window("main")
        .ok_or_else(|| "no hay ventana principal".to_string())?;

    let dl_dir = std::env::temp_dir().join("simfleet-livery-dl");
    let _ = std::fs::create_dir_all(&dl_dir);
    let last_file_page: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let lfp_nav = last_file_page.clone();
    let app_emit = app.clone();
    let app_nav = app.clone();
    let init = livery::flightsim_init_script(&livery::pref_language(&app).await);

    let mut builder = tauri::webview::WebviewBuilder::new(EMBED_LABEL, WebviewUrl::External(parsed))
        .initialization_script(init.as_str())
        // Mismo motivo que en la ventana de liveries: el handler de
        // drag&drop de Tauri deja en blanco algunas webs en WebView2.
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            let us = url.as_str();
            // Botón "Cancelar" del overlay de descarga: cerramos el webview
            // (cancela la descarga) y el front lo vuelve a crear.
            if us.contains("cancel.simfleet.invalid") {
                if let Some(wv) = app_nav.get_webview(EMBED_LABEL) {
                    let _ = wv.close();
                }
                let _ = app_nav.emit("fsto-embed://closed", ());
                return false;
            }
            if crate::flightsim_track::file_id_from_url(us).is_some() {
                if let Ok(mut g) = lfp_nav.lock() {
                    *g = Some(us.to_string());
                }
            }
            true
        })
        .on_page_load(|wv, payload| {
            tracing::debug!(target: "livery", "embebido page_load {:?} → {}", payload.event(), payload.url());
            let _ = wv.emit("fsto-embed://url", payload.url().to_string());
        })
        .on_download(move |webview, event| {
            livery::handle_flightsim_download(&app_emit, &dl_dir, &last_file_page, &webview, event)
        });
    if let Some(dir) = livery::extensions_dir() {
        builder = builder.browser_extensions_enabled(true).extensions_path(dir);
    }

    let wv = main
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    // El webview principal debe seguir llenando la ventana al redimensionar
    // ahora que la ventana tiene más de un webview.
    if let Some(main_wv) = app.get_webview("main") {
        let _ = main_wv.set_auto_resize(true);
    }
    #[cfg(target_os = "windows")]
    {
        livery::install_web_guards(&wv, false);
        livery::install_download_progress(&wv);
    }
    tracing::info!(target: "livery", "flightsim.to embebido en Explore → {}", start);
    Ok(())
}

/// Recoloca el webview (cambio de tamaño de ventana / layout).
#[tauri::command]
pub fn fsto_embed_bounds(app: tauri::AppHandle, bounds: EmbedBounds) {
    if let Some(wv) = app.get_webview(EMBED_LABEL) {
        apply_bounds(&wv, bounds);
    }
}

/// Oculta el webview (fuera de Explore o con un panel de SimFleet abierto).
#[tauri::command]
pub fn fsto_embed_hide(app: tauri::AppHandle) {
    if let Some(wv) = app.get_webview(EMBED_LABEL) {
        let _ = wv.hide();
    }
}

/// Navegación desde la barra de Explore: "back" | "forward" | "reload" |
/// "home" o una URL de flightsim.to.
#[tauri::command]
pub fn fsto_embed_nav(app: tauri::AppHandle, action: String) -> Result<(), String> {
    let Some(wv) = app.get_webview(EMBED_LABEL) else {
        return Ok(());
    };
    match action.as_str() {
        "back" => wv.eval("history.back()").map_err(|e| e.to_string()),
        "forward" => wv.eval("history.forward()").map_err(|e| e.to_string()),
        "reload" => wv.eval("location.reload()").map_err(|e| e.to_string()),
        "home" => wv
            .navigate(HOME_URL.parse().map_err(|e| format!("{e}"))?)
            .map_err(|e| e.to_string()),
        other => {
            let u: tauri::Url = other.parse().map_err(|e| format!("{e}"))?;
            if u.host_str().is_some_and(|h| h.ends_with("flightsim.to")) {
                wv.navigate(u).map_err(|e| e.to_string())
            } else {
                Err("sólo URLs de flightsim.to".into())
            }
        }
    }
}
