//! (v6.2.49) Navegador embebido de flightsim.to para DESCARGAR liveries con la
//! cuenta/sesión del usuario.
//!
//! Por qué así: flightsim.to gatea la descarga con **Cloudflare Turnstile** (el
//! login headless devuelve `{"message":"Turnstile token is required"}`), y el
//! link de descarga se genera por JavaScript con token. No hay forma fiable de
//! bajar el archivo por HTTP. La ÚNICA vía con la cuenta del usuario es un
//! navegador REAL: abrimos una `WebviewWindow` a flightsim.to donde el usuario
//! inicia sesión una vez (su sesión persiste), y **interceptamos la descarga**
//! (`on_download`) → guardamos el archivo → emitimos un evento para que el
//! front lo instale con el MISMO flujo del drag&drop (inspect → modal → install).

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LIVERY_WIN: &str = "livery-browser";

/// Abre (o enfoca) el navegador embebido de flightsim.to. `url` opcional para
/// entrar directo a una categoría de avión (p.ej. `/liveries/pmdg-boeing-737-800`).
#[tauri::command]
pub fn open_livery_browser(app: tauri::AppHandle, url: Option<String>) -> Result<(), String> {
    // Ya abierto → mostrar + enfocar.
    if let Some(w) = app.get_webview_window(LIVERY_WIN) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let start = url.unwrap_or_else(|| "https://flightsim.to/liveries".to_string());
    let parsed: tauri::Url = start
        .parse()
        .map_err(|e| format!("URL inválida: {e}"))?;

    // Carpeta temporal para las descargas interceptadas. La limpiamos de restos
    // viejos para no acumular.
    let dl_dir = std::env::temp_dir().join("simfleet-livery-dl");
    let _ = std::fs::create_dir_all(&dl_dir);
    cleanup_old_downloads(&dl_dir);

    let app_emit = app.clone();
    let dl_dir_cb = dl_dir.clone();

    WebviewWindowBuilder::new(&app, LIVERY_WIN, WebviewUrl::External(parsed))
        .title("Buscar liveries — flightsim.to (inicia sesión con tu cuenta)")
        .inner_size(1280.0, 880.0)
        .min_inner_size(900.0, 600.0)
        .on_download(move |_webview, event| {
            use tauri::webview::DownloadEvent;
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let fname = download_filename(&url);
                    let target = dl_dir_cb.join(&fname);
                    tracing::info!(
                        target: "livery",
                        "descarga interceptada: {} → {}",
                        url, target.display()
                    );
                    *destination = target;
                }
                DownloadEvent::Finished { url, path, success } => {
                    tracing::info!(
                        target: "livery",
                        "descarga terminada (ok={}) url={} path={:?}",
                        success, url, path
                    );
                    if success {
                        if let Some(p) = path {
                            let _ = app_emit.emit(
                                "livery-download://finished",
                                p.to_string_lossy().to_string(),
                            );
                        }
                    } else {
                        let _ = app_emit.emit("livery-download://failed", url.to_string());
                    }
                }
                _ => {}
            }
            // Siempre permitimos la descarga (ya redirigimos el destino).
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    tracing::info!(target: "livery", "navegador de liveries abierto");
    Ok(())
}

/// Nombre de archivo a partir de la URL de descarga. Garantiza una extensión de
/// archivo soportada (`.zip` por defecto) para que el pipeline de drop sepa
/// extraerlo.
fn download_filename(url: &tauri::Url) -> String {
    let raw = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string();
    let decoded = percent_decode(&raw);
    let sanitized = sanitize_filename(decoded.trim());
    let has_archive_ext = sanitized
        .rsplit('.')
        .next()
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "rar" | "7z"))
        .unwrap_or(false);
    if sanitized.is_empty() {
        return format!("livery-{}.zip", now_secs());
    }
    if has_archive_ext {
        sanitized
    } else {
        format!("{}.zip", sanitized.trim_end_matches('.'))
    }
}

/// Percent-decode mínimo (%20 → espacio, etc.), best-effort.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Quita caracteres ilegales de un nombre de archivo Windows.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Borra archivos de descargas de más de 1 día en la carpeta temporal.
fn cleanup_old_downloads(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in rd.flatten() {
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .map(|age| age.as_secs() > 86_400)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
