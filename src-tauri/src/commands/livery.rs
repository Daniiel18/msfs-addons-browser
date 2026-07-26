//! (v6.2.53) Descarga de liveries de flightsim.to con la cuenta del usuario.
//!
//! Dos caminos coexisten:
//!   · `open_livery_browser` — navegador EMBEBIDO (WebviewWindow) a flightsim.to
//!     con captura de descarga. Salía en blanco; en v6.2.53 probamos el fix de
//!     desactivar el drag&drop handler (causa conocida de página en blanco en
//!     WebView2 con multi-webview) + UA de Chrome + logging de navegación.
//!   · `start_livery_download_watch` — FALLBACK fiable: abre flightsim.to en el
//!     navegador real y vigila la carpeta de Descargas para auto-instalar.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LIVERY_WIN: &str = "livery-browser";

/// Navegador embebido de flightsim.to con captura de descarga.
#[tauri::command]
pub fn open_livery_browser(app: tauri::AppHandle, url: Option<String>) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LIVERY_WIN) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let start = url.unwrap_or_else(|| "https://flightsim.to/liveries".to_string());
    let parsed: tauri::Url = start.parse().map_err(|e| format!("URL inválida: {e}"))?;

    let dl_dir = std::env::temp_dir().join("simfleet-livery-dl");
    let _ = std::fs::create_dir_all(&dl_dir);

    let app_emit = app.clone();
    let dl_dir_cb = dl_dir.clone();

    const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
        AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    WebviewWindowBuilder::new(&app, LIVERY_WIN, WebviewUrl::External(parsed))
        .title("Buscar liveries — flightsim.to (inicia sesión con tu cuenta)")
        .inner_size(1280.0, 880.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .user_agent(CHROME_UA)
        // (v6.2.53) HIPÓTESIS del blanco: el drag&drop handler que Tauri instala
        // en cada webview rompe el render de WebView2 en algunos sitios. Lo
        // desactivamos en ESTA ventana (no necesitamos soltar archivos aquí).
        .disable_drag_drop_handler()
        .on_navigation(|url| {
            tracing::info!(target: "livery", "webview navega → {}", url);
            true
        })
        .on_page_load(|_w, payload| {
            tracing::info!(
                target: "livery",
                "webview page_load {:?} → {}",
                payload.event(), payload.url()
            );
        })
        .on_download(move |_webview, event| {
            use tauri::webview::DownloadEvent;
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let fname = download_filename(&url);
                    let target = dl_dir_cb.join(&fname);
                    tracing::info!(target: "livery", "descarga interceptada: {} → {}", url, target.display());
                    *destination = target;
                }
                DownloadEvent::Finished { url, path, success } => {
                    tracing::info!(target: "livery", "descarga terminada ok={} url={} path={:?}", success, url, path);
                    if success {
                        if let Some(p) = path {
                            let _ = app_emit.emit("livery-download://finished", p.to_string_lossy().to_string());
                        }
                    }
                }
                _ => {}
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    tracing::info!(target: "livery", "navegador embebido de liveries abierto → {}", start);
    Ok(())
}

fn download_filename(url: &tauri::Url) -> String {
    let raw = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("")
        .to_string();
    let sanitized: String = raw
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .collect();
    let has_ext = sanitized
        .rsplit('.')
        .next()
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "rar" | "7z"))
        .unwrap_or(false);
    if sanitized.is_empty() {
        format!("livery-{}.zip", now_secs())
    } else if has_ext {
        sanitized
    } else {
        format!("{}.zip", sanitized.trim_end_matches('.'))
    }
}

// ===========================================================================
// Fallback: vigilancia de la carpeta de Descargas
// ===========================================================================

static WATCHING: AtomicBool = AtomicBool::new(false);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
const WATCH_WINDOW_SECS: u64 = 25 * 60;

#[tauri::command]
pub fn start_livery_download_watch(app: tauri::AppHandle) -> Result<(), String> {
    DEADLINE.store(now_secs() + WATCH_WINDOW_SECS, Ordering::SeqCst);
    if WATCHING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let dirs = downloads_dirs();
    if dirs.is_empty() {
        WATCHING.store(false, Ordering::SeqCst);
        return Err("No pude localizar la carpeta de Descargas".into());
    }
    tracing::info!(target: "livery", "watch: vigilando {:?}", dirs);
    std::thread::Builder::new()
        .name("livery-download-watch".into())
        .spawn(move || watch_loop(app, dirs))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn watch_loop(app: tauri::AppHandle, dirs: Vec<PathBuf>) {
    let mut processed: HashSet<PathBuf> = HashSet::new();
    for d in &dirs {
        for f in list_archives(d) {
            processed.insert(f);
        }
    }
    let mut sizes: HashMap<PathBuf, u64> = HashMap::new();
    loop {
        std::thread::sleep(Duration::from_secs(3));
        if now_secs() > DEADLINE.load(Ordering::SeqCst) {
            break;
        }
        for d in &dirs {
            for f in list_archives(d) {
                if processed.contains(&f) {
                    continue;
                }
                let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                let prev = sizes.insert(f.clone(), size);
                if prev != Some(size) || size == 0 {
                    continue;
                }
                processed.insert(f.clone());
                sizes.remove(&f);
                handle_new_archive(&app, &f);
            }
        }
    }
    WATCHING.store(false, Ordering::SeqCst);
    tracing::info!(target: "livery", "watch: terminada");
}

fn handle_new_archive(app: &tauri::AppHandle, path: &Path) {
    let state = app.state::<crate::AppState>();
    let insp = match crate::drop_install::inspect(path, None, &state.drop_sessions) {
        Ok(i) => i,
        Err(_) => return,
    };
    let has_livery = insp
        .items
        .iter()
        .any(|i| matches!(i.kind.as_str(), "pmdg_livery" | "ifly_livery"));
    crate::drop_install::cancel(&insp.session_id, &state.drop_sessions);
    if has_livery {
        tracing::info!(target: "livery", "watch: livery detectada en {} → instalando", path.display());
        let _ = app.emit("livery-download://finished", path.to_string_lossy().to_string());
    }
}

fn list_archives(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "rar" | "7z"))
                    .unwrap_or(false)
        })
        .collect()
}

fn downloads_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    if let Some(p) = downloads_from_registry() {
        out.push(p);
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        let p = Path::new(&up).join("Downloads");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.into_iter().filter(|p| p.is_dir()).collect()
}

#[cfg(windows)]
fn downloads_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .ok()?;
    let val: String = key.get_value("{374DE290-123F-4565-9164-39C4925E467B}").ok()?;
    let trimmed = val.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
