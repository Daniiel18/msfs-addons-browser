//! (23/10) Avisos de escritorio con el diseño de SimFleet.
//!
//! Antes eran notificaciones de Windows (plugin de notificaciones): con el
//! diseño de Windows y, en el entorno de pruebas, firmadas por "PowerShell".
//! Ahora son una ventana propia ("notify") en la esquina inferior derecha del
//! monitor principal: transparente, siempre encima (también del sim en
//! pantalla completa sin bordes), fuera de la barra de tareas y SIN robar el
//! foco. Hasta 3 avisos apilados; cada uno se va solo a los 7 s.
//!
//! La ventana no usa IPC (no tiene capability propia): el backend le pasa la
//! lista con `eval` y los clics llegan como navegaciones a
//! `https://sfnotice.local/{open|close}/{id}`, que se interceptan y cancelan.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "notify";
const NAV_HOST: &str = "sfnotice.local";
/// Medidas lógicas: deben coincidir con `DesktopNotices.tsx`.
const WIDTH: f64 = 360.0;
const CARD_H: f64 = 76.0;
const GAP: f64 = 8.0;
const PAD: f64 = 10.0;
const MARGIN: f64 = 12.0;
const MAX: usize = 3;
const TTL: Duration = Duration::from_secs(7);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: u64,
    pub title: String,
    pub body: Option<String>,
    /// "info" | "success" | "error".
    pub kind: String,
}

static NOTICES: Mutex<Vec<Notice>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// El último aviso pidió ir en la esquina de la ventana de SimFleet.
static ANCHOR_APP: AtomicBool = AtomicBool::new(false);

/// Muestra un aviso. Nunca falla: si la ventana no se puede crear, se loguea.
/// `in_app`: en la esquina de la ventana de SimFleet (si está a la vista).
pub fn show(app: &AppHandle, title: String, body: Option<String>, kind: Option<String>, in_app: bool) {
    ANCHOR_APP.store(in_app, Ordering::Relaxed);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let kind = match kind.as_deref() {
        Some("success") => "success",
        Some("error") => "error",
        _ => "info",
    };
    if let Ok(mut list) = NOTICES.lock() {
        list.push(Notice { id, title, body, kind: kind.into() });
        while list.len() > MAX {
            list.remove(0);
        }
    }
    render(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TTL).await;
        dismiss(&app, id);
    });
}

fn dismiss(app: &AppHandle, id: u64) {
    let changed = NOTICES
        .lock()
        .map(|mut list| {
            let before = list.len();
            list.retain(|n| n.id != id);
            list.len() != before
        })
        .unwrap_or(false);
    if changed {
        render(app);
    }
}

fn snapshot() -> Vec<Notice> {
    NOTICES.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Le pasa la lista actual a la página (si ya cargó, la pinta al instante).
fn push_list(win: &tauri::WebviewWindow) {
    let json = serde_json::to_string(&snapshot()).unwrap_or_else(|_| "[]".into());
    let _ = win.eval(format!("window.__sfNotices && window.__sfNotices({json})"));
}

/// Clic en un aviso: "open" trae SimFleet al frente; ambos lo cierran.
fn on_click(app: &AppHandle, url: &tauri::Url) {
    let mut parts = url.path().trim_matches('/').split('/');
    let action = parts.next().unwrap_or_default();
    let id = parts.next().and_then(|s| s.parse::<u64>().ok());
    if action == "open" {
        // `get_window`: la ventana principal puede tener 2 webviews (flightsim.to).
        if let Some(main) = app.get_window("main") {
            let _ = main.unminimize();
            let _ = main.show();
            let _ = main.set_focus();
        }
    }
    if let Some(id) = id {
        dismiss(app, id);
    }
}

/// Crea la ventana (una sola vez). NO desde el hilo principal: en Windows
/// crear un WebView2 ahí puede bloquearse (wry#583); `show` llega siempre
/// desde un comando async o una tarea, y los clics sólo ocurren con la
/// ventana ya creada.
fn window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    static BUILD: Mutex<()> = Mutex::new(());
    let _guard = BUILD.lock().ok()?;
    if let Some(w) = app.get_webview_window(LABEL) {
        return Some(w);
    }
    let nav_app = app.clone();
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("SimFleet")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        // Mismas opciones de entorno WebView2 que `main`/`osd` (tauri.conf.json):
        // si no coinciden, WebView2 se niega a crearla (0x8007139F).
        .browser_extensions_enabled(true)
        .inner_size(WIDTH, PAD * 2.0 + CARD_H)
        .on_navigation(move |url| {
            if url.host_str() == Some(NAV_HOST) {
                on_click(&nav_app, url);
                return false;
            }
            true
        })
        // La página acaba de cargar: pintarle lo que haya ahora.
        .on_page_load(|w, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                push_list(&w);
            }
        })
        .build();
    match built {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(target: "notify", "no se pudo crear la ventana de avisos: {e}");
            None
        }
    }
}

fn render(app: &AppHandle) {
    let count = snapshot().len();
    if count == 0 {
        if let Some(w) = app.get_webview_window(LABEL) {
            // Vaciar también la página: si no, al volver a mostrarse
            // asomaría un instante el último aviso.
            push_list(&w);
            let _ = app.run_on_main_thread(move || hide_window(&w));
        }
        return;
    }
    let Some(win) = window(app) else { return };
    push_list(&win);
    // Posición y Win32 (mostrar sin activar) en el hilo principal.
    let _ = app.run_on_main_thread(move || place_and_show(&win, count));
}

/// Esquina inferior derecha del área de trabajo del monitor principal
/// (encima de la barra de tareas), con la altura justa para `count` avisos:
/// el resto de la pantalla no queda tapado por una ventana invisible.
fn place_and_show(win: &tauri::WebviewWindow, count: usize) {
    let n = count as f64;
    let h = PAD * 2.0 + n * CARD_H + (n - 1.0).max(0.0) * GAP;
    // Rectángulo donde va (físico): la ventana de SimFleet si se pidió y está a
    // la vista; si no, el área de trabajo del monitor principal.
    let app_rect = ANCHOR_APP
        .load(Ordering::Relaxed)
        .then(|| win.app_handle().get_window("main"))
        .flatten()
        .filter(|m| m.is_visible().unwrap_or(false) && !m.is_minimized().unwrap_or(true))
        .and_then(|m| Some((m.outer_position().ok()?, m.outer_size().ok()?, m.scale_factor().ok()?)));
    let (pos, size, scale) = match app_rect {
        Some((p, s, f)) => (p, s, f),
        None => {
            let Ok(Some(mon)) = win.primary_monitor() else {
                let _ = win.show();
                return;
            };
            let area = mon.work_area();
            (area.position, area.size, mon.scale_factor())
        }
    };
    let (pw, ph) = ((WIDTH * scale).round() as i32, (h * scale).round() as i32);
    let m = (MARGIN * scale).round() as i32;
    let x = pos.x + size.width as i32 - pw - m;
    let y = pos.y + size.height as i32 - ph - m;
    let _ = win.set_size(tauri::PhysicalSize::new(pw as u32, ph as u32));
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    show_no_activate(win);
}

/// Muestra la ventana encima de todo SIN activarla: quien esté en el sim no
/// pierde el foco. `WS_EX_NOACTIVATE` además evita que un clic la active.
#[cfg(windows)]
fn show_no_activate(win: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };
    let Ok(raw) = win.hwnd() else {
        let _ = win.show();
        return;
    };
    let hwnd = HWND(raw.0);
    // SAFETY: HWND de una ventana viva; corre en el hilo de UI.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize;
        if ex != want {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Oculta con Win32: la ventana se mostró con `ShowWindow` (sin activar) y
/// Tauri la sigue creyendo oculta, así que su `hide()` no haría nada.
#[cfg(windows)]
fn hide_window(win: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    match win.hwnd() {
        // SAFETY: HWND de una ventana viva; corre en el hilo de UI.
        Ok(raw) => unsafe {
            let _ = ShowWindow(HWND(raw.0), SW_HIDE);
        },
        Err(_) => {
            let _ = win.hide();
        }
    }
}

#[cfg(not(windows))]
fn show_no_activate(win: &tauri::WebviewWindow) {
    let _ = win.show();
}

#[cfg(not(windows))]
fn hide_window(win: &tauri::WebviewWindow) {
    let _ = win.hide();
}
