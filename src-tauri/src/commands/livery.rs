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
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LIVERY_WIN: &str = "livery-browser";

/// Navegador embebido de flightsim.to con captura de descarga.
#[tauri::command]
pub async fn open_livery_browser(
    app: tauri::AppHandle,
    url: Option<String>,
) -> Result<(), String> {
    let start = url.unwrap_or_else(|| "https://flightsim.to/liveries".to_string());
    tracing::info!(target: "livery", "open_livery_browser INICIO → {}", start);
    let parsed: tauri::Url = start.parse().map_err(|e| format!("URL inválida: {e}"))?;

    // (v6.2.57.2) Ya abierto → NAVEGAR a la nueva URL + mostrar + enfocar. Ahora
    // el comando es `async`, así que `navigate()` ya no cuelga el hilo UI (fue el
    // deadlock que arreglamos). Así un 2º badge GSX hace la búsqueda real del
    // nuevo ICAO en la misma ventana, no sólo focus.
    if let Some(w) = app.get_webview_window(LIVERY_WIN) {
        tracing::info!(target: "livery", "reabrir: navegar → {}", start);
        let _ = w.navigate(parsed);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let dl_dir = std::env::temp_dir().join("simfleet-livery-dl");
    let _ = std::fs::create_dir_all(&dl_dir);

    let app_emit = app.clone();
    let dl_dir_cb = dl_dir.clone();

    // (v6.2.60) Última página de archivo de flightsim.to (`/file/<id>` o
    // `/addon/<id>`) visitada en el embebido. La escribe `on_navigation` y la
    // lee `on_download` para asociar la descarga a su `file_id` → tracking de
    // updates del addon una vez instalado.
    let last_file_page: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let lfp_nav = last_file_page.clone();
    let lfp_dl = last_file_page.clone();

    const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
        AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    // (v6.2.55) FIX del render en BLANCO: proveer un `initialization_script`
    // fuerza a Tauri a preparar la tubería de inyección ANTES de navegar.
    // (v6.2.66) Los anuncios se bloquean a nivel de RED (`install_ad_blocker`),
    // NO por CSS/JS (eso congelaba/ennegrecía). (v6.2.68) Se re-añade sólo el
    // salto del countdown, en su forma más ligera y segura: un MutationObserver
    // que mira los nodos NUEVOS y dispara el link de descarga cuando aparece el
    // "your download will start in N seconds". No toca timers ni CSS.
    const LIVERY_INIT_JS: &str = r#"
(function(){
  try { window.__simfleetLivery = true; } catch(e){}
  function trySkip(node){
    try {
      var txt = node.textContent;
      if(!txt || txt.length > 3000) return;
      if(!/download will start in|your download will start/i.test(txt)) return;
      var link=node.querySelector && node.querySelector('a[download], a[href*="cdn"][href*="flightsim"], a[href$=".zip"], a[href$=".rar"], a[href$=".7z"]');
      if(link){ link.click(); }
    } catch(e){}
  }
  try {
    var obs=new MutationObserver(function(muts){
      for(var mi=0;mi<muts.length;mi++){ var an=muts[mi].addedNodes;
        for(var ni=0;ni<an.length;ni++){ if(an[ni].nodeType===1) trySkip(an[ni]); } }
    });
    obs.observe(document.documentElement,{childList:true,subtree:true});
  } catch(e){}
})();
"#;

    let win = WebviewWindowBuilder::new(&app, LIVERY_WIN, WebviewUrl::External(parsed))
        .initialization_script(LIVERY_INIT_JS)
        .title("Buscar liveries — flightsim.to (inicia sesión con tu cuenta)")
        .inner_size(1280.0, 880.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .user_agent(CHROME_UA)
        // (v6.2.53) HIPÓTESIS del blanco: el drag&drop handler que Tauri instala
        // en cada webview rompe el render de WebView2 en algunos sitios. Lo
        // desactivamos en ESTA ventana (no necesitamos soltar archivos aquí).
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            let us = url.as_str();
            tracing::info!(target: "livery", "webview navega → {}", us);
            // (v6.2.60) Recordar la última página de archivo de flightsim.to
            // para asociar la próxima descarga a su file_id.
            if crate::flightsim_track::file_id_from_url(us).is_some() {
                if let Ok(mut g) = lfp_nav.lock() {
                    *g = Some(us.to_string());
                }
            }
            true
        })
        .on_page_load(|_w, payload| {
            tracing::info!(
                target: "livery",
                "webview page_load {:?} → {}",
                payload.event(), payload.url()
            );
        })
        .on_download(move |webview, event| {
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
                            let ps = p.to_string_lossy().to_string();
                            // (v6.2.60) Asociar la descarga con la página de
                            // flightsim.to de la que salió → tracking de updates.
                            // Fuente primaria: la URL ACTUAL del webview (cubre la
                            // navegación SPA, que no dispara on_navigation).
                            // Respaldo: la última página de archivo navegada.
                            let live = webview
                                .url()
                                .ok()
                                .map(|u| u.to_string())
                                .filter(|u| crate::flightsim_track::file_id_from_url(u).is_some());
                            let src_url = live.or_else(|| {
                                lfp_dl.lock().ok().and_then(|g| g.clone())
                            });
                            if let Some(fid) = src_url
                                .as_deref()
                                .and_then(crate::flightsim_track::file_id_from_url)
                            {
                                if let Some(state) = app_emit.try_state::<crate::AppState>() {
                                    if let Ok(mut m) = state.livery_downloads.lock() {
                                        tracing::info!(target: "livery", "descarga {} ↔ flightsim file_id={}", ps, fid);
                                        m.insert(ps.clone(), fid);
                                    }
                                }
                            }
                            let _ = app_emit.emit("livery-download://finished", ps);
                        }
                    }
                }
                _ => {}
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;

    // (v6.2.66) Ad blocker A NIVEL DE RED: bloquea las peticiones a dominios de
    // anuncios/tracking (no las oculta con CSS/JS). No toca el DOM ni la
    // hidratación del SPA — se acabaron los congelamientos y pantallazos negros.
    #[cfg(target_os = "windows")]
    install_ad_blocker(&win);

    tracing::info!(target: "livery", "navegador embebido de liveries abierto → {}", start);
    Ok(())
}

/// (v6.2.66) Instala un filtro de recursos de WebView2 que responde 403 a las
/// peticiones hacia dominios de anuncios/tracking conocidos + el endpoint de
/// anuncios propio de flightsim.to. NO oculta nada del DOM (eso rompía la
/// hidratación); bloquea la descarga del anuncio de raíz.
#[cfg(target_os = "windows")]
fn install_ad_blocker(win: &tauri::WebviewWindow) {
    let _ = win.with_webview(|webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        };
        use webview2_com::WebResourceRequestedEventHandler;
        use windows_core::{w, Interface, PWSTR};
        // SAFETY: API COM de WebView2 en el hilo de UI (mismo patrón que en
        // `run()` para desactivar los accelerator keys).
        unsafe {
            let controller = webview.controller();
            let Ok(core) = controller.CoreWebView2() else {
                return;
            };
            // Filtro para TODAS las peticiones.
            let _ = core
                .AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
            // Entorno para fabricar la respuesta "bloqueado".
            let env = core
                .cast::<ICoreWebView2_2>()
                .ok()
                .and_then(|c2| c2.Environment().ok());
            let handler = WebResourceRequestedEventHandler::create(Box::new(move |_wv, args| {
                if let Some(args) = args {
                    if let Ok(req) = args.Request() {
                        let mut pw = PWSTR::null();
                        if req.Uri(&mut pw).is_ok() {
                            let uri = webview2_com::take_pwstr(pw);
                            if is_ad_url(&uri) {
                                if let Some(env) = env.as_ref() {
                                    if let Ok(resp) = env.CreateWebResourceResponse(
                                        None,
                                        403,
                                        w!("Blocked"),
                                        w!(""),
                                    ) {
                                        let _ = args.SetResponse(&resp);
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }));
            let mut token = 0i64;
            let _ = core.add_WebResourceRequested(&handler, &mut token);
        }
    });
}

/// ¿La URL apunta a un dominio de anuncios/tracking? Lista conservadora: sólo
/// redes de ads/analytics conocidas + el endpoint de anuncios de flightsim.to.
/// NO bloquea el contenido/CDN/API propios de flightsim.to.
#[cfg(target_os = "windows")]
fn is_ad_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    const HOSTS: &[&str] = &[
        "googlesyndication.com",
        "doubleclick.net",
        "googleadservices.com",
        "googletagservices.com",
        "google-analytics.com",
        "googletagmanager.com",
        "adservice.google",
        "adnxs.com",
        "amazon-adsystem.com",
        "pubmatic.com",
        "rubiconproject.com",
        "criteo.com",
        "criteo.net",
        "taboola.com",
        "outbrain.com",
        "scorecardresearch.com",
        "moatads.com",
        "adform.net",
        "adsafeprotected.com",
        "aniview.com",
        "pub.network",
        "sekindo.com",
        "3lift.com",
        "casalemedia.com",
        "openx.net",
        "smartadserver.com",
        "teads.tv",
        "yieldmo.com",
        "sharethrough.com",
        "bidswitch.net",
        "onetag.com",
        "gumgum.com",
        "indexww.com",
        "adsrvr.org",
        "quantserve.com",
    ];
    if HOSTS.iter().any(|h| u.contains(h)) {
        return true;
    }
    // Endpoint de anuncios propio de flightsim.to.
    u.contains("flightsim.to/backend/ads/")
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
    // Conserva la extensión si es un archivo conocido (comprimido O instalador);
    // si no hay una clara, asume `.zip`.
    let known_ext = sanitized
        .rsplit('.')
        .next()
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "zip" | "rar" | "7z" | "exe" | "msi" | "msix" | "msixbundle" | "appx"
            )
        })
        .unwrap_or(false);
    if sanitized.is_empty() {
        format!("livery-{}.zip", now_secs())
    } else if known_ext {
        sanitized
    } else {
        format!("{}.zip", sanitized.trim_end_matches('.'))
    }
}

/// (v6.2.56) Copia una app/instalador descargado a la carpeta que ELIJA el
/// usuario — NUNCA a Community. Devuelve la ruta final.
#[tauri::command]
pub fn save_installer_to(src: String, dest_folder: String) -> Result<String, String> {
    let src = std::path::PathBuf::from(&src);
    let dest_dir = std::path::PathBuf::from(&dest_folder);
    let file_name = src
        .file_name()
        .ok_or_else(|| "archivo sin nombre".to_string())?;
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let dest = dest_dir.join(file_name);
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    tracing::info!(
        target: "livery",
        "app/instalador guardado: {} → {}",
        src.display(), dest.display()
    );
    Ok(dest.to_string_lossy().into_owned())
}

// ===========================================================================
// Tracking de updates de addons de flightsim.to (v6.2.60)
// ===========================================================================

/// Revisa updates de los addons que descargaste de flightsim.to por el
/// navegador embebido. Compara el `updatedAt` actual del catálogo contra el
/// baseline guardado al instalar. Devuelve una entrada por carpeta rastreada
/// (el front filtra por `hasUpdate`).
#[tauri::command]
pub async fn flightsim_check_updates(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::flightsim_track::FlightsimUpdate>, String> {
    Ok(crate::flightsim_track::check_updates(&state.db, &state.http).await)
}

/// Marca un addon de flightsim.to como manejado: avanza su baseline al
/// `updatedAt` actual para que el badge no reaparezca tras pulsarlo.
#[tauri::command]
pub async fn flightsim_ack_update(
    folder_name: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    crate::flightsim_track::ack_update(&state.db, &state.http, &folder_name).await;
    Ok(())
}

/// (DEBUG, temporal) Fuerza que los addons rastreados de flightsim.to aparezcan
/// con update (rebobina el baseline). Para probar el flujo del badge/campana.
/// Devuelve cuántos se afectaron (0 = aún no descargaste nada por el embebido).
#[tauri::command]
pub async fn flightsim_debug_force_update(
    state: tauri::State<'_, crate::AppState>,
) -> Result<u64, String> {
    Ok(crate::flightsim_track::debug_force_update(&state.db).await)
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
