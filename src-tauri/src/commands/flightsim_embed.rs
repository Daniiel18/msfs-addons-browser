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
    // (v8.3) Descargas → jobs del panel de Descargas (la web no se bloquea).
    let url_jobs: livery::ExternalJobs = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let url_jobs_dl = url_jobs.clone();
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
            livery::handle_flightsim_download(&app_emit, &dl_dir, &last_file_page, &url_jobs_dl, &webview, event)
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
        livery::install_external_job_progress(&wv, app.clone(), url_jobs.clone());
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

/// (25/10) Foto PNG (data URL) de flightsim.to tal como se ve ahora. Mientras
/// un panel o modal de SimFleet tapa el hueco, el webview nativo se oculta
/// (quedaría por encima) y el hueco enseña esta foto: la página no
/// "desaparece". `None` si no hay webview o la captura falla.
#[tauri::command]
pub async fn fsto_embed_snapshot(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use base64::Engine;
        let Some(wv) = app.get_webview(EMBED_LABEL) else { return Ok(None) };
        type Tx = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Option<Vec<u8>>>>>>;
        fn send(tx: &Tx, v: Option<Vec<u8>>) {
            if let Some(t) = tx.lock().unwrap().take() {
                let _ = t.send(v);
            }
        }
        let (tx, rx) = tokio::sync::oneshot::channel::<Option<Vec<u8>>>();
        let tx: Tx = Arc::new(Mutex::new(Some(tx)));
        let _ = wv.with_webview(move |w| {
            use webview2_com::CapturePreviewCompletedHandler;
            use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
            use windows::Win32::Foundation::HGLOBAL;
            use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
            use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
            // SAFETY: COM de WebView2 en el hilo de UI (mismo patrón que
            // `livery::install_web_guards`); el stream vive en este hilo.
            unsafe {
                let Ok(core) = w.controller().CoreWebView2() else {
                    send(&tx, None);
                    return;
                };
                let Ok(stream) = CreateStreamOnHGlobal(HGLOBAL::default(), true) else {
                    send(&tx, None);
                    return;
                };
                let stream_cb = stream.clone();
                let tx_cb = tx.clone();
                let handler = CapturePreviewCompletedHandler::create(Box::new(move |hr| {
                    if hr.is_err() {
                        send(&tx_cb, None);
                        return Ok(());
                    }
                    let mut stat = STATSTG::default();
                    let read = stream_cb.Stat(&mut stat, STATFLAG_NONAME).ok().and_then(|_| {
                        let len = stat.cbSize as usize;
                        let mut buf = vec![0u8; len];
                        let mut got = 0u32;
                        stream_cb.Seek(0, STREAM_SEEK_SET, None).ok()?;
                        stream_cb.Read(buf.as_mut_ptr().cast(), len as u32, Some(&mut got)).ok().ok()?;
                        buf.truncate(got as usize);
                        (!buf.is_empty()).then_some(buf)
                    });
                    send(&tx_cb, read);
                    Ok(())
                }));
                if core
                    .CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler)
                    .is_err()
                {
                    send(&tx, None);
                }
            }
        });
        let png = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        Ok(png.map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b))))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Ok(None)
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
