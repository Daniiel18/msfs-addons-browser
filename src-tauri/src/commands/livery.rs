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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const LIVERY_WIN: &str = "livery-browser";

/// (v7) Carpeta PADRE de extensiones para el WebView (contiene `ublock-lite/`).
/// La prepara `lib.rs` al arranque (extrae el zip bundleado de uBlock Origin
/// Lite). Las ventanas de descarga la usan para cargar la extensión y matar
/// los anuncios de los hosters (mixdrop, etc.). None → no se cargó/extajo.
static UBOL_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Llamado una vez desde `lib.rs` tras extraer la extensión.
pub(crate) fn set_extensions_dir(dir: PathBuf) {
    let _ = UBOL_DIR.set(dir);
}

/// (v7) Carpeta temporal donde caen las descargas del navegador de HOSTERS
/// (modsfire, mixdrop, get.php de sceneryaddons…). El frontend la reconoce
/// (`isEmbeddedDownload`) para auto-instalar + auto-borrar sin preguntar.
const MIRROR_DL_DIR: &str = "simfleet-mirror-dl";

/// User-Agent de Chrome — algunos hosters/CDN rechazan el UA por defecto de
/// WebView2. Compartido por el navegador de liveries y el de hosters.
const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Navegador embebido de flightsim.to con captura de descarga.
#[tauri::command]
pub async fn open_livery_browser(
    app: tauri::AppHandle,
    url: Option<String>,
) -> Result<(), String> {
    let start = url.unwrap_or_else(|| "https://flightsim.to/liveries".to_string());
    tracing::info!(target: "livery", "open_livery_browser INICIO → {}", start);
    let parsed: tauri::Url = start.parse().map_err(|e| crate::tr!("livery.invalidUrl", e = e))?;

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
    // Para el botón ✕ del overlay (cerrar la ventana de liveries).
    let app_cancel_lv = app.clone();

    // (v6.2.55) FIX del render en BLANCO: proveer un `initialization_script`
    // fuerza a Tauri a preparar la tubería de inyección ANTES de navegar.
    // (v6.2.66) Los anuncios se bloquean a nivel de RED (`install_web_guards`),
    // NO por CSS/JS (eso congelaba/ennegrecía).
    //
    // (v7, 6.2.82) SALTO de la cuenta atrás de descarga de flightsim.to
    // ("your download will start in N seconds"), BIEN hecho:
    //   · ANTES (roto): un MutationObserver clicaba a ciegas cualquier link de
    //     descarga al ver el texto → a veces bajaba una página intermedia
    //     'd.php' (zip inválido: "Could not find EOCD"). Eliminado.
    //   · AHORA: recortamos los timers en rango de cuenta-atrás (500–2000 ms →
    //     40 ms) para que el contador corra solo y flightsim.to dispare la
    //     descarga REAL del CDN (que capturamos bien). El override se instala
    //     DESPUÉS de `load` (+2 s) para NO romper la hidratación/clics (regla
    //     dura: pisar timers en document-start deja la página sin aceptar
    //     clics hasta F5). Sólo afecta timers NUEVOS (la cuenta atrás arranca
    //     al pulsar Download, ya con el override puesto).
    // (v7) Overlay: en flightsim.to el usuario NAVEGA para buscar, así que el
    // overlay bloqueante NO va desde el arranque — aparece SÓLO cuando arranca
    // la descarga (aparece el texto del contador), bloquea ~5 s (cubre la
    // cuenta atrás acelerada) y se apaga solo para seguir navegando.
    const LIVERY_DRIVER_JS: &str = r#"
(function(){
  try { window.__simfleetLivery = true; } catch(e){}
  function fastCountdown(){
    try {
      var _sto = window.setTimeout, _sti = window.setInterval;
      function clamp(d){ return (typeof d === 'number' && d >= 500 && d <= 2000) ? 40 : d; }
      window.setTimeout = function(fn, d){
        var a = Array.prototype.slice.call(arguments, 2);
        return _sto.apply(window, [fn, clamp(d)].concat(a));
      };
      window.setInterval = function(fn, d){
        var a = Array.prototype.slice.call(arguments, 2);
        return _sti.apply(window, [fn, clamp(d)].concat(a));
      };
    } catch(e){}
  }
  try {
    if (document.readyState === 'complete') { setTimeout(fastCountdown, 2000); }
    else { window.addEventListener('load', function(){ setTimeout(fastCountdown, 2000); }); }
  } catch(e){}
  // (v7 fix) El overlay NO se dispara por el texto del countdown (eso tapaba y
  // bloqueaba el botón de Download que aparece DESPUÉS). El overlay aparece
  // cuando la descarga ARRANCA de verdad (on_download Requested → eval), igual
  // que en los hosters, y se quita al terminar (on_download Finished → eval).
})();
"#;

    // Overlay compartido + driver de liveries, con el idioma de la app.
    let lang = {
        match app.try_state::<crate::AppState>() {
            Some(state) => sqlx::query_as::<_, (String,)>(
                "SELECT value FROM settings WHERE key='pref_language'",
            )
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .map(|r| r.0)
            .unwrap_or_default(),
            None => String::new(),
        }
    };
    let livery_init = format!("{}\n{OVERLAY_JS}\n{LIVERY_DRIVER_JS}", lang_prefix(&lang));

    let mut builder = WebviewWindowBuilder::new(&app, LIVERY_WIN, WebviewUrl::External(parsed))
        .initialization_script(livery_init.as_str())
        .title(crate::tr!("livery.windowTitle"))
        // (v7.1) Ventana FLOTANTE más chica que la principal. Es OWNED de la
        // ventana `main` (se setea abajo, antes de `build`) → queda SIEMPRE
        // encima de SimFleet pero NO encima del resto de apps de Windows, así el
        // usuario puede seguir usando su PC mientras la descarga corre. (Antes
        // era `always_on_top` = se ponía sobre TODO y bloqueaba el monitor.)
        .inner_size(1024.0, 720.0)
        .min_inner_size(820.0, 560.0)
        .center()
        .focused(true)
        .user_agent(CHROME_UA)
        // (v6.2.53) HIPÓTESIS del blanco: el drag&drop handler que Tauri instala
        // en cada webview rompe el render de WebView2 en algunos sitios. Lo
        // desactivamos en ESTA ventana (no necesitamos soltar archivos aquí).
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            let us = url.as_str();
            // (v7) Centinela del botón ✕ del overlay → cerrar la ventana.
            if us.contains("cancel.simfleet.invalid") {
                if let Some(w) = app_cancel_lv.get_webview_window(LIVERY_WIN) {
                    let _ = w.close();
                }
                return false;
            }
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
                    // (v7) La descarga ARRANCÓ (el usuario ya clicó el botón de
                    // Download que aparece tras el countdown) → recién AHORA
                    // ponemos el overlay bloqueante. Aviso también al front.
                    let _ = webview.eval(
                        "window.__sfDownloading=true;if(window.__sfSet)window.__sfSet('block');",
                    );
                    let _ = app_emit.emit("embedded-download://started", fname.clone());
                    *destination = target;
                }
                DownloadEvent::Finished { url, path, success } => {
                    tracing::info!(target: "livery", "descarga terminada ok={} url={} path={:?}", success, url, path);
                    // (v7) La ventana de liveries NO se auto-cierra (el usuario
                    // sigue navegando para bajar más), así que quitamos el
                    // overlay al terminar la descarga.
                    let _ = webview.eval(
                        "window.__sfDownloading=false;if(window.__sfSet)window.__sfSet('off');",
                    );
                    if success {
                        if let Some(p) = path {
                            // (v7) Misma corrección de extensión por firma que en
                            // hosters (por si un CDN sirve el archivo sin nombre).
                            let fixed = fix_archive_extension(&p);
                            let ps = fixed.to_string_lossy().to_string();
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
        });
    // (v7) uBlock Origin Lite también en la ventana de flightsim.to (misma
    // extensión; si no cargó, se ignora).
    if let Some(dir) = UBOL_DIR.get() {
        builder = builder.browser_extensions_enabled(true).extensions_path(dir);
    }
    // (v7.1) OWNED de la ventana principal → encima de SimFleet pero NO encima
    // del resto de apps de Windows (el usuario puede usar su PC mientras baja).
    if let Some(main_win) = app.get_webview_window("main") {
        builder = builder.parent(&main_win).map_err(|e| e.to_string())?;
    }
    let win = builder.build().map_err(|e| e.to_string())?;

    // (v6.2.66) Ad blocker A NIVEL DE RED: bloquea las peticiones a dominios de
    // anuncios/tracking (no las oculta con CSS/JS). No toca el DOM ni la
    // hidratación del SPA — se acabaron los congelamientos y pantallazos negros.
    // (v7) Liveries sin bloqueo de popups (comportamiento de siempre).
    #[cfg(target_os = "windows")]
    {
        install_web_guards(&win, false);
        install_download_progress(&win);
    }

    tracing::info!(target: "livery", "navegador embebido de liveries abierto → {}", start);
    Ok(())
}

/// (v7) Overlay de SimFleet inyectado en el WebView. Es DOM ADITIVO (un `<div>`
/// + listener de teclado), NO pisa globales nativos → no dispara el anti-tamper
/// de los hosters. Expone `window.__sfSet(mode, msg)`:
///   · `'block'` → tapa toda la pantalla, bloquea clics (spinner "descargando"),
///     y bloquea F5/Ctrl+R. Los clics PROGRAMÁTICOS (auto-avance) igual pasan.
///   · `'hint'`  → banner NO bloqueante arriba (ej. "resolvé el captcha").
///   · `'off'`   → oculto.
const OVERLAY_JS: &str = r#"
(function(){
  if(window.__sfOv) return;
  // Idioma: `window.__sfLang` lo pone Rust ('es'|'en'|''); '' → navigator.language.
  var en=(window.__sfLang==='en')||(!window.__sfLang&&(navigator.language||'').toLowerCase().indexOf('es')!==0);
  window.__sfL = en ? {
    dl:'SimFleet is downloading…',
    sub:"Don't touch anything — it's automatic and installs when it's done.",
    proc:'Processing files…',
    procsub:"Almost there — SimFleet is extracting and will open the install window.",
    cap:'🔒  Solve the captcha to continue',
    click:'👉  Click DOWNLOAD to get the file — SimFleet installs it automatically',
    cancel:'Cancel'
  } : {
    dl:'SimFleet está descargando…',
    sub:'No toques nada — se hace solo y se instala al terminar.',
    proc:'Procesando archivos…',
    procsub:'Casi listo — SimFleet está extrayendo y abrirá la ventana de instalación.',
    cap:'🔒  Resolvé el captcha para continuar',
    click:'👉  Hacé clic en DESCARGAR para bajar el archivo — se instala solo',
    cancel:'Cancelar'
  };
  var S={mode:'off',msg:''}; window.__sfOv=S;
  function el(i){return document.getElementById(i);}
  function scrollLock(on){
    try{
      var de=document.documentElement, bd=document.body;
      if(de) de.style.overflow=on?'hidden':'';
      if(bd) bd.style.overflow=on?'hidden':'';
    }catch(e){}
  }
  function build(){
    if(el('__sfov')) return el('__sfov');
    var st=document.createElement('style');
    st.textContent='@keyframes sfbar{0%{transform:translateX(-130%)}100%{transform:translateX(340%)}}'
      +'#__sfov_cancel:hover{background:#dc2626 !important}';
    (document.head||document.documentElement).appendChild(st);
    var o=document.createElement('div'); o.id='__sfov';
    o.style.cssText='position:fixed;left:0;top:0;right:0;bottom:0;z-index:2147483646;'
      +'display:none;flex-direction:column;align-items:center;justify-content:center;'
      +'gap:22px;font-family:system-ui,Segoe UI,Roboto,sans-serif;color:#eafaf1';
    // Bloque de descarga: título + sub + (barra con % ·vel a la izq / ETA a la der).
    var main=document.createElement('div'); main.id='__sfov_main';
    main.style.cssText='display:none;flex-direction:column;align-items:center;gap:14px';
    var m=document.createElement('div'); m.id='__sfov_m';
    m.style.cssText='font-size:18px;font-weight:700;text-align:center;max-width:82vw';
    var s=document.createElement('div'); s.id='__sfov_s';
    s.style.cssText='font-size:13px;color:#93c5a9;text-align:center;max-width:82vw';
    var wrap=document.createElement('div');
    wrap.style.cssText='display:flex;flex-direction:column;gap:7px;width:360px;max-width:80vw';
    var bar=document.createElement('div');
    bar.style.cssText='width:100%;height:9px;border-radius:999px;background:rgba(34,197,94,.16);overflow:hidden';
    var fill=document.createElement('div'); fill.id='__sfov_fill';
    fill.style.cssText='width:38%;height:100%;border-radius:999px;'
      +'background:linear-gradient(90deg,#16a34a,#4ade80);animation:sfbar 1.1s ease-in-out infinite';
    bar.appendChild(fill);
    var row=document.createElement('div');
    row.style.cssText='display:flex;justify-content:space-between;align-items:baseline;width:100%;font-size:12.5px';
    var pct=document.createElement('div'); pct.id='__sfov_pct'; pct.style.cssText='color:#eafaf1;font-weight:600';
    var eta=document.createElement('div'); eta.id='__sfov_eta'; eta.style.cssText='color:#93c5a9';
    row.appendChild(pct); row.appendChild(eta);
    wrap.appendChild(bar); wrap.appendChild(row);
    main.appendChild(m); main.appendChild(s); main.appendChild(wrap);
    // Banner de hint (captcha / mixdrop) — fijo arriba, no bloqueante.
    var hint=document.createElement('div'); hint.id='__sfov_hint';
    hint.style.cssText='position:fixed;top:14px;left:50%;transform:translateX(-50%);'
      +'font-size:15px;font-weight:700;background:rgba(6,10,16,.94);padding:10px 16px;'
      +'border-radius:12px;border:1px solid rgba(34,197,94,.45);border-left:4px solid #22c55e;'
      +'max-width:84vw;text-align:center;display:none;pointer-events:none';
    // Botón Cancelar (ROJO). En descarga va debajo de los labels; en las demás
    // fases va fijo abajo-centro (siempre disponible — reemplaza el botón de
    // cerrar de la ventana, que se quitó).
    var cancel=document.createElement('div'); cancel.id='__sfov_cancel';
    cancel.style.cssText='pointer-events:auto;cursor:pointer;user-select:none;'
      +'background:#b91c1c;color:#fff;font-weight:700;font-size:13px;padding:9px 30px;'
      +'border-radius:10px;border:1px solid rgba(255,255,255,.12);box-shadow:0 6px 18px rgba(0,0,0,.4)';
    cancel.textContent=window.__sfL.cancel;
    cancel.onclick=function(){ try{ location.href='https://cancel.simfleet.invalid/'; }catch(e){} };
    o.appendChild(main); o.appendChild(hint); o.appendChild(cancel);
    (document.body||document.documentElement).appendChild(o);
    return o;
  }
  function render(){
    var o=build(),main=el('__sfov_main'),hint=el('__sfov_hint'),cancel=el('__sfov_cancel');
    o.style.display='flex';
    if(S.mode==='block'){
      o.style.background='rgba(6,10,16,.94)'; o.style.pointerEvents='auto'; o.style.justifyContent='center';
      main.style.display='flex'; hint.style.display='none';
      cancel.style.position='static'; cancel.style.transform='';
      if(window.__sfProc){
        el('__sfov_m').textContent=window.__sfL.proc;
        el('__sfov_s').textContent=window.__sfL.procsub;
      } else {
        el('__sfov_m').textContent=window.__sfL.dl+(window.__sfPart?('  ·  '+window.__sfPart):'');
        el('__sfov_s').textContent=window.__sfL.sub;
      }
      scrollLock(true);
    } else {
      // off / hint: página interactiva; sólo el Cancelar (abajo-centro) y, en
      // hint, el banner arriba.
      o.style.background='transparent'; o.style.pointerEvents='none'; o.style.justifyContent='center';
      main.style.display='none';
      if(S.mode==='hint'){ hint.style.display='block'; hint.textContent=S.msg||window.__sfL.cap; }
      else { hint.style.display='none'; }
      cancel.style.position='fixed'; cancel.style.left='50%'; cancel.style.bottom='16px';
      cancel.style.transform='translateX(-50%)';
      scrollLock(false);
    }
  }
  window.__sfSet=function(mode,msg){
    // (fix) Si no cambió, NO re-renderizar: el driver llama __sfSet cada 400ms
    // mientras baja, y re-renderizar pisaría el % del progreso.
    if(S.mode===mode && S.msg===(msg||'')) return;
    S.mode=mode; S.msg=msg||''; try{render();}catch(e){}
  };
  // (v7) Progreso REAL (lo empuja Rust vía ExecuteScript desde BytesReceivedChanged):
  // barra determinada + "% · velocidad" a la IZQUIERDA y "ETA" a la DERECHA.
  window.__sfProgress=function(pct,speed,eta){
    try{
      var f=el('__sfov_fill'), pctEl=el('__sfov_pct'), etaEl=el('__sfov_eta');
      if(pct>=0){
        var p=Math.max(2,Math.min(100,pct));
        if(f){ f.style.animation='none'; f.style.transition='width .25s ease'; f.style.width=p+'%'; }
        if(pctEl) pctEl.textContent=(pct|0)+'%'+(speed?('  ·  '+speed):'');
        if(etaEl) etaEl.textContent=(eta||'');
      } else {
        // Indeterminado (sin Content-Length): barra animada + velocidad.
        if(pctEl) pctEl.textContent=(speed||'');
        if(etaEl) etaEl.textContent='';
      }
    }catch(e){}
  };
  // (v7) Fase de PROCESAMIENTO: la descarga terminó y SimFleet está extrayendo
  // los archivos (el WebView tapa a SimFleet, así que el progreso va acá). Barra
  // "trickle" que avanza asintóticamente hacia ~95% (no sabemos el % exacto de
  // extracción) y la ventana se cierra sola cuando el modal está listo.
  var __sfTrickle=null;
  window.__sfProcessing=function(){
    try{
      window.__sfProc=true; window.__sfDownloading=true; window.__sfPart='';
      if(S.mode!=='block'){ window.__sfSet('block'); } else { render(); }
      var etaEl=el('__sfov_eta'); if(etaEl) etaEl.textContent='';
      if(__sfTrickle) return;
      var p=4;
      __sfTrickle=setInterval(function(){
        if(!window.__sfProc){ clearInterval(__sfTrickle); __sfTrickle=null; return; }
        p += (95-p)*0.045;
        var f=el('__sfov_fill'), pctEl=el('__sfov_pct');
        if(f){ f.style.animation='none'; f.style.transition='width .3s ease'; f.style.width=p+'%'; }
        if(pctEl) pctEl.textContent=(p|0)+'%';
      },350);
    }catch(e){}
  };
  // Render inicial (modo 'off') → el botón Cancelar existe desde el arranque.
  try{ render(); document.addEventListener('DOMContentLoaded',function(){ try{render();}catch(e){} }); }catch(e){}
  try{
    window.addEventListener('keydown',function(e){
      if(window.__sfOv&&window.__sfOv.mode==='block'){
        var k=e.key||'';
        if(k==='F5'||((e.ctrlKey||e.metaKey)&&(k==='r'||k==='R'))){ e.preventDefault(); e.stopPropagation(); }
      }
    },true);
  }catch(e){}
})();
"#;

/// (v7) Driver del navegador de HOSTERS. Decide el modo del overlay por página:
///   · captcha presente (Turnstile/reCAPTCHA) → `'hint'` "resolvé el captcha"
///     (NO bloquea, para que el usuario lo resuelva).
///   · `modsfire.com` sin captcha → `'block'` (ahí corre el auto-avance).
///   · resto (get.php, mixdrop…) → `'off'` (el usuario interactúa normal).
/// Además, sólo en `modsfire.com`, auto-avanza el botón de descarga cuando tiene
/// `href` real (JS aditivo; jamás se saltea un captcha).
const HOSTER_DRIVER_JS: &str = r#"
(function(){
  function host(){ try{return (location.hostname||'').toLowerCase();}catch(e){return '';} }
  // Sólo captchas VISIBLES cuentan (Turnstile / reCAPTCHA v2 con checkbox).
  // reCAPTCHA v3 es INVISIBLE (score-based, solo el badge) → NO es algo que
  // resolver → NO debe mostrar "resolvé el captcha" (bug reportado en mixdrop).
  function visibleCaptcha(){
    try{
      var sel='.cf-turnstile,iframe[src*="challenges.cloudflare.com"],'
        +'.g-recaptcha:not(.grecaptcha-badge),'
        +'iframe[src*="/recaptcha/api2/anchor"],iframe[src*="/recaptcha/enterprise/anchor"]';
      var els=document.querySelectorAll(sel);
      for(var i=0;i<els.length;i++){
        var r=els[i].getBoundingClientRect();
        if(r && r.width>60 && r.height>40) return true;
      }
    }catch(e){}
    return false;
  }
  function isMixdrop(){ var h=host();
    return h.indexOf('mixdrop')!==-1||h.indexOf('xdrop')!==-1||h.indexOf('mdrop')!==-1; }
  function drive(){
    try{
      // (v7) Fallo terminal → el overlay ya muestra el mensaje; no lo pisamos.
      if(window.__sfStop){ return; }
      // (v7 fix) El overlay se bloquea SOLO cuando la descarga ARRANCÓ de verdad
      // (on_download Requested puso __sfDownloading). NO bloqueamos por estar en
      // modsfire.com: eso tapaba la página y frenaba el auto-avance / los clics
      // (regresión reportada — modsfire dejó de auto-arrancar). Durante el
      // auto-avance la página queda interactiva; el overlay aparece al bajar.
      if(window.__sfDownloading){ window.__sfSet('block'); return; }
      if(visibleCaptcha()){ window.__sfSet('hint', window.__sfL.cap); }
      else if(isMixdrop()){ window.__sfSet('hint', window.__sfL.click); }
      else { window.__sfSet('off'); }
    }catch(e){}
  }
  try{ drive(); document.addEventListener('DOMContentLoaded',drive); setInterval(drive,400); }catch(e){}
  // (v7 fix, verificado en vivo) modsfire QUITA el href del botón al cargar y
  // sólo lo restaura 5 s DESPUÉS de un clic. El selector viejo
  // 'a.download-button[href]' NUNCA matcheaba en la página 1 → deadlock (sin
  // href no clicaba, sin clic no aparecía el href). Fix: selector SIN [href] +
  // un clic para arrancar la cuenta atrás (gateado por el texto "Generating…"
  // para no reclicar). Página 2 ya trae href → clic directo.
  if(host().indexOf('modsfire.com')!==-1){
    var done=false;
    function tick(){
      if(done) return;
      if(window.__sfStop){ done=true; return; }
      var b=document.querySelector('a.download-button');
      if(!b) return;
      var h=b.getAttribute('href');
      if(h && h!=='#'){
        done=true;
        try{ b.click(); }catch(e){ try{ location.href=b.href; }catch(e2){} }
        return;
      }
      if((b.textContent||'').indexOf('Generating')===-1){
        try{ b.click(); }catch(e){}
      }
    }
    var n=0,iv=setInterval(function(){ n++; tick(); if(done||n>60) clearInterval(iv); },500);
    try{ document.addEventListener('DOMContentLoaded',tick); }catch(e){}
  }
})();
"#;

/// (v7) Navegador embebido GENÉRICO para descargar directo de hosters
/// (modsfire, mixdrop, `dl.sceneryaddons.org/get.php?hoster=…`) SIN salir de
/// SimFleet. Reemplaza al viejo "abrir en el navegador del sistema".
///
/// Monta una ventana WebView2 propia con:
///   · ad blocker a nivel de red (mismos dominios que liveries + las redes de
///     popunders de los hosters) → sin anuncios ni "botones de descarga falsos",
///   · bloqueo de popups/popunders (`install_popup_blocker` + `window.open`
///     anulado en el init script),
///   · captura de la descarga a `%TEMP%\simfleet-mirror-dl`.
///
/// Al terminar la descarga emite `embedded-download://finished` con el path;
/// el instalador de drop (frontend) la recoge, la instala en la Community
/// correcta y borra el temporal. La ventana se cierra sola al capturar el
/// archivo. `modsfire` baja casi solo; `mixdrop`/`get.php` pueden pedir un
/// clic o captcha que el usuario resuelve en la ventana.
/// (v7) Prefijo JS que le pasa el idioma de la app al overlay. Sólo 'es'/'en';
/// cualquier otro ('auto'/'') → el overlay cae a `navigator.language`.
fn lang_prefix(lang: &str) -> String {
    let l = if lang == "en" || lang == "es" { lang } else { "" };
    format!("window.__sfLang={l:?};")
}

pub(crate) fn open_hoster_download(
    app: &tauri::AppHandle,
    win_label: String,
    url: String,
    title: String,
    lang: &str,
    parts: Vec<String>,
    password: Option<String>,
) -> Result<(), String> {
    tracing::info!(
        target: "hoster",
        "open_hoster_download → {} ({} parte(s) extra, pw={})",
        url, parts.len(), password.is_some()
    );
    let parsed: tauri::Url = url.parse().map_err(|e| crate::tr!("livery.invalidUrl", e = e))?;

    // Ya existe esa ventana → navegar + enfocar (evita duplicados).
    if let Some(w) = app.get_webview_window(&win_label) {
        let _ = w.navigate(parsed);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    // (v7) Lista ORDENADA de URLs: parte 1 (la que abre la ventana) + el resto
    // (multi-parte de Skybound). Para el caso normal `total == 1`.
    let urls: Vec<String> = {
        let mut v = Vec::with_capacity(parts.len() + 1);
        v.push(url.clone());
        v.extend(parts);
        v
    };
    let total = urls.len();
    let multipart = total > 1;

    // Carpeta temporal POR JOB (subcarpeta con el label de la ventana): aísla
    // las partes de otras descargas concurrentes para poder unirlas sin mezclar.
    // Sigue conteniendo "simfleet-mirror-dl" → el frontend la reconoce y la
    // purga de arranque la borra recursivamente.
    let dl_dir = std::env::temp_dir().join(MIRROR_DL_DIR).join(&win_label);
    let _ = std::fs::create_dir_all(&dl_dir);
    let dl_dir_cb = dl_dir.clone();

    let app_emit = app.clone();
    let label_close = win_label.clone();
    // Para el botón ✕ del overlay (cancelar descarga + cerrar la ventana).
    let app_cancel = app.clone();
    let label_cancel = win_label.clone();
    let dl_dir_cancel = dl_dir.clone();

    // Estado del progreso multi-parte: archivos ya bajados + índice de la parte
    // en curso. Se comparten con el callback de descarga (todo en el hilo de UI,
    // pero el `Mutex` mantiene el préstamo simple entre invocaciones del `Fn`).
    let mp_files: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let mp_idx: Arc<Mutex<usize>> = Arc::new(Mutex::new(0usize));
    let urls_cb = urls.clone();

    // Overlay de SimFleet + driver de hosters (bloqueo/auto-avance/captcha),
    // con el idioma de la app inyectado.
    let hoster_init = format!("{}\n{OVERLAY_JS}\n{HOSTER_DRIVER_JS}", lang_prefix(lang));

    let mut builder = WebviewWindowBuilder::new(app, &win_label, WebviewUrl::External(parsed))
        .initialization_script(hoster_init.as_str())
        .title(title)
        // (v7.1) Ventana FLOTANTE más chica que la principal. Es OWNED de la
        // ventana `main` (se setea abajo, antes de `build`) → queda SIEMPRE
        // encima de SimFleet pero NO encima del resto de apps de Windows, así el
        // usuario puede seguir usando su PC mientras la descarga corre. (Antes
        // era `always_on_top` = se ponía sobre TODO y bloqueaba el monitor.)
        .inner_size(1024.0, 720.0)
        .min_inner_size(820.0, 560.0)
        .center()
        .focused(true)
        // (v7) SIN decoraciones → sin botones de minimizar/maximizar/cerrar del
        // sistema; se maneja como un popout de SimFleet (se cierra con el ✕ del
        // overlay o solo al terminar).
        .decorations(false)
        .user_agent(CHROME_UA)
        // Mismo fix del render en blanco que en liveries.
        .disable_drag_drop_handler()
        // (v7) Intercepta el centinela del botón ✕: cancela la descarga (borra
        // el temporal) y cierra la ventana. Además loguea la navegación.
        .on_navigation(move |url| {
            let us = url.as_str();
            if us.contains("cancel.simfleet.invalid") {
                tracing::info!(target: "hoster", "cancelar descarga (✕) → cierro ventana");
                let _ = std::fs::remove_dir_all(&dl_dir_cancel);
                if let Some(w) = app_cancel.get_webview_window(&label_cancel) {
                    let _ = w.close();
                }
                return false;
            }
            tracing::info!(target: "hoster", "webview navega → {}", us);
            true
        })
        .on_page_load(|_w, payload| {
            tracing::info!(
                target: "hoster",
                "webview page_load {:?} → {}", payload.event(), payload.url()
            );
        })
        .on_download(move |webview, event| {
            use tauri::webview::DownloadEvent;
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let idx = *mp_idx.lock().unwrap();
                    let part_label = if multipart {
                        crate::tr!("livery.partOf", i = idx + 1, total = total)
                    } else {
                        String::new()
                    };
                    // (v7) La descarga ARRANCÓ → bloqueamos la pantalla + label.
                    let js = format!(
                        "window.__sfPart={part_label:?};window.__sfDownloading=true;\
                         if(window.__sfSet)window.__sfSet('block');"
                    );
                    let _ = webview.eval(&js);

                    if is_direct_download_url(url.as_str()) {
                        // (v7) URL DIRECTA (R2 presignada) → la baja `reqwest`
                        // (reanuda + reintenta): el downloader de WebView2 se
                        // cuelga en archivos grandes (se cortaba a ~1.5 GB).
                        // CANCELAMOS la descarga de WebView2 (return false) y la
                        // hace la tarea async, que al terminar avanza/une.
                        // Multi-parte: nombre secuencial (unimos por firma/orden).
                        // 1 archivo: conservamos nombre/extensión reales (así un
                        // instalador .exe/.msi suelto no se pierde en el join).
                        let target = if multipart {
                            dl_dir_cb.join(format!("sf-part-{idx:03}.bin"))
                        } else {
                            dl_dir_cb.join(download_filename(&url))
                        };
                        tracing::info!(
                            target: "hoster",
                            "descarga DIRECTA [{}/{}] por reqwest: {} → {}",
                            idx + 1, total, url, target.display()
                        );
                        let _ = app_emit
                            .emit("embedded-download://started", crate::tr!("livery.part", i = idx + 1));
                        let url_s = url.to_string();
                        let app2 = app_emit.clone();
                        let label2 = label_close.clone();
                        let files2 = mp_files.clone();
                        let idx2 = mp_idx.clone();
                        let urls2 = urls_cb.clone();
                        let dir2 = dl_dir_cb.clone();
                        let pl = part_label.clone();
                        let tgt = target.clone();
                        let pw2 = password.clone();
                        tauri::async_runtime::spawn(async move {
                            match fetch_direct_to_file(url_s, tgt.clone(), app2.clone(), label2.clone(), pl)
                                .await
                            {
                                Ok(()) => advance_after_part(
                                    tgt, &files2, &idx2, &urls2, &dir2, &app2, &label2,
                                    pw2.as_deref(),
                                ),
                                Err(e) => {
                                    tracing::error!(target: "hoster", "descarga directa falló: {e}");
                                    show_hoster_failure(
                                        &app2, &label2,
                                        &crate::tr!("livery.downloadFailedConn"),
                                    );
                                }
                            }
                        });
                        return false;
                    }

                    // WebView2 (flujo get.php de SceneryAddons/Simplaza): descarga
                    // nativa como siempre.
                    let fname = if multipart {
                        format!("sf-part-{idx:03}.bin")
                    } else {
                        download_filename(&url)
                    };
                    let target = dl_dir_cb.join(&fname);
                    tracing::info!(
                        target: "hoster",
                        "descarga interceptada [{}/{}]: {} → {}",
                        idx + 1, total, url, target.display()
                    );
                    let _ = app_emit.emit("embedded-download://started", fname.clone());
                    *destination = target;
                }
                DownloadEvent::Finished { url, path, success } => {
                    tracing::info!(
                        target: "hoster",
                        "descarga terminada ok={} url={} path={:?}", success, url, path
                    );
                    // Las URLs directas (R2) las maneja la tarea reqwest —
                    // ignoramos su Finished (WebView2 la canceló).
                    if is_direct_download_url(url.as_str()) {
                        return true;
                    }
                    if !success {
                        // No dejamos la ventana colgada: mostramos el fallo. Los
                        // archivos grandes ya van por reqwest, así que esto es
                        // raro (sólo el flujo get.php de un archivo chico).
                        tracing::warn!(target: "hoster", "descarga WebView2 falló (ok=false)");
                        show_hoster_failure(
                            &app_emit, &label_close,
                            &crate::tr!("livery.downloadFailed"),
                        );
                        return true;
                    }
                    let Some(p) = path else {
                        return true;
                    };
                    if multipart {
                        advance_after_part(
                            p, &mp_files, &mp_idx, &urls_cb, &dl_dir_cb, &app_emit, &label_close,
                            password.as_deref(),
                        );
                    } else {
                        // Caso normal (1 archivo, flujo get.php): corrige la
                        // extensión por FIRMA (`d.php.zip` que en realidad es .rar),
                        // muestra "procesando…" en el overlay y emite {path,
                        // window, password} para instalar + cerrar la ventana.
                        let fixed = fix_archive_extension(&p);
                        if let Some(w) = app_emit.get_webview_window(&label_close) {
                            let _ = w.eval("if(window.__sfProcessing)window.__sfProcessing();");
                        }
                        let _ = app_emit.emit(
                            "embedded-download://finished",
                            serde_json::json!({
                                "path": fixed.to_string_lossy(),
                                "window": label_close.clone(),
                                "password": password.as_deref(),
                            }),
                        );
                    }
                }
                _ => {}
            }
            true
        });
    // (v7) uBlock Origin Lite: si la extensión está lista, la cargamos en esta
    // ventana → mata los anuncios de los hosters (mixdrop, etc.) con EasyList,
    // mucho mejor que la lista de dominios a mano.
    if let Some(dir) = UBOL_DIR.get() {
        builder = builder.browser_extensions_enabled(true).extensions_path(dir);
    }
    // (v7.1) OWNED de la ventana principal → encima de SimFleet pero NO encima
    // del resto de apps de Windows (el usuario puede usar su PC mientras baja).
    if let Some(main_win) = app.get_webview_window("main") {
        builder = builder.parent(&main_win).map_err(|e| e.to_string())?;
    }
    let win = builder.build().map_err(|e| e.to_string())?;

    // Ad blocker (red) + bloqueo de popups NATIVO + progreso real de descarga.
    // Best-effort; si falla, la descarga sigue funcionando, sólo que con más
    // anuncios / sin barra de progreso.
    #[cfg(target_os = "windows")]
    {
        install_web_guards(&win, true);
        install_download_progress(&win);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = &win;

    tracing::info!(target: "hoster", "navegador de descarga abierto → {}", url);
    Ok(())
}

/// (v6.2.66 / v7) Guardas de red + ventanas para una ventana WebView2, en UN
/// solo `with_webview` (evita registrar dos closures COM por separado).
///
///   · **Ad blocker (siempre)**: responde 403 a los dominios de anuncios/
///     tracking/popunders conocidos (`is_ad_url`) + el endpoint de anuncios de
///     flightsim.to. NO toca el DOM (eso rompía la hidratación); corta la
///     descarga del anuncio de raíz — así también los "botones de descarga
///     falsos" y los scripts de popunder ni siquiera cargan.
///   · **Popup blocker (sólo si `block_new_windows`)**: cancela SIEMPRE la
///     apertura de ventanas nuevas (`window.open`, `target="_blank"`, los
///     popunders de los hosters). Si el destino NO es un anuncio, lo navega en
///     la MISMA ventana (una descarga legítima que abría en pestaña). Es
///     NATIVO → invisible para el JS de la página, así que no dispara la
///     detección anti-tamper de modsfire/rapidgator (que dejaba la página en
///     blanco cuando se pisaba `window.open` desde JS). Liveries lo dejan en
///     `false` (comportamiento idéntico al de siempre).
#[cfg(target_os = "windows")]
fn install_web_guards(win: &tauri::WebviewWindow, block_new_windows: bool) {
    let _ = win.with_webview(move |webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Settings3, ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        };
        use webview2_com::{NewWindowRequestedEventHandler, WebResourceRequestedEventHandler};
        use windows_core::{w, Interface, PWSTR};
        // SAFETY: API COM de WebView2 en el hilo de UI (mismo patrón que en
        // `run()` para desactivar los accelerator keys).
        unsafe {
            let controller = webview.controller();
            let Ok(core) = controller.CoreWebView2() else {
                return;
            };
            // ── (v7) Ventana de descarga de hosters: desactivar F5/Ctrl+R y el
            // menú contextual → un refresh/back NO interrumpe el auto-avance ni
            // el token de descarga (que es de un solo uso). Sólo en hosters
            // (`block_new_windows`), no en liveries (ahí el usuario navega).
            if block_new_windows {
                if let Ok(settings) = core.Settings() {
                    let _ = settings.SetAreDefaultContextMenusEnabled(false);
                    if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
                        let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
                    }
                }
            }
            // ── Ad blocker ────────────────────────────────────────────────
            let _ = core
                .AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
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

            // ── Popup blocker (opt-in) ────────────────────────────────────
            // Cancela SÓLO las ventanas nuevas hacia dominios de anuncios
            // (popunders). Las ventanas NO-anuncio se dejan pasar tal cual: si
            // una descarga legítima abre en pestaña nueva, WebView2 la maneja y
            // `on_download` la captura igual. (Antes navegábamos la ventana
            // principal al destino, pero eso perdía el `referer`/sesión y
            // rompía descargas token-gated como la de modsfire → "no pasa
            // nada". Los popunders ya se matan además a nivel de red porque sus
            // scripts (hai8g, xadsmart, …) están en la lista de bloqueo.)
            if block_new_windows {
                let popup = NewWindowRequestedEventHandler::create(Box::new(move |_wv, args| {
                    if let Some(args) = args {
                        let mut pw = PWSTR::null();
                        if args.Uri(&mut pw).is_ok() {
                            let uri = webview2_com::take_pwstr(pw);
                            if is_ad_url(&uri) {
                                let _ = args.SetHandled(true.into());
                            }
                        }
                    }
                    Ok(())
                }));
                let mut token2 = 0i64;
                let _ = core.add_NewWindowRequested(&popup, &mut token2);
            }
        }
    });
}

/// (v7) Progreso REAL de la descarga de WebView2 → lo empuja al overlay
/// (`window.__sfProgress(pct, "velocidad")`). Se engancha al evento nativo
/// `DownloadStarting` (que wry ya usa para capturar la descarga) SIN tocar
/// `Handled`/`ResultFilePath`/`Cancel` — sólo lee la operación y su
/// `BytesReceivedChanged`. Todo corre en el hilo de UI, así que la velocidad
/// se calcula con estado capturado (Δbytes/Δt) sin locks. Verificado contra
/// webview2-com 0.38.2.
#[cfg(target_os = "windows")]
fn install_download_progress(win: &tauri::WebviewWindow) {
    let _ = win.with_webview(move |webview| {
        use std::time::Instant;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2, ICoreWebView2ExecuteScriptCompletedHandler, ICoreWebView2_4,
        };
        use webview2_com::{BytesReceivedChangedEventHandler, DownloadStartingEventHandler};
        use windows_core::{Interface, HSTRING};
        // SAFETY: API COM de WebView2 en el hilo de UI (mismo patrón que
        // `install_web_guards`).
        unsafe {
            let controller = webview.controller();
            let Ok(core) = controller.CoreWebView2() else {
                return;
            };
            let Ok(wv4) = core.cast::<ICoreWebView2_4>() else {
                return;
            };
            // Clon del core para llamar ExecuteScript desde el callback de bytes
            // (robusto ante `sender == None`).
            let core_cb: ICoreWebView2 = core.clone();
            let handler = DownloadStartingEventHandler::create(Box::new(move |_wv, args| {
                let Some(args) = args else { return Ok(()) };
                let op = args.DownloadOperation()?;
                let core_js = core_cb.clone();
                // Estado de velocidad — el closure FnMut lo muta entre ticks.
                let mut last_bytes: i64 = 0;
                let mut last_t = Instant::now();
                let on_bytes =
                    BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
                        let Some(op) = sender else { return Ok(()) };
                        let mut received: i64 = 0;
                        op.BytesReceived(&mut received)?;
                        let mut total: i64 = 0;
                        let _ = op.TotalBytesToReceive(&mut total); // -1/0 si no hay Content-Length
                                                                    // Throttle ~4 Hz: no empujamos en cada buffer.
                        let now = Instant::now();
                        let dt = now.duration_since(last_t).as_secs_f64();
                        if dt < 0.25 {
                            return Ok(());
                        }
                        let bps = (received - last_bytes) as f64 / dt;
                        last_bytes = received;
                        last_t = now;
                        let pct: f64 = if total > 0 {
                            (received as f64 / total as f64) * 100.0
                        } else {
                            -1.0 // indeterminado
                        };
                        let eta = if total > 0 && bps > 1.0 {
                            fmt_eta((total - received) as f64 / bps)
                        } else {
                            String::new()
                        };
                        let js = format!(
                            "window.__sfProgress&&window.__sfProgress({:.1},'{}','{}')",
                            pct,
                            fmt_speed(bps),
                            eta
                        );
                        let _ = core_js.ExecuteScript(
                            &HSTRING::from(js),
                            None::<&ICoreWebView2ExecuteScriptCompletedHandler>,
                        );
                        Ok(())
                    }));
                let mut tok: i64 = 0;
                let _ = op.add_BytesReceivedChanged(&on_bytes, &mut tok);
                Ok(())
            }));
            let mut token: i64 = 0;
            let _ = wv4.add_DownloadStarting(&handler, &mut token);
        }
    });
}

/// Formatea una velocidad en B/s → "12.3 MB/s" / "540 KB/s" / "60 B/s".
fn fmt_speed(bytes_per_sec: f64) -> String {
    let b = bytes_per_sec.max(0.0);
    if b >= 1_048_576.0 {
        format!("{:.1} MB/s", b / 1_048_576.0)
    } else if b >= 1_024.0 {
        format!("{:.0} KB/s", b / 1_024.0)
    } else {
        format!("{:.0} B/s", b)
    }
}

/// Formatea segundos restantes → "ETA 1h 5m" / "ETA 2m 10s" / "ETA 45s".
fn fmt_eta(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("ETA {}h {}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("ETA {}m {}s", s / 60, s % 60)
    } else {
        format!("ETA {}s", s)
    }
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
        // (v7) OJO: `googletagmanager.com` (gtag) y `google-analytics.com` NO
        // se bloquean — son ANALYTICS, no anuncios, y algunos hosters
        // (modsfire: `alt095.js`) llaman `gtag()` sin guardas y REVIENTAN si no
        // está → el botón de descarga queda muerto y "no pasa nada" al clicar.
        // Bloquearlos no quita ningún anuncio visible; sí rompe las descargas.
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
        // (v7) Redes de popunders / anuncios de los HOSTERS de archivos
        // (modsfire, mixdrop y similares). Bloquearlas a nivel de red mata
        // los popunders y los "botones de descarga falsos" de raíz — el
        // script del anuncio nunca llega a cargar. NINGUNA es un captcha:
        // reCAPTCHA (google.com/recaptcha, gstatic) y Cloudflare Turnstile
        // (challenges.cloudflare.com) NO están aquí, así se pueden resolver.
        "hai8g.com",
        "crn77.com",
        "xadsmart.com",
        "greatdexchange.com",
        "snigelweb.com",
        "playwire.com",
        "intergient.com",
        "popads.net",
        "popcash.net",
        "propellerads.com",
        "hilltopads.com",
        "hilltopads.net",
        "adsterra.com",
        "clickadu.com",
        "exoclick.com",
        "juicyads.com",
        "mgid.com",
        "poptm.com",
        "onclickalgo.com",
        "highperformanceformat.com",
        "profitabledisplaynetwork.com",
        "adnium.com",
        "trafficjunky.com",
        "adcash.com",
        // (v7) Dominios de ENTREGA (delivery/serving) — la clave que faltaba:
        // mixdrop/hosters NO sirven los popunders desde el dominio de MARCA
        // (exoclick.com) sino desde estos. Bloquear la marca no bloqueaba el
        // anuncio. (Verificado en la investigación de adblock.)
        "realsrv.com",
        "exdynsrv.com",
        "exosrv.com",
        "exv6.com",
        "magsrv.com",
        "orbsrv.com",
        "optnx.com",
        "onclckds.com",
        "propu.net",
        "propellerpops.com",
        "propellerclick.com",
        "monetag.com",
        "ad-maven.com",
        "admaven.com",
        "pxlad.io",
        "grptaam.com",
        "tsyndicate.com",
        "trafficstars.com",
        "twinred.com",
        "clickadilla.com",
        "bidvertiser.com",
        "bdtcdn.com",
        "revenuehits.com",
        "galaksion.com",
        "richads.com",
        "yllix.com",
        "adbutler.com",
        "adskeeper.com",
        "coinzilla.com",
        "czilladx.com",
        "cointraffic.io",
        "media.net",
        "revcontent.com",
        "adspyglass.com",
        // (v7) Vistos en vivo en mixdrop (los estables; los aleatorios/rotativos
        // los cubre uBlock Origin Lite con EasyList).
        "network-n.com",
        "work-n.com",
        "btloader.com",
        "hadronid.net",
        "bounceexchange.com",
        "snigelweb.com",
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

/// (v7) Corrige la extensión de un archivo descargado según sus BYTES MÁGICOS
/// (la firma del contenido), no según la URL. Los hosters sirven el archivo
/// desde endpoints sin nombre real (modsfire: `d.php?p=<base64>`), así que
/// `download_filename` lo guarda como `d.php.zip` aunque el contenido sea un
/// `.rar` → el instalador intenta abrir un rar como zip y falla ("Could not
/// find EOCD"). Leemos la firma y renombramos a la extensión correcta. Si la
/// firma no es de un archivo conocido (o falla el rename), se deja igual.
fn fix_archive_extension(path: &Path) -> PathBuf {
    use std::io::Read;
    let mut buf = [0u8; 8];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    if read < 4 {
        return path.to_path_buf();
    }
    let ext = if buf.starts_with(b"PK") {
        "zip"
    } else if buf.starts_with(b"Rar!") {
        "rar"
    } else if buf.starts_with(&[0x37, 0x7A, 0xBC, 0xAF]) {
        "7z"
    } else {
        return path.to_path_buf(); // firma desconocida → no tocar
    };
    let cur = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if cur == ext {
        return path.to_path_buf();
    }
    let new_path = path.with_extension(ext);
    match std::fs::rename(path, &new_path) {
        Ok(()) => {
            tracing::info!(
                target: "hoster",
                "extensión corregida por firma: {} → {}",
                path.display(), new_path.display()
            );
            new_path
        }
        Err(e) => {
            tracing::warn!(target: "hoster", "no se pudo renombrar {}: {e}", path.display());
            path.to_path_buf()
        }
    }
}

/// (v7) Une las partes de un archivo multi-volumen (Skybound "Part 1/2/…") y
/// devuelve la ruta lista para instalar. `files` viene en orden de descarga
/// (parte 1..N). Detecta el tipo por la firma de la parte 1:
///   · **RAR5** (TODAS las partes empiezan con `Rar!`): es un set de volúmenes.
///     Renombra a `simfleet-addon.partN.rar` — al extraer la parte 1, `unrar`
///     encuentra y une los hermanos del mismo directorio solo.
///   · **resto** (7z/zip byte-split `.001/.002`, o RAR viejo): concatena las
///     partes EN ORDEN en un único archivo con la extensión correcta.
/// Con una sola parte, corrige la extensión por firma (igual que el single-file).
fn join_parts(files: &[PathBuf], dir: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind, Read};
    if files.is_empty() {
        return Err(Error::new(ErrorKind::NotFound, "sin partes"));
    }
    if files.len() == 1 {
        return Ok(fix_archive_extension(&files[0]));
    }
    // Firma (primeros 8 bytes) de cada parte.
    let mut sigs: Vec<[u8; 8]> = Vec::with_capacity(files.len());
    for f in files {
        let mut buf = [0u8; 8];
        let n = std::fs::File::open(f)
            .and_then(|mut fh| fh.read(&mut buf))
            .unwrap_or(0);
        if n < 4 {
            return Err(Error::new(ErrorKind::InvalidData, "parte vacía/ilegible"));
        }
        sigs.push(buf);
    }
    let is_rar = |s: &[u8; 8]| s.starts_with(b"Rar!");

    // Caso 1: set de volúmenes RAR5 (todas las partes firmadas `Rar!`).
    if is_rar(&sigs[0]) && sigs.iter().all(is_rar) {
        let mut first = None;
        for (k, f) in files.iter().enumerate() {
            let target = dir.join(format!("simfleet-addon.part{}.rar", k + 1));
            std::fs::rename(f, &target)?;
            if k == 0 {
                first = Some(target);
            }
        }
        let first = first.unwrap();
        tracing::info!(
            target: "hoster",
            "multi-parte RAR: {} volúmenes → {}", files.len(), first.display()
        );
        return Ok(first);
    }

    // Caso 2: byte-split (7z/zip `.001/.002`, o RAR viejo) → concatenar en orden.
    let ext = if is_rar(&sigs[0]) {
        "rar"
    } else if sigs[0].starts_with(&[0x37, 0x7A, 0xBC, 0xAF]) {
        "7z"
    } else if sigs[0].starts_with(b"PK") {
        "zip"
    } else {
        "bin"
    };
    let out = dir.join(format!("simfleet-addon.{ext}"));
    {
        let mut w = std::fs::File::create(&out)?;
        for f in files {
            let mut r = std::fs::File::open(f)?;
            std::io::copy(&mut r, &mut w)?;
        }
    }
    // Borra las partes crudas ya concatenadas.
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    tracing::info!(
        target: "hoster",
        "multi-parte concatenado: {} partes → {}", files.len(), out.display()
    );
    if ext == "bin" {
        // Firma desconocida → intenta corregir por los bytes del resultado.
        return Ok(fix_archive_extension(&out));
    }
    Ok(out)
}

/// (v7) ¿La URL resuelta es un enlace DIRECTO (presignado) a un CDN, que
/// podemos bajar por HTTP sin cookies del navegador? Los hosters (modsfire)
/// redirigen su `d.php` a una URL presignada de Cloudflare R2
/// (`…r2.cloudflarestorage.com/…?X-Amz-Signature=…`). Esas se bajan mucho más
/// fiable con `reqwest` (reanuda + reintenta) que con el downloader de WebView2,
/// que se cuelga en archivos grandes (bug reportado: se cortaba a ~1.5 GB).
/// El flujo `get.php` de SceneryAddons/Simplaza NO casa aquí → sigue por
/// WebView2 (funciona para sus archivos, más chicos).
fn is_direct_download_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("x-amz-signature=") || u.contains("r2.cloudflarestorage.com")
}

/// (v7) Avanza el flujo tras completar UNA parte (venga de reqwest o de
/// WebView2): guarda el archivo, y o navega a la siguiente parte (el hoster
/// re-resuelve su URL) o une todas y manda a instalar. Sirve igual para el caso
/// de 1 sola parte (`urls.len()==1` → une 1 archivo → instala).
fn advance_after_part(
    target: PathBuf,
    files: &Arc<Mutex<Vec<PathBuf>>>,
    idx: &Arc<Mutex<usize>>,
    urls: &[String],
    dir: &Path,
    app: &tauri::AppHandle,
    win_label: &str,
    password: Option<&str>,
) {
    files.lock().unwrap().push(target);
    let done = {
        let mut i = idx.lock().unwrap();
        *i += 1;
        *i
    };
    if done < urls.len() {
        tracing::info!(target: "hoster", "parte {}/{} lista — voy por la siguiente", done, urls.len());
        if let Some(w) = app.get_webview_window(win_label) {
            let _ = w.eval(
                "window.__sfDownloading=false;window.__sfPart='';\
                 if(window.__sfSet)window.__sfSet('off');",
            );
            if let Ok(u) = urls[done].parse::<tauri::Url>() {
                let _ = w.navigate(u);
            }
        }
    } else {
        let files_v = files.lock().unwrap().clone();
        match join_parts(&files_v, dir) {
            Ok(out) => {
                // (v7) Bajó todo → mostramos "procesando archivos…" en el overlay
                // (el WebView tapa a SimFleet) y mandamos a instalar con la clave.
                if let Some(w) = app.get_webview_window(win_label) {
                    let _ = w.eval("if(window.__sfProcessing)window.__sfProcessing();");
                }
                let _ = app.emit(
                    "embedded-download://finished",
                    serde_json::json!({
                        "path": out.to_string_lossy(),
                        "window": win_label,
                        "password": password,
                    }),
                );
            }
            Err(e) => {
                // NO instalamos un fragmento: unir falló → avisamos y NO emitimos
                // `finished` (si no, el instalador tomaría la parte 1 como el
                // addon completo).
                tracing::error!(target: "hoster", "no se pudieron unir {} partes: {e}", files_v.len());
                show_hoster_failure(
                    app, win_label,
                    &crate::tr!("livery.joinPartsFailed"),
                );
            }
        }
    }
}

/// (v7) Muestra un fallo terminal en el overlay del hoster: desbloquea, corta el
/// auto-avance del driver y deja el mensaje + botón Cancelar.
fn show_hoster_failure(app: &tauri::AppHandle, win_label: &str, msg: &str) {
    if let Some(w) = app.get_webview_window(win_label) {
        let js = format!(
            "window.__sfStop=true;window.__sfDownloading=false;\
             if(window.__sfSet)window.__sfSet('hint',{msg:?});"
        );
        let _ = w.eval(&js);
    }
}

/// (v7) Baja una URL DIRECTA (presignada) a un archivo con `reqwest`, con
/// **reanudación** (HTTP Range) y **reintentos**, empujando el progreso real al
/// overlay. Es el downloader fiable para archivos grandes de los hosters (el de
/// WebView2 se cuelga). Aborta si la ventana se cerró (cancelar). `part_label`
/// se muestra como "parte X de N".
async fn fetch_direct_to_file(
    url: String,
    target: PathBuf,
    app: tauri::AppHandle,
    win_label: String,
    part_label: String,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    const MAX_ATTEMPTS: u32 = 4;
    // Sin datos por este lapso → lo tratamos como stall: reintenta (reanuda) y de
    // paso chequea si el usuario cerró la ventana. Clave para NO colgarse cuando
    // el CDN acepta la conexión pero deja de mandar bytes (socket vivo, sin EOF).
    const STALL_SECS: u64 = 25;

    let client = reqwest::Client::builder()
        .user_agent(CHROME_UA)
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut last_err = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        // Cancelado (ventana cerrada) antes de (re)empezar.
        if app.get_webview_window(&win_label).is_none() {
            return Err("cancelado (ventana cerrada)".into());
        }
        // Reanuda desde lo ya bajado (si quedó algo de un intento previo).
        let have = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        let mut req = client.get(&url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        // send() acotado: si el server acepta la conexión pero nunca manda
        // headers, no colgamos para siempre.
        let resp = match tokio::time::timeout(Duration::from_secs(45), req.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                last_err = format!("conexión: {e}");
                tokio::time::sleep(Duration::from_millis(800)).await;
                continue;
            }
            Err(_) => {
                last_err = "timeout de conexión".into();
                continue;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            // 403/410 → el enlace presignado expiró (no sirve reintentarlo).
            if matches!(status.as_u16(), 403 | 410) {
                return Err(format!("el enlace de descarga expiró (HTTP {status})"));
            }
            last_err = format!("HTTP {status}");
            tokio::time::sleep(Duration::from_millis(800)).await;
            continue;
        }
        let resuming = status.as_u16() == 206;
        let base = if resuming { have } else { 0 };
        let total = resp.content_length().map(|l| base + l).unwrap_or(0);

        let file = if resuming && have > 0 {
            tokio::fs::OpenOptions::new().append(true).open(&target).await
        } else {
            tokio::fs::File::create(&target).await
        };
        let mut file = file.map_err(|e| format!("archivo: {e}"))?;

        let mut downloaded = base;
        let mut last_bytes = downloaded;
        let mut last_t = std::time::Instant::now();
        let mut stream = resp.bytes_stream();
        let mut mid_fail = false;
        loop {
            // Cada chunk con tope de tiempo: si no llega nada en STALL_SECS, o es
            // un stall (reintenta/reanuda) o el usuario canceló (ventana cerrada).
            let chunk = match tokio::time::timeout(Duration::from_secs(STALL_SECS), stream.next()).await {
                Err(_) => {
                    if app.get_webview_window(&win_label).is_none() {
                        return Err("cancelado (ventana cerrada)".into());
                    }
                    last_err = format!("sin datos {STALL_SECS}s (stall)");
                    mid_fail = true;
                    break;
                }
                Ok(None) => break, // EOF
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(e))) => {
                    last_err = format!("stream: {e}");
                    mid_fail = true;
                    break;
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                return Err(format!("escritura: {e}"));
            }
            downloaded += chunk.len() as u64;
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_t).as_secs_f64();
            if dt >= 0.3 {
                // ¿Ventana cerrada (el usuario canceló)? → abortar limpio.
                if app.get_webview_window(&win_label).is_none() {
                    return Err("cancelado (ventana cerrada)".into());
                }
                let bps = (downloaded - last_bytes) as f64 / dt;
                last_bytes = downloaded;
                last_t = now;
                let pct = if total > 0 {
                    (downloaded as f64 / total as f64) * 100.0
                } else {
                    -1.0
                };
                let eta = if total > 0 && downloaded <= total && bps > 1.0 {
                    fmt_eta((total - downloaded) as f64 / bps)
                } else {
                    String::new()
                };
                if let Some(w) = app.get_webview_window(&win_label) {
                    let js = format!(
                        "window.__sfPart={part_label:?};window.__sfProgress&&window.__sfProgress({pct:.1},'{}','{}')",
                        fmt_speed(bps), eta
                    );
                    let _ = w.eval(&js);
                }
            }
        }
        let _ = file.flush().await;

        // Éxito sólo si NO cortó a mitad, bajó ALGO, y (si conocemos el total)
        // llegó al total. Así un 200 vacío/truncado NO se toma por completo.
        if !mid_fail && downloaded > 0 && (total == 0 || downloaded >= total) {
            tracing::info!(
                target: "hoster",
                "reqwest bajó {} bytes → {} (intento {})", downloaded, target.display(), attempt
            );
            return Ok(());
        }
        // Cortó a mitad → reintenta (reanudará desde `downloaded`).
        tracing::warn!(
            target: "hoster",
            "descarga directa cortada ({downloaded} bytes) — reintento {attempt}/{MAX_ATTEMPTS}: {last_err}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }
    Err(format!("falló tras {MAX_ATTEMPTS} intentos: {last_err}"))
}

/// (v6.2.56) Copia una app/instalador descargado a la carpeta que ELIJA el
/// usuario — NUNCA a Community. Devuelve la ruta final.
#[tauri::command]
pub fn save_installer_to(src: String, dest_folder: String) -> Result<String, String> {
    let src = std::path::PathBuf::from(&src);
    let dest_dir = std::path::PathBuf::from(&dest_folder);
    let file_name = src
        .file_name()
        .ok_or_else(|| crate::tr!("livery.fileNoName"))?;
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

/// (v6.2.74) Compatibilidad 2020/2024 de una descarga del embebido (por su path
/// temporal → file_id → detail de flightsim.to). None si ese path no vino de una
/// descarga del embebido asociada a un archivo de flightsim.to.
#[tauri::command]
pub async fn flightsim_download_meta(
    path: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<crate::flightsim_track::FlightsimMeta>, String> {
    let file_id = state
        .livery_downloads
        .lock()
        .ok()
        .and_then(|m| m.get(&path).cloned());
    let Some(file_id) = file_id else {
        return Ok(None);
    };
    Ok(crate::flightsim_track::fetch_meta(&state.http, &file_id).await)
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
        return Err(crate::tr!("livery.downloadsFolderNotFound"));
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
