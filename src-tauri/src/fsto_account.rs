//! (26/10) Actualizaciones de la CUENTA de flightsim.to ("My Downloads").
//!
//! flightsim.to sabe qué descargó el usuario y qué tiene ya versión nueva
//! (`/backend/user/my-downloads?filter=updates`). SimFleet lo lee con la
//! sesión de su propia pestaña flightsim.to: una vista OCULTA del mismo
//! perfil de WebView2 abre una página mínima del sitio (`robots.txt`) y desde
//! ahí pide los datos con `fetch`, como la propia web; la sesión nunca sale
//! del navegador, SimFleet sólo recoge el texto de la respuesta.
//!
//! Se lee al arrancar y cada 6 h; lo último visto se guarda (settings) para
//! que el mapa y la campanita lo enseñen al instante, sin esperar a la red.
//! Sólo cuenta lo INSTALADO: perfiles GSX por ICAO (su .ini en la carpeta de
//! GSX) y addons de Community que SimFleet sabe que vienen de flightsim.to.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{Emitter, Manager};

const LABEL: &str = "fsto-account";
const HOME: &str = "https://flightsim.to/robots.txt";
const CACHE_KEY: &str = "fsto_account_updates_v1";
const DISMISS_KEY: &str = "fsto_account_dismissed_v1";
/// Cada cuánto se vuelve a leer la cuenta.
pub const EVERY_HOURS: i64 = 6;
/// Evento para que el frontend recargue GSX y flightsim.to.
pub const EVENT_UPDATED: &str = "fsto-account://updated";

/// Una descarga de la cuenta con versión nueva.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountItem {
    pub file_id: i64,
    pub title: String,
    pub link: String,
    pub thumbnail: Option<String>,
    /// `gsx-pro`, `airports`, `flybywire-a380x`…
    pub sub_category: String,
    pub latest_version: Option<String>,
    /// Epoch (s) de cuando el usuario lo descargó.
    pub downloaded_at: i64,
}

/// Lo último que se leyó de la cuenta.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountState {
    pub checked_at: Option<String>,
    pub logged_in: bool,
    pub items: Vec<AccountItem>,
}

/// Fila tal cual la da `my-downloads`.
#[derive(Deserialize)]
struct RawItem {
    file_id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    thumbnail: Option<String>,
    #[serde(default, rename = "subCategorySlug")]
    sub_category: Option<String>,
    latest_version: Option<String>,
    #[serde(default)]
    update_available: i64,
    #[serde(default)]
    downloaded_at: i64,
}

impl RawItem {
    fn into_item(self) -> AccountItem {
        let link = if self.slug.is_empty() {
            format!("https://flightsim.to/addon/{}", self.file_id)
        } else {
            format!("https://flightsim.to/addon/{}/{}", self.file_id, self.slug)
        };
        AccountItem {
            file_id: self.file_id,
            title: self.title.trim().to_string(),
            link,
            thumbnail: self.thumbnail.filter(|t| !t.trim().is_empty()),
            sub_category: self.sub_category.unwrap_or_default(),
            latest_version: self.latest_version.filter(|v| !v.trim().is_empty()),
            downloaded_at: self.downloaded_at,
        }
    }
}

/// Una página de `my-downloads?filter=updates`: (items con update, páginas).
fn parse_page(body: &str) -> Option<(Vec<AccountItem>, i64)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = v.get("data")?.as_array()?;
    let items = data
        .iter()
        .filter_map(|x| serde_json::from_value::<RawItem>(x.clone()).ok())
        .filter(|r| r.update_available != 0)
        .map(RawItem::into_item)
        .collect();
    let pages = v.pointer("/meta/totalPages").and_then(|p| p.as_i64()).unwrap_or(1);
    Some((items, pages))
}

// ─────────────────────────── caché y ignorados ───────────────────────────

async fn setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str) {
    let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await;
}

/// Lo último leído de la cuenta (sin red).
pub async fn cached(pool: &SqlitePool) -> Option<AccountState> {
    serde_json::from_str(&setting(pool, CACHE_KEY).await?).ok()
}

/// La cuenta, si hubo sesión en la última lectura: entonces manda ella.
pub async fn logged_in(pool: &SqlitePool) -> Option<AccountState> {
    cached(pool).await.filter(|s| s.logged_in)
}

/// Ignorados por el usuario: `file_id` → versión que se ignoró (vuelve a
/// salir si flightsim.to publica otra).
async fn dismissed(pool: &SqlitePool) -> HashMap<i64, String> {
    setting(pool, DISMISS_KEY)
        .await
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn version_key(item: &AccountItem) -> String {
    item.latest_version.clone().unwrap_or_default()
}

/// Lo de la cuenta que el usuario no ignoró.
pub async fn active_items(pool: &SqlitePool, state: &AccountState) -> Vec<AccountItem> {
    let d = dismissed(pool).await;
    state
        .items
        .iter()
        .filter(|i| d.get(&i.file_id) != Some(&version_key(i)))
        .cloned()
        .collect()
}

/// Ignora (hasta la próxima versión) las descargas `file_ids` de la cuenta.
pub async fn dismiss(pool: &SqlitePool, file_ids: &[i64]) {
    let Some(state) = cached(pool).await else { return };
    let mut d = dismissed(pool).await;
    for i in state.items.iter().filter(|i| file_ids.contains(&i.file_id)) {
        d.insert(i.file_id, version_key(i));
    }
    if let Ok(json) = serde_json::to_string(&d) {
        set_setting(pool, DISMISS_KEY, &json).await;
    }
}

// ─────────────────────────── emparejar lo instalado ───────────────────────────

/// Palabras de 4 letras de un título que pueden ser un ICAO.
pub fn icaos_in_title(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|w| w.to_ascii_uppercase())
        .collect()
}

fn words(s: &str) -> HashSet<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3 && !matches!(*w, "gsx" | "profile" | "profiles" | "pro" | "the" | "for" | "and"))
        .map(str::to_string)
        .collect()
}

/// Perfiles GSX instalados con versión nueva en la cuenta: `(icao, ítem)`.
/// `installed`: (ICAO, nombre del .ini). Si hay varios perfiles de la cuenta
/// para el mismo ICAO, el que comparte palabras con el .ini (el autor).
pub fn gsx_matches(items: &[AccountItem], installed: &[(String, String)]) -> Vec<(String, AccountItem)> {
    let gsx: Vec<&AccountItem> = items.iter().filter(|i| i.sub_category == "gsx-pro").collect();
    let mut out = Vec::new();
    for (icao, stem) in installed {
        let cands: Vec<&&AccountItem> = gsx.iter().filter(|i| icaos_in_title(&i.title).contains(icao)).collect();
        let pick = match cands.len() {
            0 => None,
            1 => Some(cands[0]),
            _ => {
                let sw = words(stem);
                cands.iter().copied().max_by_key(|i| words(&i.title).intersection(&sw).count())
            }
        };
        if let Some(i) = pick {
            out.push((icao.clone(), (*i).clone()));
        }
    }
    out
}

// ─────────────────────────── leer la cuenta ───────────────────────────

/// Una lectura a la vez.
static IN_FLIGHT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type LoadTx = Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>;
fn page_loaded() -> &'static LoadTx {
    static L: OnceLock<LoadTx> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// La vista oculta de flightsim.to (mismo perfil y opciones que la pestaña de
/// Explorar), con `robots.txt` cargado.
async fn open_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    *page_loaded().lock().unwrap() = Some(tx);
    let url: tauri::Url = HOME.parse().map_err(|e| format!("{e}"))?;
    let mut b = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::External(url))
        .title("flightsim.to")
        .visible(false)
        .skip_taskbar(true)
        .focused(false)
        .inner_size(480.0, 320.0)
        .disable_drag_drop_handler()
        .on_page_load(|_w, p| {
            if matches!(p.event(), tauri::webview::PageLoadEvent::Finished) {
                if let Some(tx) = page_loaded().lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
    if let Some(dir) = crate::commands::livery::extensions_dir() {
        b = b.browser_extensions_enabled(true).extensions_path(dir);
    }
    let w = b.build().map_err(|e| e.to_string())?;
    if !matches!(tokio::time::timeout(Duration::from_secs(25), rx).await, Ok(Ok(()))) {
        // Sin esto la vista oculta se quedaba abierta (y la siguiente lectura
        // la reutilizaba a medio cargar).
        let _ = w.close();
        return Err("flightsim.to no cargó".to_string());
    }
    Ok(w)
}

/// Ejecuta `js` en la vista y devuelve su resultado como texto.
#[cfg(target_os = "windows")]
async fn eval_string(win: &tauri::WebviewWindow, js: &str) -> Option<String> {
    type Tx = Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>>;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx: Tx = Arc::new(Mutex::new(Some(tx)));
    let js = js.to_string();
    let _ = win.with_webview(move |w| {
        use webview2_com::ExecuteScriptCompletedHandler;
        use windows_core::HSTRING;
        let tx_cb = tx.clone();
        // SAFETY: COM de WebView2 en el hilo de UI.
        unsafe {
            let Ok(core) = w.controller().CoreWebView2() else { return };
            let handler = ExecuteScriptCompletedHandler::create(Box::new(move |_hr, json| {
                if let Some(t) = tx_cb.lock().unwrap().take() {
                    let _ = t.send(json);
                }
                Ok(())
            }));
            let _ = core.ExecuteScript(&HSTRING::from(js.as_str()), &handler);
        }
    });
    let json = tokio::time::timeout(Duration::from_secs(5), rx).await.ok()?.ok()?;
    // ExecuteScript devuelve el resultado en JSON: una cadena viene entre comillas.
    serde_json::from_str::<Option<String>>(&json).ok().flatten()
}

#[cfg(not(target_os = "windows"))]
async fn eval_string(_win: &tauri::WebviewWindow, _js: &str) -> Option<String> {
    None
}

/// `GET path` desde la página de flightsim.to (con su sesión): (estado, cuerpo).
async fn fetch_text(win: &tauri::WebviewWindow, path: &str) -> Result<(u16, String), String> {
    let start = format!(
        "window.__sfAcct=null;fetch({path:?},{{credentials:'include',headers:{{Accept:'application/json'}}}})\
         .then(async r=>{{window.__sfAcct=JSON.stringify({{s:r.status,b:await r.text()}});}})\
         .catch(e=>{{window.__sfAcct=JSON.stringify({{s:0,b:String(e)}});}});'ok'"
    );
    eval_string(win, &start).await.ok_or("no se pudo pedir a flightsim.to")?;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(raw) = eval_string(win, "window.__sfAcct").await {
            #[derive(Deserialize)]
            struct R {
                s: u16,
                b: String,
            }
            let r: R = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            return Ok((r.s, r.b));
        }
    }
    Err("flightsim.to no respondió".into())
}

/// Lee la cuenta ahora (con red), la guarda y avisa al frontend.
pub async fn refresh(app: &tauri::AppHandle, pool: &SqlitePool) -> Result<AccountState, String> {
    let _one = IN_FLIGHT.lock().await;
    let win = open_window(app).await?;
    let result = read_all(&win).await;
    let _ = win.close();
    let state = result?;
    tracing::info!(
        target: "flightsim",
        "cuenta de flightsim.to: {} ({} descarga(s) con versión nueva)",
        if state.logged_in { "con sesión" } else { "SIN sesión" },
        state.items.len()
    );
    if let Ok(json) = serde_json::to_string(&state) {
        set_setting(pool, CACHE_KEY, &json).await;
    }
    let _ = app.emit(EVENT_UPDATED, ());
    Ok(state)
}

async fn read_all(win: &tauri::WebviewWindow) -> Result<AccountState, String> {
    let mut items = Vec::new();
    let mut page = 1;
    loop {
        let path = format!("/backend/user/my-downloads?page={page}&limit=100&filter=updates&sort=downloaded");
        let (status, body) = fetch_text(win, &path).await?;
        if matches!(status, 401 | 403) {
            return Ok(AccountState { checked_at: Some(chrono::Utc::now().to_rfc3339()), logged_in: false, items: Vec::new() });
        }
        if status != 200 {
            return Err(format!("flightsim.to respondió HTTP {status}"));
        }
        // Sin sesión puede llegar la página de entrar (HTML), no el JSON.
        let Some((found, pages)) = parse_page(&body) else {
            return Ok(AccountState { checked_at: Some(chrono::Utc::now().to_rfc3339()), logged_in: false, items: Vec::new() });
        };
        items.extend(found);
        if page >= pages.min(10) {
            break;
        }
        page += 1;
    }
    Ok(AccountState { checked_at: Some(chrono::Utc::now().to_rfc3339()), logged_in: true, items })
}

/// ¿Toca leer otra vez? (sin lectura o más vieja que `EVERY_HOURS`).
async fn due(pool: &SqlitePool) -> bool {
    let Some(at) = cached(pool).await.and_then(|s| s.checked_at) else { return true };
    chrono::DateTime::parse_from_rfc3339(&at).map_or(true, |t| {
        chrono::Utc::now() - t.with_timezone(&chrono::Utc) >= chrono::Duration::hours(EVERY_HOURS)
    })
}

/// Al arrancar (tras dejar que la app se asiente) y luego cada 6 h.
pub fn spawn_loop(app: tauri::AppHandle, pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if due(&pool).await {
                if let Err(e) = refresh(&app, &pool).await {
                    tracing::warn!(target: "flightsim", "cuenta de flightsim.to: no se pudo leer: {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, title: &str, sub: &str) -> AccountItem {
        AccountItem { file_id: id, title: title.into(), sub_category: sub.into(), ..Default::default() }
    }

    #[test]
    fn a_my_downloads_page_keeps_only_what_has_an_update() {
        let body = r#"{"data":[
            {"file_id":98574,"title":"ESSA Stockholm Arlanda by Orbx GSX Profile ","slug":"essa-orbx","thumbnail":"https://cdn/x.jpg","subCategorySlug":"gsx-pro","latest_version":"1.12.1","update_available":1,"downloaded_at":1792385802},
            {"file_id":1,"title":"Old","slug":"old","subCategorySlug":"airports","update_available":0,"downloaded_at":5}
        ],"meta":{"totalPages":2,"totalUpdates":31}}"#;
        let (items, pages) = parse_page(body).unwrap();
        assert_eq!(pages, 2);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "ESSA Stockholm Arlanda by Orbx GSX Profile");
        assert_eq!(items[0].link, "https://flightsim.to/addon/98574/essa-orbx");
        assert!(parse_page("<html>login</html>").is_none());
    }

    #[test]
    fn installed_gsx_profiles_are_matched_by_icao_and_author() {
        let items = vec![
            item(1, "ESSA Stockholm Arlanda by Orbx GSX Profile", "gsx-pro"),
            item(2, "GSX Profile - KMCI (gregmason)", "gsx-pro"),
            item(3, "LEMD Madrid by LatinVFR GSX Profile", "gsx-pro"),
            item(4, "LEMD Madrid by Other Dev GSX Profile", "gsx-pro"),
            item(5, "EHAM Amsterdam livery pack", "airports"),
        ];
        let installed = vec![
            ("ESSA".to_string(), "essa-orbx-likeusb-GSX-VDGS".to_string()),
            ("LEMD".to_string(), "lemd-latinvfr-v2".to_string()),
            ("EHAM".to_string(), "eham-flytampa".to_string()),
        ];
        let m = gsx_matches(&items, &installed);
        let got: Vec<(String, i64)> = m.iter().map(|(i, a)| (i.clone(), a.file_id)).collect();
        assert_eq!(got, vec![("ESSA".into(), 1), ("LEMD".into(), 3)], "KMCI no está instalado; EHAM no es un perfil");
        assert_eq!(icaos_in_title("GSX Profile - KMCI (gregmason)"), vec!["KMCI".to_string()]);
    }
}
