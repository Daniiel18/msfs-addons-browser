//! (19/10) Etapa 2 — Sincronía de SimFleet con Fenix TechLog y PMDG TechLog.
//!
//! Para los aviones que tienen TechLog, TechLog es la FUENTE DE VERDAD del
//! mantenimiento (decisión del usuario): SimFleet muestra su estado en el
//! Hangar, deja de aplicarles su propio desgaste y cobra en Finanzas lo que
//! TechLog hace (servicios, reparaciones, reemplazos, defectos, checks).
//!
//! Dos caminos de lectura:
//!   · EN VIVO (TechLog abierto): su API local (`localhost:7733` Fenix,
//!     `localhost:7755` PMDG). Desde loopback no pide PIN; las acciones (POST)
//!     exigen la cabecera `X-TechLog: 1`. La API actúa sobre el avión ACTIVO
//!     de TechLog, así que SimFleet sólo ofrece acciones para ese avión.
//!   · SIN TECHLOG: su `techlog.db` en SOLO LECTURA. Nunca abrimos el archivo
//!     original: copiamos `techlog.db` + `-wal` a una carpeta temporal y
//!     leemos la copia (cero riesgo de bloquear o corromper su base). Las
//!     cifras derivadas (vida restante, checks) usan las mismas fórmulas que
//!     TechLog (ver `SimFleet-capturas/techlog-analisis/ANALISIS-TECHLOG.md`).
//!
//! Eventos en vivo: un hilo por app escucha `GET /api/events` (SSE) y, ante un
//! evento `state`, avisa al front (`techlog://changed`) y sincroniza cobros.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Row, SqliteConnection, SqlitePool};
use tauri::Emitter;

// ---------------------------------------------------------------------------
// Fuentes e instalaciones
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlSource {
    Fenix,
    Pmdg,
    /// (19/10) Etapa 3: Tech Log propio de SimFleet (resto de aviones).
    Simfleet,
}

impl TlSource {
    pub const ALL: [TlSource; 2] = [TlSource::Fenix, TlSource::Pmdg];

    pub fn id(self) -> &'static str {
        match self {
            TlSource::Fenix => "fenix",
            TlSource::Pmdg => "pmdg",
            TlSource::Simfleet => "simfleet",
        }
    }
    pub fn app_name(self) -> &'static str {
        match self {
            TlSource::Fenix => "Fenix TechLog",
            TlSource::Pmdg => "PMDG TechLog",
            TlSource::Simfleet => "SimFleet Tech Log",
        }
    }
    pub fn port(self) -> u16 {
        if let Some(p) = test_override(self, "PORT").and_then(|p| p.parse().ok()) {
            return p;
        }
        match self {
            TlSource::Fenix => 7733,
            TlSource::Pmdg => 7755,
            TlSource::Simfleet => 0,
        }
    }
    pub fn base_url(self) -> String {
        format!("http://localhost:{}", self.port())
    }
    pub fn parse(s: &str) -> Option<TlSource> {
        match s {
            "fenix" => Some(TlSource::Fenix),
            "pmdg" => Some(TlSource::Pmdg),
            "simfleet" => Some(TlSource::Simfleet),
            _ => None,
        }
    }
}

/// (20/10) Sólo en el entorno de pruebas: `SIMFLEET_TECHLOG_<FENIX|PMDG>_DIR`,
/// `_PORT` y `_ARGS` apuntan a una COPIA de TechLog (otra carpeta, otro
/// puerto, p. ej. `--offline`) para probar sin tocar la instalación real.
/// La instancia principal los ignora.
fn test_override(source: TlSource, what: &str) -> Option<String> {
    if !crate::test_env::is_active() {
        return None;
    }
    std::env::var(format!("SIMFLEET_TECHLOG_{}_{what}", source.id().to_uppercase()))
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// Carpeta de instalación de una app TechLog: la del desinstalador de Windows
/// (si el usuario eligió otra) o la de por defecto.
pub fn install_dir(source: TlSource) -> Option<PathBuf> {
    if let Some(dir) = test_override(source, "DIR") {
        let p = PathBuf::from(dir);
        return p.join("catalog").is_dir().then_some(p);
    }
    #[cfg(windows)]
    if let Some(p) = registry_install_location(source.app_name()) {
        if p.join("catalog").is_dir() {
            return Some(p);
        }
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let p = local.join("Programs").join(source.app_name());
    p.join("catalog").is_dir().then_some(p)
}

/// `techlog.db`: en la carpeta de instalación (`data\`) o, en builds antiguas
/// de PMDG TechLog, en `%LOCALAPPDATA%\PMDG TechLog\data`.
fn db_path(source: TlSource, dir: &Path) -> Option<PathBuf> {
    let p = dir.join("data").join("techlog.db");
    if p.is_file() {
        return Some(p);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let p = local.join(source.app_name()).join("data").join("techlog.db");
    p.is_file().then_some(p)
}

#[cfg(windows)]
fn registry_install_location(display_name: &str) -> Option<PathBuf> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ,
    };
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

    fn read_sz(root: HKEY, subkey: &str, value: &str) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(
                root,
                &HSTRING::from(subkey),
                &HSTRING::from(value),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut size),
            )
        };
        if r.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut hk = HKEY::default();
        if unsafe { RegOpenKeyExW(root, &HSTRING::from(UNINSTALL), Some(0), KEY_READ, &mut hk) }
            .is_err()
        {
            continue;
        }
        let mut found = None;
        for i in 0u32.. {
            let mut name = [0u16; 260];
            let mut len = name.len() as u32;
            let r = unsafe {
                RegEnumKeyExW(hk, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None)
            };
            if r.is_err() {
                break;
            }
            let sub = format!(
                r"{UNINSTALL}\{}",
                String::from_utf16_lossy(&name[..len as usize])
            );
            if read_sz(root, &sub, "DisplayName").is_some_and(|d| d.trim() == display_name) {
                if let Some(loc) = read_sz(root, &sub, "InstallLocation") {
                    let loc = loc.trim().trim_matches('"').to_string();
                    if !loc.is_empty() {
                        found = Some(PathBuf::from(loc));
                        break;
                    }
                }
            }
        }
        unsafe {
            let _ = RegCloseKey(hk);
        }
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Matrícula normalizada como en SimFleet (`airline_economy::normalize_reg`),
/// quitando antes el sufijo de tipo que TechLog añade cuando dos libreas
/// comparten matrícula (`G-FENX-A320-CFM` → `GFENX`).
pub fn normalize_reg(reg: &str) -> String {
    let parts: Vec<&str> = reg.trim().split('-').collect();
    let base = if parts.len() >= 3
        && matches!(parts[parts.len() - 1].to_ascii_uppercase().as_str(), "CFM" | "IAE")
    {
        parts[..parts.len() - 2].join("-")
    } else {
        reg.to_string()
    };
    crate::airline_economy::normalize_reg(&base)
}

/// Matrícula para mostrar: la de TechLog sin el sufijo de tipo
/// (`G-FENX-A320-CFM` → `G-FENX`).
pub fn display_reg(reg: &str) -> String {
    let parts: Vec<&str> = reg.trim().split('-').collect();
    if parts.len() >= 3 && matches!(parts[parts.len() - 1].to_ascii_uppercase().as_str(), "CFM" | "IAE") {
        parts[..parts.len() - 2].join("-")
    } else {
        reg.trim().to_string()
    }
    .to_uppercase()
}

/// (21/10) Un avión de Fenix/PMDG TechLog (para el Hangar).
#[derive(Debug, Clone)]
pub struct TlAirframeInfo {
    pub registration: String,
    pub variant: String,
    pub engine: String,
    pub last_seen_utc: Option<String>,
}

/// (21/10) Todos los aviones de las apps TechLog, desde su base en solo
/// lectura: el Hangar muestra también los que aún no tienen vuelos en SimFleet.
pub async fn airframe_list() -> Vec<TlAirframeInfo> {
    let mut out = Vec::new();
    for source in TlSource::ALL {
        let Some(dir) = install_dir(source) else { continue };
        let Some(db) = db_path(source, &dir) else { continue };
        if let Ok(mut conn) = open_snapshot(source, &db).await {
            if let Ok(afs) = db_airframes(&mut conn).await {
                out.extend(afs.into_iter().map(|a| TlAirframeInfo {
                    registration: a.registration,
                    variant: a.variant,
                    engine: a.engine,
                    last_seen_utc: a.last_seen_utc,
                }));
            }
            conn.close().await.ok();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Catálogo (sólo lo que hace falta para leer la base sin TechLog)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CompDef {
    key: String,
    name: String,
    ata: i64,
    system: Option<String>,
    positions: Option<Vec<String>>,
    basis: Option<String>,
    life: f64,
    kind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ConsDef {
    key: String,
    name: String,
    unit: String,
    positions: Option<Vec<String>>,
    capacity: Option<f64>,
    capacity_by_engine: Option<HashMap<String, f64>>,
    min_dispatch: Option<f64>,
    min_dispatch_by_engine: Option<HashMap<String, f64>>,
    low: Option<f64>,
    low_by_engine: Option<HashMap<String, f64>>,
    fills: bool,
}

impl ConsDef {
    fn pick(single: Option<f64>, by: &Option<HashMap<String, f64>>, engine: &str) -> Option<f64> {
        by.as_ref().and_then(|m| m.get(engine).copied()).or(single)
    }
    fn capacity(&self, engine: &str) -> f64 {
        Self::pick(self.capacity, &self.capacity_by_engine, engine).unwrap_or(100.0)
    }
    fn min_dispatch(&self, engine: &str) -> f64 {
        Self::pick(self.min_dispatch, &self.min_dispatch_by_engine, engine).unwrap_or(0.0)
    }
    fn low(&self, engine: &str) -> f64 {
        Self::pick(self.low, &self.low_by_engine, engine).unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ComponentsFile {
    components: Vec<CompDef>,
    consumables: Vec<ConsDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CheckDef {
    key: String,
    name: String,
    hours: Option<f64>,
    days: Option<f64>,
    flight_hours: Option<f64>,
    cycles: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CommonDef {
    worn_below_pct: f64,
}
impl Default for CommonDef {
    fn default() -> Self {
        Self { worn_below_pct: 25.0 }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct LimitsFile {
    checks: Vec<CheckDef>,
    common: CommonDef,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MelItem {
    id: String,
    title: String,
    fix_hours: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct MelFile {
    items: Vec<MelItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SparesFile {
    common_ata: Vec<i64>,
    rare_ata: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
struct Catalog {
    components: Vec<CompDef>,
    consumables: Vec<ConsDef>,
    limits: LimitsFile,
    mel: HashMap<String, MelItem>,
    spares: SparesFile,
}

impl Catalog {
    fn component(&self, key: &str) -> Option<&CompDef> {
        self.components.iter().find(|c| c.key == key)
    }
    /// ATA de una pieza a partir de su nombre visible ("Brake 3", "Pack 1
    /// flow control"): los registros de servicio de TechLog guardan el nombre.
    fn ata_by_name(&self, name: &str) -> Option<i64> {
        self.by_name(name).map(|c| c.ata)
    }
    /// (24/10) Clave de catálogo de una pieza por su nombre visible.
    fn key_by_name(&self, name: &str) -> Option<String> {
        self.by_name(name).map(|c| c.key.clone())
    }
    fn by_name(&self, name: &str) -> Option<&CompDef> {
        let name = name.trim();
        self.components.iter().find(|c| {
            let positions = c.positions.clone().unwrap_or_else(|| vec![String::new()]);
            positions
                .iter()
                .any(|p| c.name.replace("{pos}", p).trim().eq_ignore_ascii_case(name))
        })
    }
}

/// JSON con comentarios (`//`, `/* */`) y comas finales → JSON.
/// Recorre BYTES y copia bytes: los delimitadores son ASCII y en UTF-8 un byte
/// ASCII nunca forma parte de un carácter multibyte, así que "°", "—" o "ñ"
/// pasan intactos (antes se copiaba cada byte como `char` y salía "Â°").
fn strip_jsonc(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            out.push(b[i]);
            if c == '\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(b[i]);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    // Sólo se quitaron tramos delimitados por ASCII → sigue siendo UTF-8
    // válido (el `lossy` es sólo defensa).
    let out = String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    // Comas finales antes de } o ].
    let re = regex::Regex::new(r",(\s*[}\]])").expect("regex");
    re.replace_all(&out, "$1").into_owned()
}

fn read_jsonc<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    // Los .json de TechLog son UTF-8; `from_utf8_lossy` por si acaso: así un
    // byte raro no rompe el parse de todo el catálogo.
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            serde_json::from_str(&strip_jsonc(&text)).unwrap_or_else(|e| {
                tracing::warn!(target: "techlog", "catálogo {} ilegible: {e}", path.display());
                T::default()
            })
        }
        Err(_) => T::default(),
    }
}

/// (23/10) Fenix TechLog 2.2+: el aceite de motor lo lleva el Fenix y TechLog
/// lo lee (`oil1Qt` con dataref en su catálogo); rellenarlo no hace nada.
fn oil_follows_sim(source: TlSource, dir: &Path) -> bool {
    if source != TlSource::Fenix {
        return false;
    }
    let v: Value = read_jsonc(&dir.join("catalog").join("fenix-datarefs.json"));
    v.pointer("/datarefs/oil1Qt")
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

/// (25/10) ¿Es un 777? PMDG TechLog 0.2 lleva el 777 ("777-200" … "777F").
fn is_777(variant: &str) -> bool {
    variant.trim().starts_with("777")
}

/// Carpeta de catálogo de un avión: PMDG TechLog 0.2 trae el del 777 aparte
/// (`catalog/b777`: sus piezas, MEL, límites y fallos).
fn catalog_dir(dir: &Path, variant: &str) -> PathBuf {
    let root = dir.join("catalog");
    let b777 = root.join("b777");
    if is_777(variant) && b777.join("components.json").is_file() {
        b777
    } else {
        root
    }
}

fn catalog_for(dir: &Path, variant: &str) -> Catalog {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (SystemTime, Catalog)>>> = OnceLock::new();
    let cat_dir = catalog_dir(dir, variant);
    let stamp = std::fs::metadata(cat_dir.join("components.json"))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(g) = cache.lock() {
        if let Some((s, c)) = g.get(&cat_dir) {
            if *s == stamp {
                return c.clone();
            }
        }
    }
    let comps: ComponentsFile = read_jsonc(&cat_dir.join("components.json"));
    let limits: LimitsFile = read_jsonc(&cat_dir.join("limits.json"));
    let mel: MelFile = read_jsonc(&cat_dir.join("mel.json"));
    // El del 777 no trae repuestos propios: los del catálogo general.
    let spares_dir = if cat_dir.join("spares.json").is_file() { cat_dir.clone() } else { dir.join("catalog") };
    let spares: SparesFile = read_jsonc(&spares_dir.join("spares.json"));
    let cat = Catalog {
        components: comps.components,
        consumables: comps.consumables,
        limits,
        mel: mel.items.into_iter().map(|m| (m.id.clone(), m)).collect(),
        spares,
    };
    if let Ok(mut g) = cache.lock() {
        g.insert(cat_dir, (stamp, cat.clone()));
    }
    cat
}

// ---------------------------------------------------------------------------
// Copia de solo lectura de techlog.db
// ---------------------------------------------------------------------------

fn file_sig(p: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Copia `techlog.db` (+ `-wal`) a `%TEMP%\simfleet-techlog\<fuente>\` si
/// cambió desde la última copia, y abre la COPIA. El `-shm` no se copia:
/// SQLite lo reconstruye y así nunca arrastramos un índice a medio escribir.
async fn open_snapshot(source: TlSource, db: &Path) -> Result<SqliteConnection> {
    static SIGS: OnceLock<Mutex<HashMap<TlSource, String>>> = OnceLock::new();
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    let sig = format!("{:?}|{:?}", file_sig(db), file_sig(&wal));
    // En tests, carpeta propia: no pisar la copia que use la app abierta. Y una
    // por base: las pruebas corren a la vez en el mismo proceso (una con una
    // base de prueba, otra leyendo qué matrículas lleva TechLog).
    let dir = if cfg!(test) {
        let tag = db.display().to_string().bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        std::env::temp_dir().join(format!("simfleet-techlog-test-{}-{tag:08x}", std::process::id()))
    } else {
        std::env::temp_dir().join("simfleet-techlog")
    }
    .join(source.id());
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join("techlog.db");
    let sigs = SIGS.get_or_init(|| Mutex::new(HashMap::new()));
    let fresh = sigs
        .lock()
        .map(|g| g.get(&source).map(|s| s == &sig).unwrap_or(false))
        .unwrap_or(false);
    if !fresh || !copy.is_file() {
        let _ = std::fs::remove_file(dir.join("techlog.db-shm"));
        let _ = std::fs::remove_file(dir.join("techlog.db-wal"));
        std::fs::copy(db, &copy).with_context(|| format!("copiando {}", db.display()))?;
        if wal.is_file() {
            std::fs::copy(&wal, dir.join("techlog.db-wal"))?;
        }
        if let Ok(mut g) = sigs.lock() {
            g.insert(source, sig);
        }
    }
    let conn = SqliteConnectOptions::new()
        .filename(&copy)
        .create_if_missing(false)
        .disable_statement_logging()
        .connect()
        .await?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Vista unificada que consume el front (Hangar / FlightBook)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlDispatch {
    /// "GO" | "GO_MEL" | "NO_GO"
    pub status: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlDefect {
    pub id: i64,
    pub ata: i64,
    pub title: String,
    pub detail: Option<String>,
    /// "NoGo" | "A" | "B" | "C" | "D" | …
    pub category: String,
    /// "Open" | "Deferred"
    pub status: String,
    pub mel_id: Option<String>,
    pub position: Option<String>,
    pub raised_utc: Option<String>,
    pub deferred_until: Option<String>,
    /// Última fecha a la que se puede diferir (sólo en vivo).
    pub latest_deferral: Option<String>,
    pub can_defer: bool,
    pub failure: Option<String>,
    pub placard: Option<String>,
    pub restrictions: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlPart {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub position: Option<String>,
    pub ata: i64,
    pub system: String,
    /// "life" | "wear" | "cabin"
    pub kind: String,
    pub remaining_pct: f64,
    /// "Ok" | "Degraded" | "Failed"
    pub status: String,
    pub repairs: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TlCondition {
    pub good: usize,
    pub low: usize,
    pub failed: usize,
    /// (20/10) Vida media restante de TODAS las piezas (una fallada cuenta 0 %).
    pub avg_remaining_pct: f64,
    #[serde(skip)]
    rem_sum: f64,
}

impl TlCondition {
    /// Cuenta una pieza: su estado y su vida restante para la media.
    pub fn count(&mut self, status: &str, low: bool, remaining_pct: f64) {
        if status == "Failed" {
            self.failed += 1;
        } else if low || status == "Degraded" {
            self.low += 1;
        } else {
            self.good += 1;
        }
        self.rem_sum += if status == "Failed" { 0.0 } else { remaining_pct.clamp(0.0, 100.0) };
        let n = (self.good + self.low + self.failed) as f64;
        self.avg_remaining_pct = (self.rem_sum / n * 10.0).round() / 10.0;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlConsumable {
    pub key: String,
    pub position: Option<String>,
    pub name: String,
    pub unit: String,
    pub level: f64,
    pub capacity: f64,
    pub min_dispatch: f64,
    pub low: f64,
    pub fills: bool,
    /// "ok" | "low" | "below_min"
    pub status: String,
    /// Valor para `POST /api/service`.
    pub service_kind: String,
    pub service_label: String,
    /// (23/10) El nivel lo lleva el avión del sim y TechLog sólo lo sigue
    /// (Fenix TechLog 2.2: aceite de motor): no hay nada que rellenar.
    pub follows_sim: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCheck {
    pub key: String,
    pub name: String,
    pub last_done_utc: Option<String>,
    pub due_utc: Option<String>,
    pub overdue: bool,
    pub hours_left: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechLogAircraft {
    pub source: TlSource,
    pub app_name: String,
    /// Página de tablet de TechLog (abrir en el navegador).
    pub tablet_url: String,
    /// TechLog está abierto (API respondiendo).
    pub running: bool,
    /// Este avión es el ACTIVO en TechLog y está abierto → acciones posibles.
    pub active: bool,
    pub airframe_id: i64,
    pub registration: String,
    pub variant: String,
    pub engine: String,
    pub maintenance: String,
    pub flight_hours: f64,
    pub cycles: f64,
    pub egt_margin_c: Vec<f64>,
    pub dispatch: TlDispatch,
    pub defects: Vec<TlDefect>,
    /// Sólo las piezas gastadas (bajo el umbral "worn"), degradadas o falladas.
    pub parts: Vec<TlPart>,
    pub condition: TlCondition,
    pub consumables: Vec<TlConsumable>,
    pub checks: Vec<TlCheck>,
    pub last_seen_utc: Option<String>,
    /// (24/10) Por qué una orden ahora quedaría en cola: `sim_closed` |
    /// `not_loaded` | `not_parked`. `None`: las órdenes salen ya.
    pub order_wait: Option<String>,
    /// (24/10) Trabajo en curso en TechLog (tiempos realistas): se cobra al
    /// terminar.
    pub pending_work: Option<String>,
}

// ---------------------------------------------------------------------------
// Lectura desde la base (TechLog cerrado o avión no activo)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Counters {
    flight_hours: f64,
    block_hours: f64,
    cycles: f64,
    landings: f64,
    pax_cycles: f64,
    eng_hours: Vec<f64>,
    eng_cycles: Vec<f64>,
    eng_starts: Vec<f64>,
    apu_hours: f64,
    apu_starts: f64,
    rev_deploys: Vec<f64>,
    wear_extra: HashMap<String, f64>,
}

impl Counters {
    /// Igual que `Counters.WearValue` de TechLog.
    fn wear_value(&self, basis: &str, engine: usize, now: DateTime<Utc>) -> f64 {
        let e = engine.clamp(1, 2) - 1;
        let at = |v: &Vec<f64>| v.get(e).copied().unwrap_or(0.0);
        let (value, enum_name) = match basis {
            "flightHours" => (self.flight_hours, "FlightHours"),
            "blockHours" => (self.block_hours, "BlockHours"),
            "cycles" => (self.cycles, "Cycles"),
            "landings" => (self.landings, "Landings"),
            "paxCycles" => (self.pax_cycles, "PaxCycles"),
            "engHours" => (at(&self.eng_hours), "EngHours"),
            "engCycles" => (at(&self.eng_cycles), "EngCycles"),
            "engStarts" => (at(&self.eng_starts), "EngStarts"),
            "apuHours" => (self.apu_hours, "ApuHours"),
            "apuStarts" => (self.apu_starts, "ApuStarts"),
            "revDeploys" => (at(&self.rev_deploys), "RevDeploys"),
            "days" => {
                let epoch = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(now);
                return (now - epoch).num_seconds() as f64 / 86_400.0;
            }
            _ => (self.flight_hours, "FlightHours"),
        };
        value
            + self
                .wear_extra
                .get(&format!("{enum_name}:{}", e + 1))
                .copied()
                .unwrap_or(0.0)
    }
}

fn engine_index(position: Option<&str>) -> usize {
    position
        .and_then(|p| p.chars().next())
        .and_then(|c| c.to_digit(10))
        .map(|d| (d as usize).clamp(1, 2))
        .unwrap_or(1)
}

fn opt_str(row: &sqlx::sqlite::SqliteRow, col: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(col)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty() && s != "null")
}

fn dispatch_of(defects: &[TlDefect]) -> TlDispatch {
    let mut reasons = Vec::new();
    for d in defects.iter().filter(|d| d.status == "Open" && d.category == "NoGo") {
        reasons.push(format!("NO-GO: {}", d.title));
    }
    for d in defects.iter().filter(|d| d.status == "Open" && d.category != "NoGo") {
        reasons.push(format!("Open, fix or defer: {}", d.title));
    }
    if !reasons.is_empty() {
        return TlDispatch { status: "NO_GO".into(), reasons };
    }
    let deferred = defects.iter().filter(|d| d.status == "Deferred").count();
    if deferred == 0 {
        TlDispatch { status: "GO".into(), reasons }
    } else {
        TlDispatch {
            status: "GO_MEL".into(),
            reasons: vec![format!("{deferred} item(s) deferred under the MEL")],
        }
    }
}

fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// Registro de avión leído de la base.
struct DbAirframe {
    id: i64,
    registration: String,
    variant: String,
    engine: String,
    maintenance: String,
    counters: Counters,
    consumables: HashMap<String, f64>,
    egt_margin: Vec<f64>,
    last_seen_utc: Option<String>,
}

async fn db_airframes(conn: &mut SqliteConnection) -> Result<Vec<DbAirframe>> {
    let rows = sqlx::query("SELECT * FROM airframe").fetch_all(&mut *conn).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let counters: Counters = opt_str(r, "counters")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let consumables: HashMap<String, f64> = opt_str(r, "consumables")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let egt_margin = opt_str(r, "health")
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| {
                    v.get("egtMarginC").and_then(|a| {
                        a.as_array()
                            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>())
                    })
                })
                .unwrap_or_default();
            DbAirframe {
                id: r.try_get("id").unwrap_or(0),
                registration: opt_str(r, "registration").unwrap_or_default(),
                variant: opt_str(r, "variant").unwrap_or_default(),
                engine: opt_str(r, "engine").unwrap_or_default(),
                maintenance: opt_str(r, "maintenance").unwrap_or_else(|| "standard".into()),
                counters,
                consumables,
                egt_margin,
                last_seen_utc: opt_str(r, "last_seen_utc"),
            }
        })
        .collect())
}

async fn wear_rates(conn: &mut SqliteConnection) -> (f64, f64) {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key='settings'")
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten();
    let v: Value = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    (
        v.get("wearRate").and_then(|x| x.as_f64()).unwrap_or(3.0),
        v.get("cabinWearRate").and_then(|x| x.as_f64()).unwrap_or(2.0),
    )
}

async fn aircraft_from_db(
    source: TlSource,
    dir: &Path,
    db: &Path,
    reg_norm: &str,
) -> Result<Option<TechLogAircraft>> {
    let mut conn = open_snapshot(source, db).await?;
    let airframes = db_airframes(&mut conn).await?;
    let Some(a) = airframes
        .into_iter()
        .find(|a| normalize_reg(&a.registration) == reg_norm)
    else {
        conn.close().await.ok();
        return Ok(None);
    };
    let cat = catalog_for(dir, &a.variant);
    let (rate, cabin_rate) = wear_rates(&mut conn).await;
    let now = Utc::now();

    // Defectos abiertos y diferidos.
    let defect_rows = sqlx::query(
        "SELECT * FROM defect WHERE airframe_id = ?1 AND status IN ('Open','Deferred') ORDER BY raised_utc",
    )
    .bind(a.id)
    .fetch_all(&mut conn)
    .await?;
    let defects: Vec<TlDefect> = defect_rows
        .iter()
        .map(|r| {
            let category = opt_str(r, "category").unwrap_or_default();
            TlDefect {
                id: r.try_get("id").unwrap_or(0),
                ata: r.try_get("ata").unwrap_or(0),
                title: opt_str(r, "title").unwrap_or_default(),
                detail: opt_str(r, "detail"),
                can_defer: category != "NoGo",
                category,
                status: opt_str(r, "status").unwrap_or_default(),
                mel_id: opt_str(r, "mel_id"),
                position: opt_str(r, "position"),
                raised_utc: opt_str(r, "raised_utc"),
                deferred_until: opt_str(r, "deferred_until"),
                latest_deferral: None,
                failure: opt_str(r, "active_failure").or_else(|| opt_str(r, "deferred_failure")),
                placard: opt_str(r, "placard"),
                restrictions: opt_str(r, "restrictions"),
            }
        })
        .collect();

    // Piezas: % restante con la fórmula de TechLog.
    let part_rows = sqlx::query("SELECT * FROM component WHERE airframe_id = ?1")
        .bind(a.id)
        .fetch_all(&mut conn)
        .await?;
    let worn_below = cat.limits.common.worn_below_pct;
    let mut condition = TlCondition::default();
    let mut parts = Vec::new();
    for r in &part_rows {
        let key = opt_str(r, "key").unwrap_or_default();
        let Some(def) = cat.component(&key) else {
            continue;
        };
        let position = opt_str(r, "position");
        let status = opt_str(r, "status").unwrap_or_else(|| "Ok".into());
        let kind = def.kind.clone().unwrap_or_else(|| "life".into());
        let remaining = if kind == "wear" {
            100.0 - r.try_get::<f64, _>("wear").unwrap_or(0.0)
        } else {
            let life: f64 = r.try_get("life").unwrap_or(def.life);
            let basis_at: f64 = r.try_get("basis_at_install").unwrap_or(0.0);
            let rr = if kind == "cabin" { cabin_rate } else { rate };
            let span = life / rr.max(0.05);
            let used = a.counters.wear_value(
                def.basis.as_deref().unwrap_or("flightHours"),
                engine_index(position.as_deref()),
                now,
            ) - basis_at;
            if span <= 0.0 {
                0.0
            } else {
                100.0 * (1.0 - used / span)
            }
        }
        .clamp(0.0, 100.0);
        condition.count(&status, remaining < worn_below, remaining);
        if status != "Ok" || remaining < worn_below {
            let pos_label = position.clone().unwrap_or_default();
            parts.push(TlPart {
                id: r.try_get("id").unwrap_or(0),
                key: key.clone(),
                name: def.name.replace("{pos}", &pos_label).trim().to_string(),
                position,
                ata: def.ata,
                system: def.system.clone().unwrap_or_default(),
                kind,
                remaining_pct: (remaining * 10.0).round() / 10.0,
                status,
                repairs: r.try_get("repairs").unwrap_or(0),
            });
        }
    }
    parts.sort_by(|x, y| x.remaining_pct.total_cmp(&y.remaining_pct));

    // Consumibles.
    let mut consumables = Vec::new();
    for c in &cat.consumables {
        let positions = c.positions.clone().unwrap_or_else(|| vec![String::new()]);
        for p in positions {
            let ck = if p.is_empty() { c.key.clone() } else { format!("{}:{p}", c.key) };
            let cap = c.capacity(&a.engine);
            let level = a.consumables.get(&ck).copied().unwrap_or(if c.fills { 0.0 } else { cap });
            let (min, low) = (c.min_dispatch(&a.engine), c.low(&a.engine));
            // Mismos umbrales que TechLog: en los tanques que se LLENAN
            // (residuos) el NO-GO es superar su máximo de despacho.
            let status = if c.fills {
                if min > 0.0 && level >= min {
                    "below_min"
                } else if low > 0.0 && level >= low {
                    "low"
                } else {
                    "ok"
                }
            } else if level < min {
                "below_min"
            } else if level < low.max(min + (cap - min) * 0.25) {
                "low"
            } else {
                "ok"
            };
            consumables.push(TlConsumable {
                key: c.key.clone(),
                position: (!p.is_empty()).then(|| p.clone()),
                name: c.name.replace("{pos}", &p).trim().to_string(),
                unit: c.unit.clone(),
                level: (level * 10.0).round() / 10.0,
                capacity: cap,
                min_dispatch: min,
                low,
                fills: c.fills,
                status: status.into(),
                service_kind: ck,
                service_label: if c.fills { "Empty".into() } else { "Fill".into() },
                follows_sim: c.key == "ENG_OIL" && oil_follows_sim(source, dir),
            });
        }
    }

    // Checks.
    let check_rows = sqlx::query("SELECT * FROM check_record WHERE airframe_id = ?1")
        .bind(a.id)
        .fetch_all(&mut conn)
        .await?;
    let checks = cat
        .limits
        .checks
        .iter()
        .map(|def| {
            let row = check_rows
                .iter()
                .find(|r| opt_str(r, "key").as_deref() == Some(def.key.as_str()));
            let last = row.and_then(|r| opt_str(r, "last_done_utc"));
            let last_fh: f64 = row.and_then(|r| r.try_get("last_done_fh").ok()).unwrap_or(0.0);
            let last_fc: f64 = row.and_then(|r| r.try_get("last_done_fc").ok()).unwrap_or(0.0);
            let interval_h = def.days.map(|d| d * 24.0).or(def.hours);
            let due = last
                .as_deref()
                .and_then(parse_utc)
                .zip(interval_h)
                .map(|(t, h)| t + chrono::Duration::minutes((h * 60.0) as i64));
            let mut left: Option<f64> = due.map(|d| (d - now).num_minutes() as f64 / 60.0);
            if let Some(fh) = def.flight_hours {
                let l = last_fh + fh - a.counters.flight_hours;
                left = Some(left.map_or(l, |x| x.min(l)));
            }
            let fc_over = def
                .cycles
                .is_some_and(|fc| a.counters.cycles >= last_fc + fc);
            TlCheck {
                key: def.key.clone(),
                name: def.name.clone(),
                last_done_utc: last,
                due_utc: due.map(|d| d.to_rfc3339()),
                overdue: left.is_some_and(|l| l <= 0.0) || fc_over,
                hours_left: left.map(|l| (l * 10.0).round() / 10.0),
            }
        })
        .collect();
    conn.close().await.ok();

    Ok(Some(TechLogAircraft {
        source,
        app_name: source.app_name().into(),
        tablet_url: format!("{}/", source.base_url()),
        running: false,
        active: false,
        airframe_id: a.id,
        registration: a.registration.clone(),
        variant: a.variant,
        engine: a.engine,
        maintenance: a.maintenance,
        flight_hours: (a.counters.flight_hours * 10.0).round() / 10.0,
        cycles: a.counters.cycles,
        egt_margin_c: a.egt_margin,
        dispatch: dispatch_of(&defects),
        defects,
        parts,
        condition,
        consumables,
        checks,
        last_seen_utc: a.last_seen_utc,
        order_wait: Some("sim_closed".into()),
        pending_work: None,
    }))
}

// ---------------------------------------------------------------------------
// API en vivo
// ---------------------------------------------------------------------------

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .no_proxy()
            .build()
            .unwrap_or_default()
    })
}

async fn api_get(source: TlSource, path: &str) -> Result<Value> {
    let r = http()
        .get(format!("{}{path}", source.base_url()))
        .timeout(Duration::from_secs(4))
        .send()
        .await?
        .error_for_status()?;
    Ok(r.json().await?)
}

async fn api_post(source: TlSource, path: &str, body: Option<Value>) -> Result<(bool, String)> {
    let mut req = http()
        .post(format!("{}{path}", source.base_url()))
        .header("X-TechLog", "1");
    req = match body {
        Some(b) => req.json(&b),
        None => req.header("Content-Type", "application/json").body("{}"),
    };
    let r = req.send().await?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(status.is_success());
    let msg = v
        .get("message")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((ok, msg))
}

pub async fn is_running(source: TlSource) -> bool {
    api_get(source, "/api/ping").await.is_ok()
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_string())
}
fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// Estado del avión ACTIVO desde la API (cifras exactas de TechLog).
async fn aircraft_live(source: TlSource, state: &Value) -> Result<TechLogAircraft> {
    let af = state.get("airframe").cloned().unwrap_or(Value::Null);
    let comps = api_get(source, "/api/components").await.unwrap_or(Value::Null);
    let cons = api_get(source, "/api/consumables").await.unwrap_or(Value::Null);
    let arr = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let defects: Vec<TlDefect> = arr(state.get("defects").unwrap_or(&Value::Null))
        .iter()
        .filter(|d| matches!(s(d, "status").as_deref(), Some("Open") | Some("Deferred")))
        .map(|d| TlDefect {
            id: d.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
            ata: d.get("ata").and_then(|x| x.as_i64()).unwrap_or(0),
            title: s(d, "title").unwrap_or_default(),
            detail: s(d, "detail"),
            // La API da "NO-GO" o la letra ("A".."D"); la base, "NoGo" o la letra.
            category: match s(d, "category").unwrap_or_default().as_str() {
                "NO-GO" | "NoGo" => "NoGo".to_string(),
                other => other.to_string(),
            },
            status: s(d, "status").unwrap_or_default(),
            mel_id: s(d, "melId"),
            position: s(d, "position"),
            raised_utc: s(d, "raisedUtc"),
            deferred_until: s(d, "deferredUntil"),
            latest_deferral: s(d, "latestDeferral"),
            can_defer: d.get("canDefer").and_then(|x| x.as_bool()).unwrap_or(false),
            failure: s(d, "fenixFailureName").or_else(|| s(d, "fenixFailure")),
            placard: s(d, "placard"),
            restrictions: s(d, "restrictions"),
        })
        .collect();
    let mut condition = TlCondition::default();
    let mut parts: Vec<TlPart> = Vec::new();
    for c in arr(&comps) {
        let status = s(&c, "status").unwrap_or_else(|| "Ok".into());
        let remaining = f(&c, "remainingPct");
        let worn = c.get("worn").and_then(|x| x.as_bool()).unwrap_or(false);
        condition.count(&status, worn, remaining);
        if status != "Ok" || worn {
            parts.push(TlPart {
                id: c.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
                key: s(&c, "key").unwrap_or_default(),
                name: s(&c, "name").unwrap_or_default(),
                position: s(&c, "position"),
                ata: c.get("ata").and_then(|x| x.as_i64()).unwrap_or(0),
                system: s(&c, "system").unwrap_or_default(),
                kind: s(&c, "kind").unwrap_or_else(|| "Life".into()).to_lowercase(),
                remaining_pct: (remaining * 10.0).round() / 10.0,
                status,
                repairs: c.get("repairs").and_then(|x| x.as_i64()).unwrap_or(0),
            });
        }
    }
    parts.sort_by(|x, y| x.remaining_pct.total_cmp(&y.remaining_pct));
    let consumables = arr(&cons)
        .iter()
        .map(|c| {
            // TechLog: "NO-GO" | "LOW" | "SERVICE SOON" | "OK".
            let raw = s(c, "status").unwrap_or_default().to_uppercase();
            let status = if raw == "NO-GO" {
                "below_min"
            } else if raw == "LOW" || raw == "SERVICE SOON" {
                "low"
            } else {
                "ok"
            };
            TlConsumable {
                key: s(c, "key").unwrap_or_default(),
                position: s(c, "position"),
                name: s(c, "name").unwrap_or_default(),
                unit: s(c, "unit").unwrap_or_default(),
                level: (f(c, "level") * 10.0).round() / 10.0,
                capacity: f(c, "capacity"),
                min_dispatch: f(c, "minDispatch"),
                low: f(c, "low"),
                fills: c.get("fills").and_then(|x| x.as_bool()).unwrap_or(false),
                status: status.into(),
                service_kind: s(c, "serviceKind").unwrap_or_default(),
                service_label: s(c, "serviceLabel").unwrap_or_else(|| "Fill".into()),
                follows_sim: c.get("followsFenix").and_then(|x| x.as_bool()).unwrap_or_else(|| {
                    s(c, "key").as_deref() == Some("ENG_OIL")
                        && install_dir(source).is_some_and(|d| oil_follows_sim(source, &d))
                }),
            }
        })
        .collect();
    let checks = arr(state.get("checks").unwrap_or(&Value::Null))
        .iter()
        .map(|c| TlCheck {
            key: s(c, "key").unwrap_or_default(),
            name: s(c, "name").unwrap_or_default(),
            last_done_utc: s(c, "lastDoneUtc"),
            due_utc: s(c, "dueUtc"),
            overdue: c.get("overdue").and_then(|x| x.as_bool()).unwrap_or(false),
            hours_left: c.get("hoursLeft").and_then(|x| x.as_f64()),
        })
        .collect();
    let dispatch = state
        .get("dispatch")
        .map(|d| TlDispatch {
            status: s(d, "status").unwrap_or_else(|| "GO".into()),
            reasons: d
                .get("reasons")
                .and_then(|r| r.as_array())
                .map(|r| r.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        })
        .unwrap_or_else(|| dispatch_of(&defects));
    Ok(TechLogAircraft {
        source,
        app_name: source.app_name().into(),
        tablet_url: format!("{}/", source.base_url()),
        running: true,
        active: true,
        airframe_id: af.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
        registration: s(&af, "registration").unwrap_or_default(),
        variant: s(&af, "variant").unwrap_or_default(),
        engine: s(&af, "engine").unwrap_or_default(),
        maintenance: s(&af, "maintenance").unwrap_or_else(|| "standard".into()),
        flight_hours: (f(&af, "flightHours") * 10.0).round() / 10.0,
        cycles: f(&af, "cycles"),
        egt_margin_c: af
            .get("egtMarginC")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default(),
        dispatch,
        defects,
        parts,
        condition,
        consumables,
        checks,
        last_seen_utc: None,
        order_wait: match readiness_of(state, &normalize_reg(&s(&af, "registration").unwrap_or_default())) {
            Readiness::Ready => None,
            Readiness::Waiting(why) => Some(why.into()),
        },
        pending_work: s(state, "pendingWork").filter(|w| !w.trim().is_empty()),
    })
}

/// Estado TechLog de una matrícula (en vivo si es el avión activo de un
/// TechLog abierto; si no, desde su base en solo lectura). `None` si ninguna
/// app TechLog conoce ese avión.
pub async fn aircraft(registration: &str) -> Result<Option<TechLogAircraft>> {
    let reg = normalize_reg(registration);
    if reg.is_empty() {
        return Ok(None);
    }
    for source in TlSource::ALL {
        let Some(dir) = install_dir(source) else { continue };
        let running = is_running(source).await;
        let mut live_state = None;
        if running {
            if let Ok(state) = api_get(source, "/api/state").await {
                let active_reg = state
                    .get("airframe")
                    .and_then(|a| s(a, "registration"))
                    .map(|r| normalize_reg(&r));
                if active_reg.as_deref() == Some(reg.as_str()) {
                    return aircraft_live(source, &state).await.map(Some);
                }
                live_state = Some(state);
            }
        }
        if let Some(db) = db_path(source, &dir) {
            if let Some(mut a) = aircraft_from_db(source, &dir, &db, &reg).await? {
                a.running = running;
                if let Some(Readiness::Waiting(why)) = live_state.as_ref().map(|st| readiness_of(st, &reg)) {
                    a.order_wait = Some(why.into());
                }
                return Ok(Some(a));
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// (23/10) Cabina del Fenix — página "Cabin" de Fenix TechLog 2.1
// ---------------------------------------------------------------------------

/// Estado de cabina que TechLog lee del Fenix CARGADO en el sim (agua y
/// residuos, puertas y toboganes, baños, temperatura, presión, luces y
/// señales). `values` lleva las claves de TechLog (`cabWaterPct`,
/// `cabDoorD1L`…): las booleanas llegan como 0/1.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCabin {
    /// `false` mientras el Fenix aún no entrega valores (~15 s tras cargar).
    pub live: bool,
    pub utc: Option<String>,
    /// "Bright" | "Dim1" | "Dim2" | "Off".
    pub lights: Option<String>,
    pub can_set_lights: bool,
    pub can_set_preselect: bool,
    pub values: HashMap<String, f64>,
}

fn parse_cabin(v: &Value) -> TlCabin {
    let values = v
        .get("values")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, x)| {
                    x.as_f64()
                        .or_else(|| x.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
                        .map(|n| (k.clone(), n))
                })
                .collect()
        })
        .unwrap_or_default();
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    TlCabin {
        live: flag("live"),
        utc: s(v, "utc"),
        lights: s(v, "lights"),
        can_set_lights: flag("canSetLights"),
        can_set_preselect: flag("canSetPreselect"),
        values,
    }
}

/// App TechLog abierta cuyo avión ACTIVO es esta matrícula (la cabina es la
/// del avión cargado en el sim: nunca se cambia de avión para esto).
async fn active_source_for(reg: &str) -> Option<TlSource> {
    for source in TlSource::ALL {
        if install_dir(source).is_none() || !is_running(source).await {
            continue;
        }
        let Ok(state) = api_get(source, "/api/state").await else { continue };
        let active = state
            .get("airframe")
            .and_then(|a| s(a, "registration"))
            .map(|r| normalize_reg(&r));
        if active.as_deref() == Some(reg) {
            return Some(source);
        }
    }
    None
}

/// Cabina de una matrícula. `None` si no es el avión activo de un TechLog
/// abierto o si esa versión de TechLog no tiene la página (antes de 2.1).
pub async fn cabin(registration: &str) -> Result<Option<TlCabin>> {
    let reg = normalize_reg(registration);
    let Some(source) = active_source_for(&reg).await else {
        return Ok(None);
    };
    match api_get(source, "/api/cabin-systems").await {
        Ok(v) => Ok(Some(parse_cabin(&v))),
        Err(e) => {
            tracing::debug!(target: "techlog", "{} sin página de cabina: {e}", source.id());
            Ok(None)
        }
    }
}

/// (23/10) Lecturas en vivo del Fenix (TechLog 2.2, `/api/fenix-readings`):
/// grupos con etiqueta, valor, unidad y decimales, tal como los da TechLog.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlReadingItem {
    pub label: String,
    pub value: Option<f64>,
    pub unit: String,
    pub digits: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlReadingGroup {
    pub name: String,
    pub items: Vec<TlReadingItem>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlReadings {
    /// `false` mientras el Fenix no da ningún valor.
    pub live: bool,
    pub groups: Vec<TlReadingGroup>,
}

fn parse_readings(v: &Value) -> TlReadings {
    let groups: Vec<TlReadingGroup> = v
        .get("groups")
        .and_then(|g| g.as_array())
        .map(|gs| {
            gs.iter()
                .map(|g| TlReadingGroup {
                    name: s(g, "name").unwrap_or_default(),
                    items: g
                        .get("items")
                        .and_then(|i| i.as_array())
                        .map(|is| {
                            is.iter()
                                .map(|i| TlReadingItem {
                                    label: s(i, "label").unwrap_or_default(),
                                    value: i.get("value").and_then(|x| x.as_f64()),
                                    unit: i.get("unit").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                                    digits: i.get("digits").and_then(|x| x.as_u64()).unwrap_or(0).min(4) as u32,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let live = groups.iter().any(|g| g.items.iter().any(|i| i.value.is_some()));
    TlReadings { live, groups }
}

/// Lecturas del Fenix cargado. `None` si esa matrícula no es el avión activo
/// de un TechLog abierto o si esa versión no tiene la página (antes de 2.2).
pub async fn readings(registration: &str) -> Result<Option<TlReadings>> {
    let reg = normalize_reg(registration);
    let Some(source) = active_source_for(&reg).await else {
        return Ok(None);
    };
    match api_get(source, "/api/fenix-readings").await {
        Ok(v) => Ok(Some(parse_readings(&v))),
        Err(e) => {
            tracing::debug!(target: "techlog", "{} sin lecturas del Fenix: {e}", source.id());
            Ok(None)
        }
    }
}

/// Luces de cabina ("Bright"/"Dim1"/"Dim2"/"Off") o pre-select de agua
/// (0–100 en pasos de 25) del Fenix cargado. Sólo con ese avión activo.
pub async fn cabin_set(registration: &str, control: &str, value: &Value) -> Result<TlActionResult> {
    let reg = normalize_reg(registration);
    let source = active_source_for(&reg)
        .await
        .ok_or_else(|| anyhow!("not_active"))?;
    let (path, body) = match control {
        "lights" => {
            let text = value.as_str().filter(|t| matches!(*t, "Bright" | "Dim1" | "Dim2" | "Off"));
            let text = text.ok_or_else(|| anyhow!("luces no válidas"))?;
            ("/api/cabin-systems/lights", serde_json::json!({ "text": text }))
        }
        "preselect" => {
            let n = value.as_i64().filter(|n| (0..=100).contains(n) && n % 25 == 0);
            let n = n.ok_or_else(|| anyhow!("pre-select no válido"))?;
            ("/api/cabin-systems/preselect", serde_json::json!({ "number": n }))
        }
        other => return Err(anyhow!("control desconocido: {other}")),
    };
    let (ok, message) = api_post(source, path, Some(body)).await?;
    tracing::info!(target: "techlog", "{} {reg} cabina {control}={value}: ok={ok} {message}", source.id());
    Ok(TlActionResult { ok, message, queued: false })
}

// ---------------------------------------------------------------------------
// Matrículas gestionadas por TechLog ("manda TechLog")
// ---------------------------------------------------------------------------

static MANAGED: OnceLock<RwLock<(Option<Instant>, HashMap<String, TlSource>)>> = OnceLock::new();

/// Matrículas (normalizadas) que tienen avión en algún TechLog → SimFleet no
/// les aplica su propio desgaste. Cacheado 60 s.
pub async fn managed_registrations() -> HashMap<String, TlSource> {
    let cell = MANAGED.get_or_init(|| RwLock::new((None, HashMap::new())));
    if let Ok(g) = cell.read() {
        if g.0.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return g.1.clone();
        }
    }
    let mut map = HashMap::new();
    for source in TlSource::ALL {
        let Some(dir) = install_dir(source) else { continue };
        let Some(db) = db_path(source, &dir) else { continue };
        if let Ok(mut conn) = open_snapshot(source, &db).await {
            if let Ok(afs) = db_airframes(&mut conn).await {
                for a in afs {
                    let r = normalize_reg(&a.registration);
                    if !r.is_empty() {
                        map.entry(r).or_insert(source);
                    }
                }
            }
            conn.close().await.ok();
        }
    }
    if let Ok(mut g) = cell.write() {
        *g = (Some(Instant::now()), map.clone());
    }
    map
}

fn invalidate_managed() {
    if let Some(cell) = MANAGED.get() {
        if let Ok(mut g) = cell.write() {
            g.0 = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Acciones
// ---------------------------------------------------------------------------
//
// La API de TechLog actúa sobre su avión ACTIVO. (24/10) Una orden sólo va a
// TechLog con ESE avión cargado en el simulador y estacionado: con el sim
// cerrado TechLog la apuntaba en su base, el avión no la recibía y SimFleet
// la cobraba igual. Si no, queda en cola (`techlog_pending`) y sale sola
// cuando cargues el avión. Se cobra lo que TechLog termina (con tiempos
// realistas, cuando acaba su `pendingWork`). SimFleet ya no abre TechLog ni
// le cambia el avión para una orden.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlActionRequest {
    pub registration: String,
    /// "fix" | "defer" | "partFix" | "partReplace" | "service" | "check" |
    /// "callMaintenance" | "deferAll"
    pub action: String,
    pub id: Option<i64>,
    pub ids: Option<Vec<i64>>,
    /// Fecha `AAAA-MM-DD` para diferir.
    pub until: Option<String>,
    /// `serviceKind` del consumible o clave del check.
    pub key: Option<String>,
    /// (20/10) Texto de la orden para la cola ("Reparar · Brake 3").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlActionResult {
    pub ok: bool,
    pub message: String,
    /// (20/10) La orden quedó en cola (el sim tiene otro avión cargado).
    pub queued: bool,
}

/// Ejecutable de la app (el mismo que lanza su `.bat`).
fn exe_path(source: TlSource, dir: &Path) -> Option<PathBuf> {
    let name = match source {
        TlSource::Fenix => "TechLog.exe",
        TlSource::Pmdg => "PMDGTechLog.exe",
        TlSource::Simfleet => return None,
    };
    [dir.join("app").join(name), dir.join(name)].into_iter().find(|p| p.is_file())
}

/// Abre TechLog minimizado y espera a que su API responda. `Ok(false)` si ya
/// estaba abierto. Sólo puede haber uno: si después el simulador lo arranca,
/// esa segunda copia se cierra sola (TechLog usa un mutex por puerto).
pub async fn launch(source: TlSource) -> Result<bool> {
    if is_running(source).await {
        return Ok(false);
    }
    let dir = install_dir(source).ok_or_else(|| anyhow!("not_installed"))?;
    let exe = exe_path(source, &dir).ok_or_else(|| anyhow!("not_installed"))?;
    // Como un doble clic: sin argumentos (salvo la copia del entorno de pruebas).
    let mut args = String::new();
    if let Some(p) = test_override(source, "PORT") {
        args.push_str(&format!(" --port {p}"));
    }
    if let Some(a) = test_override(source, "ARGS") {
        args.push(' ');
        args.push_str(&a);
    }
    spawn_minimized(&exe, &dir, &args)?;
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(45) {
        tokio::time::sleep(Duration::from_millis(700)).await;
        if is_running(source).await {
            tracing::info!(target: "techlog", "{} abierto por SimFleet en {:.1} s", source.app_name(), t0.elapsed().as_secs_f32());
            invalidate_managed();
            return Ok(true);
        }
    }
    Err(anyhow!("launch_timeout"))
}

#[cfg(windows)]
fn spawn_minimized(exe: &Path, dir: &Path, args: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `start /MIN` abre su consola minimizada; la de cmd ni se ve.
    std::process::Command::new("cmd")
        .raw_arg(format!("/C start \"\" /MIN \"{}\"{args}", exe.display()))
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("no se pudo abrir TechLog")?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_minimized(_exe: &Path, _dir: &Path, _args: &str) -> Result<()> {
    Err(anyhow!("not_installed"))
}

/// (24/10) ¿Puede TechLog hacer ya una orden para un avión?
#[derive(Debug, PartialEq)]
enum Readiness {
    Ready,
    /// Por qué espera: `sim_closed` | `not_loaded` | `not_parked`.
    Waiting(&'static str),
}

/// Lista sólo si el simulador está conectado con ESE avión cargado
/// (`sim.registration`: TechLog sólo lo da cuando el avión cargado es suyo y
/// manda sobre el activo) y estacionado, como pide TechLog para mantenimiento.
fn readiness_of(state: &Value, reg: &str) -> Readiness {
    let sim = state.get("sim");
    let flag = |k: &str| sim.and_then(|x| x.get(k)).and_then(|x| x.as_bool()).unwrap_or(false);
    if !flag("connected") {
        return Readiness::Waiting("sim_closed");
    }
    let norm = |v: Option<&Value>| v.and_then(|x| s(x, "registration")).map(|r| normalize_reg(&r));
    let loaded = norm(sim);
    let active = norm(state.get("airframe"));
    if loaded.as_deref() != Some(reg) || active.as_deref() != Some(reg) {
        return Readiness::Waiting("not_loaded");
    }
    if !flag("parked") {
        return Readiness::Waiting("not_parked");
    }
    Readiness::Ready
}

async fn readiness(source: TlSource, reg: &str) -> Readiness {
    if !is_running(source).await {
        return Readiness::Waiting("sim_closed");
    }
    match api_get(source, "/api/state").await {
        Ok(state) => readiness_of(&state, reg),
        Err(_) => Readiness::Waiting("sim_closed"),
    }
}

pub async fn action(pool: &SqlitePool, req: TlActionRequest) -> Result<TlActionResult> {
    let reg = normalize_reg(&req.registration);
    let source = managed_registrations()
        .await
        .get(&reg)
        .copied()
        .ok_or_else(|| anyhow!("not_techlog"))?;
    if let Readiness::Waiting(why) = readiness(source, &reg).await {
        enqueue(pool, source, &reg, &req).await?;
        tracing::info!(target: "techlog", "{} {} {}: en cola ({why})", source.id(), reg, req.action);
        return Ok(TlActionResult { ok: true, message: why.into(), queued: true });
    }
    let (ok, message) = perform(source, &req).await?;
    tracing::info!(target: "techlog", "{} {} {}: ok={ok} {message}", source.id(), reg, req.action);
    // Con el avión en el sim, de paso, lo que tuviera en cola.
    let applied = apply_queued(pool, source, &reg).await.len();
    if ok || applied > 0 {
        // Se cobra lo que TechLog ya terminó (con trabajo en curso, al acabar).
        if let Err(e) = sync_charges(pool, source).await {
            tracing::warn!(target: "techlog", "cobros tras acción: {e}");
        }
    }
    Ok(TlActionResult { ok, message, queued: false })
}

/// Manda una orden al avión ACTIVO de TechLog.
async fn perform(source: TlSource, req: &TlActionRequest) -> Result<(bool, String)> {
    let id = || req.id.ok_or_else(|| anyhow!("falta id"));
    Ok(match req.action.as_str() {
        "fix" => api_post(source, &format!("/api/defects/{}/fix", id()?), None).await?,
        "defer" => {
            let until = req.until.clone().ok_or_else(|| anyhow!("falta la fecha"))?;
            api_post(
                source,
                &format!("/api/defects/{}/defer", id()?),
                Some(serde_json::json!({ "until": until })),
            )
            .await?
        }
        "deferAll" => api_post(source, "/api/defects/defer-all", None).await?,
        "partFix" | "partReplace" => {
            let ids = req.ids.clone().or_else(|| req.id.map(|i| vec![i])).unwrap_or_default();
            api_post(
                source,
                "/api/parts/work",
                Some(serde_json::json!({
                    "action": if req.action == "partFix" { "fix" } else { "replace" },
                    "ids": ids,
                })),
            )
            .await?
        }
        "service" => {
            let kind = req.key.clone().ok_or_else(|| anyhow!("falta el servicio"))?;
            api_post(source, "/api/service", Some(serde_json::json!({ "kind": kind }))).await?
        }
        "check" => {
            let key = req.key.clone().ok_or_else(|| anyhow!("falta el check"))?;
            api_post(source, &format!("/api/checks/{key}/perform"), None).await?
        }
        "callMaintenance" => api_post(source, "/api/maintenance/call", None).await?,
        other => return Err(anyhow!("acción desconocida: {other}")),
    })
}

// ---------------------------------------------------------------------------
// Cola de órdenes (avión que TechLog no deja elegir en ese momento)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlPendingOrder {
    pub id: i64,
    pub source: TlSource,
    pub registration: String,
    pub action: String,
    pub label: Option<String>,
    pub created_at: String,
}

/// Una orden de la cola ya aplicada (para avisar al usuario).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlPendingResult {
    pub registration: String,
    pub label: String,
    pub ok: bool,
    pub message: String,
}

async fn enqueue(pool: &SqlitePool, source: TlSource, reg: &str, req: &TlActionRequest) -> Result<()> {
    let json = serde_json::to_string(req)?;
    let dup: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM techlog_pending WHERE status = 'pending' AND registration = ?1 AND request = ?2",
    )
    .bind(reg)
    .bind(&json)
    .fetch_optional(pool)
    .await?;
    if dup.is_none() {
        sqlx::query(
            "INSERT INTO techlog_pending (source, registration, action, label, request, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(source.id())
        .bind(reg)
        .bind(&req.action)
        .bind(&req.label)
        .bind(&json)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Órdenes en cola de una matrícula.
pub async fn pending(pool: &SqlitePool, registration: &str) -> Result<Vec<TlPendingOrder>> {
    let rows = sqlx::query(
        "SELECT id, source, registration, action, label, created_at FROM techlog_pending
         WHERE status = 'pending' AND registration = ?1 ORDER BY id",
    )
    .bind(normalize_reg(registration))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| TlPendingOrder {
            id: r.try_get("id").unwrap_or(0),
            source: TlSource::parse(&r.try_get::<String, _>("source").unwrap_or_default()).unwrap_or(TlSource::Fenix),
            registration: r.try_get("registration").unwrap_or_default(),
            action: r.try_get("action").unwrap_or_default(),
            label: r.try_get("label").ok().flatten(),
            created_at: r.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

pub async fn cancel_pending(pool: &SqlitePool, id: i64) -> Result<()> {
    mark(pool, id, "cancelled", "").await;
    Ok(())
}

async fn mark(pool: &SqlitePool, id: i64, status: &str, message: &str) {
    let r = sqlx::query(
        "UPDATE techlog_pending SET status = ?2, message = ?3, done_at = ?4 WHERE id = ?1 AND status = 'pending'",
    )
    .bind(id)
    .bind(status)
    .bind(message)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await;
    if let Err(e) = r {
        tracing::warn!(target: "techlog", "cola: no se pudo marcar la orden {id}: {e}");
    }
}

/// Aplica la cola de `reg`, que ya debe ser el avión activo.
async fn apply_queued(pool: &SqlitePool, source: TlSource, reg: &str) -> Vec<TlPendingResult> {
    let rows = sqlx::query(
        "SELECT id, action, label, request FROM techlog_pending
         WHERE status = 'pending' AND source = ?1 AND registration = ?2 ORDER BY id",
    )
    .bind(source.id())
    .bind(reg)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let id: i64 = r.try_get("id").unwrap_or(0);
        let action: String = r.try_get("action").unwrap_or_default();
        let label = r.try_get::<Option<String>, _>("label").ok().flatten().unwrap_or_else(|| action.clone());
        let request: String = r.try_get("request").unwrap_or_default();
        let (ok, message) = match serde_json::from_str::<TlActionRequest>(&request) {
            Ok(req) => perform(source, &req).await.unwrap_or_else(|e| (false, e.to_string())),
            Err(e) => (false, format!("orden ilegible: {e}")),
        };
        mark(pool, id, if ok { "applied" } else { "failed" }, &message).await;
        tracing::info!(target: "techlog", "cola {} {reg} {action}: ok={ok} {message}", source.id());
        out.push(TlPendingResult { registration: reg.to_string(), label, ok, message });
    }
    out
}

/// Con TechLog abierto: aplica la cola del avión que está cargado y
/// estacionado en el simulador (las demás esperan a que lo cargues).
pub async fn process_pending(pool: &SqlitePool, source: TlSource) -> Vec<TlPendingResult> {
    let regs: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT registration FROM techlog_pending WHERE status = 'pending' AND source = ?1",
    )
    .bind(source.id())
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if regs.is_empty() || !is_running(source).await {
        return Vec::new();
    }
    let Ok(state) = api_get(source, "/api/state").await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for reg in regs {
        if readiness_of(&state, &reg) == Readiness::Ready {
            out.extend(apply_queued(pool, source, &reg).await);
        }
    }
    if !out.is_empty() {
        if let Err(e) = sync_charges(pool, source).await {
            tracing::warn!(target: "techlog", "cobros tras la cola: {e}");
        }
    }
    out
}

/// Aplica la cola y avisa al front si algo se hizo.
async fn drain_queue(app: &tauri::AppHandle, pool: &SqlitePool, source: TlSource) {
    let done = process_pending(pool, source).await;
    if !done.is_empty() {
        let _ = app.emit("techlog://pending", &done);
        let _ = app.emit("techlog://changed", source.id());
    }
}

// ---------------------------------------------------------------------------
// Cobros en Finanzas
// ---------------------------------------------------------------------------

// (24/10) Los precios salen de `maint_prices` (piezas a precio de intercambio,
// mano de obra de la región del aeropuerto, checks por horas-hombre). Aquí
// sólo queda el precio por ATA de antes, como red para piezas sin tabla.

fn part_cost(cat: &Catalog, ata: Option<i64>) -> f64 {
    part_cost_by_ata(ata, &cat.spares.common_ata, &cat.spares.rare_ata)
}

/// (19/10) Coste de una pieza nueva según su ATA (repuesto común / normal /
/// raro o de motor). (24/10) Sólo para piezas que no están en la tabla de
/// precios. Compartido con el Tech Log propio (`mx`).
pub(crate) fn part_cost_by_ata(ata: Option<i64>, common: &[i64], rare: &[i64]) -> f64 {
    match ata {
        Some(a) if rare.contains(&a) || a == 72 => 60_000.0,
        Some(a) if common.contains(&a) => 900.0,
        _ => 7_500.0,
    }
}

/// (24/10) Flota de precios de un avión de TechLog. (25/10) PMDG TechLog
/// 0.2 lleva también el 777: precios de fuselaje ancho.
fn fleet_of(source: TlSource, variant: &str) -> crate::maint_prices::Fleet {
    match source {
        TlSource::Pmdg if is_777(variant) => crate::maint_prices::Fleet::Widebody,
        TlSource::Pmdg => crate::maint_prices::Fleet::B737,
        TlSource::Fenix | TlSource::Simfleet => crate::maint_prices::Fleet::A320,
    }
}

/// (24/10) De qué sale un cobro. El importe se calcula sólo para lo nuevo,
/// con la tarifa del aeropuerto donde está el avión.
enum Basis {
    Free,
    Part { key: Option<String>, ata: Option<i64>, repair: bool },
    Consumable { key: String, amount: f64 },
    Defect { hours: f64, material: f64 },
    Check(String),
}

impl Basis {
    /// USD, antes del multiplicador del nivel de mantenimiento.
    fn cost(&self, cat: &Catalog, fleet: crate::maint_prices::Fleet, rate: f64) -> f64 {
        use crate::maint_prices as mp;
        match self {
            Basis::Free => 0.0,
            Basis::Part { key, ata, repair } => {
                let priced = key.as_deref().and_then(|k| {
                    if *repair {
                        mp::repair_cost(k, fleet, rate)
                    } else {
                        mp::replace_cost(k, fleet, rate)
                    }
                });
                priced.unwrap_or_else(|| mp::fallback_part(part_cost(cat, *ata), *repair, rate))
            }
            Basis::Consumable { key, amount } => mp::consumable_cost(key, *amount, rate),
            Basis::Defect { hours, material } => mp::defect_cost(*hours, *material, rate),
            Basis::Check(key) => mp::check_cost(key, fleet, rate),
        }
    }
}

/// Qué cobra un defecto rectificado de TechLog (`None`: reemplazado por
/// otro). Si se arregló cambiando o reparando la pieza, haciendo el check o
/// sirviendo el consumible ("Servicing required: …"), eso ya se cobra (la
/// pieza con su mano de obra, el check o el servicio): aquí no se cobra dos
/// veces. Un "finding" lleva su material.
fn defect_basis(cat: &Catalog, r: &sqlx::sqlite::SqliteRow) -> Option<Basis> {
    let how = opt_str(r, "rectified_how").unwrap_or_default();
    if how.contains("superseded") {
        return None;
    }
    let hours = opt_str(r, "mel_id")
        .as_deref()
        .and_then(|m| cat.mel.get(m))
        .and_then(|m| m.fix_hours)
        .unwrap_or(1.0)
        .max(0.5);
    let comp = opt_str(r, "component_key").unwrap_or_default();
    Some(if how.contains("part") || comp.starts_with("CHECK:") || comp.starts_with("CONS:") {
        Basis::Free
    } else {
        let material = comp
            .strip_prefix("FINDING:")
            .map(crate::maint_prices::finding_material)
            .unwrap_or(0.0);
        Basis::Defect { hours, material }
    })
}

/// Qué cobra y con qué etiqueta un registro de `servicing` de TechLog.
fn servicing_charge(cat: &Catalog, kind: &str, before: f64, after: f64) -> (Basis, String) {
    for (prefix, repair, label) in [("Replaced: ", false, "part_replace"), ("Repaired: ", true, "part_repair")] {
        if let Some(name) = kind.strip_prefix(prefix) {
            let basis = Basis::Part { key: cat.key_by_name(name), ata: cat.ata_by_name(name), repair };
            return (basis, format!("techlog:{label}:{name}"));
        }
    }
    let key = kind.split(':').next().unwrap_or(kind).to_string();
    // Lavado de motor: uno por motor (`ENGINE_WASH:1`), precio fijo.
    let amount = if key == "ENGINE_WASH" { 1.0 } else { (after - before).abs() };
    (Basis::Consumable { key, amount }, format!("techlog:service:{kind}"))
}

/// (24/10) Servicios de TechLog que GSX factura con su propio recibo (y
/// SimFleet cobra por ahí): catering, agua potable y baños ("Potable water",
/// "Lavatory service" en el recibo de handling).
fn gsx_bills(kind: &str) -> bool {
    kind.split(':')
        .next()
        .is_some_and(|k| ["CATERING", "WATER", "WASTE"].iter().any(|g| k.eq_ignore_ascii_case(g)))
}

struct ChargeEvent {
    key: String,
    component: String,
    basis: Basis,
    at: String,
}

/// Lee los eventos de mantenimiento de TechLog y cobra en Finanzas los que
/// SimFleet aún no había visto. La primera vez por avión sólo registra el
/// historial (sin cobrar). Devuelve cuántos cobró.
pub async fn sync_charges(pool: &SqlitePool, source: TlSource) -> Result<u64> {
    let Some(dir) = install_dir(source) else { return Ok(0) };
    let Some(db) = db_path(source, &dir) else { return Ok(0) };
    // (24/10) Trabajo en curso (tiempos realistas): se cobra cuando TechLog
    // lo termine, no al empezar.
    if is_running(source).await {
        if let Ok(state) = api_get(source, "/api/state").await {
            if let Some(work) = s(&state, "pendingWork").filter(|w| !w.trim().is_empty()) {
                tracing::debug!(target: "techlog", "{}: cobros en espera, trabajo en curso ({work})", source.id());
                return Ok(0);
            }
        }
    }
    sync_charges_at(pool, source, &dir, &db).await
}

/// (24/10) Un evento con fecha futura es trabajo que TechLog guardó con su
/// hora de fin: aún no está hecho ni se cobra (ni se marca como visto).
fn not_done_yet(at: &str) -> bool {
    DateTime::parse_from_rfc3339(at).is_ok_and(|t| t.with_timezone(&Utc) > Utc::now() + chrono::Duration::minutes(1))
}

async fn sync_charges_at(pool: &SqlitePool, source: TlSource, dir: &Path, db: &Path) -> Result<u64> {
    let mut conn = open_snapshot(source, db).await?;
    let airframes = db_airframes(&mut conn).await?;
    let now = Utc::now().to_rfc3339();
    let mut charged = 0u64;
    for a in &airframes {
        let reg = normalize_reg(&a.registration);
        if reg.is_empty() {
            continue;
        }
        // (25/10) Cada avión con el catálogo y los precios de su tipo (777).
        let cat = catalog_for(dir, &a.variant);
        let fleet = fleet_of(source, &a.variant);
        let mut events: Vec<ChargeEvent> = Vec::new();
        for r in sqlx::query("SELECT * FROM servicing WHERE airframe_id = ?1")
            .bind(a.id)
            .fetch_all(&mut conn)
            .await?
        {
            let id: i64 = r.try_get("id").unwrap_or(0);
            let kind = opt_str(&r, "kind").unwrap_or_default();
            let (basis, component) = servicing_charge(
                &cat,
                &kind,
                r.try_get("before").unwrap_or(0.0),
                r.try_get("after").unwrap_or(0.0),
            );
            let at = opt_str(&r, "utc").unwrap_or_else(|| now.clone());
            // (24/10) Catering hecho por GSX: lo cobra el recibo de GSX (que
            // se reparte por vuelo). TechLog sólo lo registra ("%, by GSX");
            // cobrarlo aquí también era pagarlo dos veces. Y se borra el
            // cobro doble que hubiera quedado de antes (idempotente).
            let by_gsx = opt_str(&r, "unit").is_some_and(|u| u.to_ascii_lowercase().contains("by gsx"));
            if by_gsx && gsx_bills(&kind) {
                let removed = sqlx::query(
                    "DELETE FROM maintenance_log WHERE registration = ?1 AND component = ?2 AND serviced_at = ?3",
                )
                .bind(&reg)
                .bind(&component)
                .bind(&at)
                .execute(pool)
                .await?
                .rows_affected();
                let key = format!("svc:{}:{}:{id}", source.id(), a.id);
                if removed > 0 {
                    sqlx::query("UPDATE techlog_seen SET cost = 0 WHERE event_key = ?1")
                        .bind(&key)
                        .execute(pool)
                        .await?;
                    // Que la nube tampoco lo devuelva.
                    let mk = crate::ledger_sync::maint_key(&reg, &component, &at);
                    if let Err(e) = crate::ledger_sync::record(pool, crate::ledger_sync::KIND_MAINT, &mk).await {
                        tracing::warn!(target: "techlog", "anotar cobro quitado: {e:#}");
                    }
                    tracing::info!(target: "techlog", "{} {reg}: quitado cobro doble de {component} (lo factura GSX)", source.id());
                }
                events.push(ChargeEvent { key, component, basis: Basis::Free, at });
                continue;
            }
            events.push(ChargeEvent {
                key: format!("svc:{}:{}:{id}", source.id(), a.id),
                component,
                basis,
                at,
            });
        }
        for r in sqlx::query(
            "SELECT * FROM defect WHERE airframe_id = ?1 AND rectified_utc IS NOT NULL",
        )
        .bind(a.id)
        .fetch_all(&mut conn)
        .await?
        {
            let Some(basis) = defect_basis(&cat, &r) else { continue };
            let id: i64 = r.try_get("id").unwrap_or(0);
            let title = opt_str(&r, "title").unwrap_or_default();
            events.push(ChargeEvent {
                key: format!("def:{}:{}:{id}", source.id(), a.id),
                component: format!("techlog:defect:{}", title),
                basis,
                at: opt_str(&r, "rectified_utc").unwrap_or_else(|| now.clone()),
            });
        }
        for r in sqlx::query("SELECT * FROM check_record WHERE airframe_id = ?1")
            .bind(a.id)
            .fetch_all(&mut conn)
            .await?
        {
            let key = opt_str(&r, "key").unwrap_or_default();
            let Some(done) = opt_str(&r, "last_done_utc") else { continue };
            events.push(ChargeEvent {
                key: format!("chk:{}:{}:{key}:{done}", source.id(), a.id),
                component: format!("techlog:check:{key}"),
                basis: Basis::Check(key.clone()),
                at: done,
            });
        }

        let baseline_key = format!("baseline:{}:{reg}", source.id());
        let has_baseline: Option<String> =
            sqlx::query_scalar("SELECT event_key FROM techlog_seen WHERE event_key = ?1")
                .bind(&baseline_key)
                .fetch_optional(pool)
                .await?;
        let first_time = has_baseline.is_none();
        let mult = if first_time {
            1.0
        } else {
            crate::aircraft_maintenance::level_mult_for_registration(pool, &reg).await
        };
        let seen: HashSet<String> = sqlx::query_scalar("SELECT event_key FROM techlog_seen WHERE registration = ?1")
            .bind(&reg)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
        // Tarifa de la región donde está el avión: sólo si hay algo nuevo.
        let mut rate: Option<f64> = None;
        for e in &events {
            if seen.contains(&e.key) || (!first_time && not_done_yet(&e.at)) {
                continue;
            }
            let cost = if first_time {
                0.0
            } else {
                let r = match rate {
                    Some(r) => r,
                    None => *rate.insert(crate::maint_prices::labor_rate(pool, &reg).await),
                };
                e.basis.cost(&cat, fleet, r) * mult
            };
            let res = sqlx::query(
                "INSERT OR IGNORE INTO techlog_seen (event_key, source, registration, cost, seen_at) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(&e.key)
            .bind(source.id())
            .bind(&reg)
            .bind(cost)
            .bind(&now)
            .execute(pool)
            .await?;
            if res.rows_affected() == 0 || first_time || cost <= 0.0 {
                continue;
            }
            sqlx::query(
                "INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES (?1,?2,?3,?4)",
            )
            .bind(&reg)
            .bind(&e.component)
            .bind(cost.round())
            .bind(&e.at)
            .execute(pool)
            .await?;
            charged += 1;
            tracing::info!(
                target: "techlog",
                "cobro {} {reg}: {} = {} USD",
                source.id(),
                e.component,
                cost.round() as i64
            );
        }
        if first_time {
            sqlx::query(
                "INSERT OR IGNORE INTO techlog_seen (event_key, source, registration, cost, seen_at) VALUES (?1,?2,?3,0,?4)",
            )
            .bind(&baseline_key)
            .bind(source.id())
            .bind(&reg)
            .bind(&now)
            .execute(pool)
            .await?;
            tracing::info!(
                target: "techlog",
                "{} {reg}: historial de TechLog registrado ({} eventos, sin cobrar)",
                source.id(),
                events.len()
            );
        }
    }
    conn.close().await.ok();
    Ok(charged)
}

/// Un defecto cobrado que ya no está rectificado en la base de TechLog (se
/// volvió a abrir o se reinició el avión): se deduce de su título y de lo que
/// costó con la tarifa vieja (horas del MEL × 115 USD/h × nivel).
fn defect_basis_from_charge(title: &str, old: f64, mult: f64) -> Basis {
    let t = title.to_ascii_lowercase();
    if t.starts_with("servicing required") || t.ends_with("check overdue") {
        return Basis::Free;
    }
    let hours = (old / (115.0 * mult.max(0.1)) * 4.0).round() / 4.0;
    Basis::Defect { hours: hours.max(0.5), material: 0.0 }
}

/// (24/10) Recalcula una vez, con los precios reales, los cobros ya hechos de
/// Fenix/PMDG TechLog: checks, piezas y defectos (tarifa de donde está hoy el
/// avión, nivel de mantenimiento actual). Los servicios de consumibles no
/// guardan la cantidad servida: quedan como estaban.
pub async fn reprice_history(pool: &SqlitePool) -> Result<usize> {
    let rows = sqlx::query("SELECT id, registration, component, cost, serviced_at FROM maintenance_log WHERE component LIKE 'techlog:%'")
        .fetch_all(pool)
        .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let managed = managed_registrations().await;
    // Por app TechLog: su base abierta y la variante de cada avión (777).
    type Open = (Option<SqliteConnection>, HashMap<String, String>);
    let mut cats: HashMap<TlSource, Open> = HashMap::new();
    let mut rates: HashMap<String, (f64, f64)> = HashMap::new();
    let mut changed = 0;
    for r in rows {
        let id: i64 = r.try_get("id").unwrap_or(0);
        let reg: String = r.try_get("registration").unwrap_or_default();
        let component: String = r.try_get("component").unwrap_or_default();
        let old: f64 = r.try_get("cost").unwrap_or(0.0);
        let at: String = r.try_get("serviced_at").unwrap_or_default();
        let Some((label, detail)) = component.strip_prefix("techlog:").and_then(|c| c.split_once(':')) else { continue };
        let Some(&source) = managed.get(&reg) else { continue };
        let Some(dir) = install_dir(source) else { continue };
        if let std::collections::hash_map::Entry::Vacant(slot) = cats.entry(source) {
            let mut conn = match db_path(source, &dir) {
                Some(db) => open_snapshot(source, &db).await.ok(),
                None => None,
            };
            let mut variants = HashMap::new();
            if let Some(c) = conn.as_mut() {
                for a in db_airframes(c).await.unwrap_or_default() {
                    variants.insert(normalize_reg(&a.registration), a.variant);
                }
            }
            slot.insert((conn, variants));
        }
        let (rate, mult) = match rates.get(&reg) {
            Some(v) => *v,
            None => {
                let v = (
                    crate::maint_prices::labor_rate(pool, &reg).await,
                    crate::aircraft_maintenance::level_mult_for_registration(pool, &reg).await,
                );
                *rates.entry(reg.clone()).or_insert(v)
            }
        };
        let (conn, variants) = cats.get_mut(&source).expect("base abierta");
        let variant = variants.get(&reg).cloned().unwrap_or_default();
        let cat = &catalog_for(&dir, &variant);
        let basis = match label {
            "check" => Some(Basis::Check(detail.to_string())),
            "part_replace" | "part_repair" => Some(Basis::Part {
                key: cat.key_by_name(detail),
                ata: cat.ata_by_name(detail),
                repair: label == "part_repair",
            }),
            "defect" => match conn.as_mut() {
                Some(c) => {
                    let found = sqlx::query(
                        "SELECT d.*, a.registration AS af_reg FROM defect d JOIN airframe a ON a.id = d.airframe_id
                         WHERE d.title = ?1 AND d.rectified_utc = ?2",
                    )
                    .bind(detail)
                    .bind(&at)
                    .fetch_all(&mut *c)
                    .await
                    .unwrap_or_default();
                    match found.iter().find(|d| opt_str(d, "af_reg").is_some_and(|x| normalize_reg(&x) == reg)) {
                        Some(d) => defect_basis(cat, d),
                        None => Some(defect_basis_from_charge(detail, old, mult)),
                    }
                }
                None => Some(defect_basis_from_charge(detail, old, mult)),
            },
            _ => None,
        };
        let Some(basis) = basis else { continue };
        let new = (basis.cost(cat, fleet_of(source, &variant), rate) * mult).round();
        if (new - old).abs() < 0.5 {
            continue;
        }
        if new <= 0.0 {
            // Ya se cobraba por otro lado (servicio, check, pieza): fuera, y
            // que la nube tampoco lo devuelva.
            sqlx::query("DELETE FROM maintenance_log WHERE id = ?1").bind(id).execute(pool).await?;
            let key = crate::ledger_sync::maint_key(&reg, &component, &at);
            crate::ledger_sync::record(pool, crate::ledger_sync::KIND_MAINT, &key).await?;
            tracing::info!(target: "techlog", "{reg} {component}: {old:.0} USD quitado (ya se cobraba aparte)");
            changed += 1;
            continue;
        }
        sqlx::query("UPDATE maintenance_log SET cost = ?2 WHERE id = ?1").bind(id).bind(new).execute(pool).await?;
        tracing::info!(target: "techlog", "{reg} {component}: {old:.0} → {new:.0} USD (precios reales)");
        changed += 1;
    }
    for (_, (conn, _)) in cats {
        if let Some(c) = conn {
            c.close().await.ok();
        }
    }
    Ok(changed)
}

// ---------------------------------------------------------------------------
// Vuelos de TechLog (FlightBook)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlFlightEvent {
    pub phase: Option<String>,
    pub kind: String,
    pub severity: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlFlightWear {
    pub item: String,
    pub amount: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechLogFlight {
    pub source: TlSource,
    pub app_name: String,
    pub flight_id: i64,
    pub status: String,
    pub score: Option<f64>,
    pub score_notes: Vec<String>,
    pub summary: Option<String>,
    pub events: Vec<TlFlightEvent>,
    pub wear: Vec<TlFlightWear>,
    /// (19/10) Despegó con ítems NO-GO abiertos (violación de despacho).
    pub dispatch_violation: bool,
}

/// Vuelo de TechLog que corresponde a un vuelo del FlightBook: mismo avión y
/// apertura dentro de la ventana del vuelo (±45 min).
pub async fn flight_for(
    registration: &str,
    started_at: &str,
    ended_at: Option<&str>,
) -> Result<Option<TechLogFlight>> {
    let reg = normalize_reg(registration);
    let Some(start) = parse_utc(started_at) else { return Ok(None) };
    let end = ended_at.and_then(parse_utc).unwrap_or(start + chrono::Duration::hours(18));
    let lo = start - chrono::Duration::minutes(45);
    let hi = end + chrono::Duration::minutes(45);
    for source in TlSource::ALL {
        let Some(dir) = install_dir(source) else { continue };
        let Some(db) = db_path(source, &dir) else { continue };
        let mut conn = open_snapshot(source, &db).await?;
        let afs = db_airframes(&mut conn).await?;
        let Some(a) = afs.iter().find(|a| normalize_reg(&a.registration) == reg) else {
            conn.close().await.ok();
            continue;
        };
        let rows = sqlx::query("SELECT * FROM flight WHERE airframe_id = ?1 ORDER BY opened_utc DESC")
            .bind(a.id)
            .fetch_all(&mut conn)
            .await?;
        let found = rows.into_iter().find(|r| {
            opt_str(r, "opened_utc")
                .as_deref()
                .and_then(parse_utc)
                .is_some_and(|t| t >= lo && t <= hi)
        });
        let Some(r) = found else {
            conn.close().await.ok();
            continue;
        };
        let fid: i64 = r.try_get("id").unwrap_or(0);
        let events = sqlx::query("SELECT * FROM flight_event WHERE flight_id = ?1 ORDER BY id")
            .bind(fid)
            .fetch_all(&mut conn)
            .await?
            .iter()
            .map(|e| TlFlightEvent {
                phase: opt_str(e, "phase"),
                kind: opt_str(e, "type").unwrap_or_default(),
                severity: opt_str(e, "severity").unwrap_or_else(|| "Info".into()),
                text: opt_str(e, "text").unwrap_or_default(),
            })
            .collect();
        let wear = sqlx::query("SELECT * FROM flight_wear WHERE flight_id = ?1")
            .bind(fid)
            .fetch_all(&mut conn)
            .await?
            .iter()
            .map(|w| TlFlightWear {
                item: opt_str(w, "item").unwrap_or_default(),
                amount: w.try_get("amount").unwrap_or(0.0),
                unit: opt_str(w, "unit").unwrap_or_default(),
            })
            .filter(|w| w.amount.abs() > 0.0001)
            .collect();
        let score_notes = opt_str(&r, "score_notes")
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();
        // TechLog marca la violación con el flag "dispatch-violation".
        let dispatch_violation = opt_str(&r, "flags")
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .is_some_and(|f| f.iter().any(|x| x == "dispatch-violation"));
        conn.close().await.ok();
        return Ok(Some(TechLogFlight {
            source,
            app_name: source.app_name().into(),
            flight_id: fid,
            status: opt_str(&r, "status").unwrap_or_default(),
            // SQLite puede guardarla como entero, real o texto.
            score: r
                .try_get::<Option<f64>, _>("score")
                .ok()
                .flatten()
                .or_else(|| r.try_get::<Option<i64>, _>("score").ok().flatten().map(|v| v as f64))
                .or_else(|| opt_str(&r, "score").and_then(|v| v.parse().ok())),
            score_notes,
            summary: opt_str(&r, "summary"),
            events,
            wear,
            dispatch_violation,
        }));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Estado general + escucha en vivo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlAppStatus {
    pub source: TlSource,
    pub app_name: String,
    pub installed: bool,
    pub running: bool,
    pub tablet_url: String,
    pub airframes: usize,
}

pub async fn status() -> Vec<TlAppStatus> {
    let managed = managed_registrations().await;
    let mut out = Vec::new();
    for source in TlSource::ALL {
        let installed = install_dir(source).is_some();
        out.push(TlAppStatus {
            source,
            app_name: source.app_name().into(),
            installed,
            running: installed && is_running(source).await,
            tablet_url: format!("{}/", source.base_url()),
            airframes: managed.values().filter(|s| **s == source).count(),
        });
    }
    out
}

/// Hilo de fondo por app TechLog: cuando está abierta escucha su SSE
/// (`/api/events`); ante cambios avisa al front y sincroniza cobros. Cada
/// minuto (abierta o no) revisa también la base por si TechLog cambió algo
/// mientras SimFleet no escuchaba.
pub fn spawn_watchers(app: tauri::AppHandle, pool: SqlitePool) {
    for source in TlSource::ALL {
        let app = app.clone();
        let pool = pool.clone();
        tauri::async_runtime::spawn(async move {
            // Primera pasada: registra historial/cobra lo pendiente.
            tokio::time::sleep(Duration::from_secs(20)).await;
            let mut last_sync = Instant::now() - Duration::from_secs(3600);
            loop {
                if install_dir(source).is_none() {
                    tokio::time::sleep(Duration::from_secs(300)).await;
                    continue;
                }
                if last_sync.elapsed() >= Duration::from_secs(60) {
                    last_sync = Instant::now();
                    match sync_charges(&pool, source).await {
                        Ok(n) if n > 0 => {
                            let _ = app.emit("techlog://changed", source.id());
                        }
                        Ok(_) => {}
                        Err(e) => tracing::debug!(target: "techlog", "sync {}: {e}", source.id()),
                    }
                }
                if !is_running(source).await {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    continue;
                }
                // TechLog abierto: escuchamos sus eventos hasta que se cierre.
                if let Err(e) = listen_events(&app, &pool, source, &mut last_sync).await {
                    tracing::debug!(target: "techlog", "SSE {} cerrado: {e}", source.id());
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
}

async fn listen_events(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    source: TlSource,
    last_sync: &mut Instant,
) -> Result<()> {
    // Sin timeout global: el stream es largo. Cliente propio para no heredar
    // el timeout de 6 s del cliente de consultas.
    let client = reqwest::Client::builder().no_proxy().build()?;
    let mut resp = client
        .get(format!("{}/api/events", source.base_url()))
        .header("Accept", "text/event-stream")
        .send()
        .await?
        .error_for_status()?;
    tracing::info!(target: "techlog", "{} abierto: sincronía en vivo", source.app_name());
    let _ = app.emit("techlog://changed", source.id());
    invalidate_managed();
    // (20/10) Al abrirse, lo que hubiera en cola.
    drain_queue(app, pool, source).await;
    let mut last_queue = Instant::now();
    let mut queue_due = false;
    let mut buf = String::new();
    let mut pending_change = false;
    let mut last_emit = Instant::now() - Duration::from_secs(10);
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(30), resp.chunk()).await;
        match chunk {
            Ok(Ok(Some(bytes))) => {
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(idx) = buf.find("\n\n") {
                    let block: String = buf.drain(..idx + 2).collect();
                    if block.lines().any(|l| l.trim() == "event: state" || l.trim() == "event: notice") {
                        pending_change = true;
                    }
                }
            }
            Ok(Ok(None)) => return Err(anyhow!("fin del stream")),
            Ok(Err(e)) => return Err(e.into()),
            // 30 s sin datos (ni siquiera `tick`): ¿sigue vivo?
            Err(_) => {
                if !is_running(source).await {
                    return Err(anyhow!("TechLog cerrado"));
                }
            }
        }
        if pending_change && last_emit.elapsed() >= Duration::from_millis(1500) {
            pending_change = false;
            last_emit = Instant::now();
            let _ = app.emit("techlog://changed", source.id());
            if last_sync.elapsed() >= Duration::from_secs(5) {
                *last_sync = Instant::now();
                if let Err(e) = sync_charges(pool, source).await {
                    tracing::debug!(target: "techlog", "cobros {}: {e}", source.id());
                }
            }
            // Puede haber cambiado el avión activo (cargaste el de una orden en cola).
            queue_due = true;
        }
        if queue_due && last_queue.elapsed() >= Duration::from_secs(20) {
            queue_due = false;
            last_queue = Instant::now();
            drain_queue(app, pool, source).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (25/10) PMDG TechLog 0.2: el 777 con su catálogo y precios de ancho.
    #[test]
    fn a_pmdg_777_gets_its_own_catalog_and_widebody_prices() {
        use crate::maint_prices::Fleet;
        assert!(is_777("777-300ER") && is_777("777F") && !is_777("737-800BCF"));
        assert_eq!(fleet_of(TlSource::Pmdg, "777-200LR"), Fleet::Widebody);
        assert_eq!(fleet_of(TlSource::Pmdg, "737-900ER"), Fleet::B737);
        assert_eq!(fleet_of(TlSource::Fenix, "A320"), Fleet::A320);
        let root = std::env::temp_dir().join(format!("simfleet-tl-b777-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("catalog/b777")).unwrap();
        std::fs::write(root.join("catalog/components.json"), r#"{"components":[{"key":"BRAKE","name":"Brake {pos}","ata":32,"positions":["1"]}],"consumables":[]}"#).unwrap();
        std::fs::write(root.join("catalog/b777/components.json"), r#"// del 777
{"components":[{"key":"RAT","name":"Ram air turbine","ata":29}],"consumables":[]}"#).unwrap();
        assert_eq!(catalog_for(&root, "777-300ER").key_by_name("Ram air turbine").as_deref(), Some("RAT"));
        assert_eq!(catalog_for(&root, "737-800").key_by_name("Brake 1").as_deref(), Some("BRAKE"));
        assert!(catalog_for(&root, "737-800").key_by_name("Ram air turbine").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Con el PMDG TechLog instalado (sólo lee su catálogo): el del 777
    /// carga con su MEL y límites, y cada pieza tiene precio de ancho.
    #[test]
    #[ignore]
    fn pmdg_777_real_catalog() {
        use crate::maint_prices::{part, Fleet};
        let dir = install_dir(TlSource::Pmdg).expect("PMDG TechLog instalado");
        let cat = catalog_for(&dir, "777-300ER");
        let b737 = catalog_for(&dir, "737-800");
        println!("777: {} piezas, {} consumibles, {} MEL, {} checks; 737: {} piezas",
            cat.components.len(), cat.consumables.len(), cat.mel.len(), cat.limits.checks.len(), b737.components.len());
        assert!(cat.components.len() > 100 && !cat.mel.is_empty() && !cat.limits.checks.is_empty());
        assert!(cat.component("RAT").is_some() && b737.component("RAT").is_none());
        let missing: Vec<&str> = cat
            .components
            .iter()
            .map(|c| c.key.as_str())
            .filter(|k| part(k, Fleet::Widebody).is_none())
            .collect();
        assert!(missing.is_empty(), "sin precio: {missing:?}");
    }

    #[test]
    fn an_old_defect_charge_is_repriced_from_its_title_and_old_cost() {
        let c = |title: &str, old: f64| match defect_basis_from_charge(title, old, 1.0) {
            Basis::Free => None,
            Basis::Defect { hours, .. } => Some(hours),
            _ => unreachable!(),
        };
        assert_eq!(c("Servicing required: Tyre 1 pressure", 58.0), None, "lo cobra el servicio");
        assert_eq!(c("Daily check overdue", 115.0), None, "lo cobra el check");
        assert_eq!(c("Overspeed inspection (VMO/MMO)", 345.0), Some(3.0));
        assert_eq!(c("Captain's map light dim", 58.0), Some(0.5));
    }

    /// (24/10) Una orden sólo sale con ESE avión cargado y estacionado.
    #[test]
    fn orders_wait_for_the_aircraft_loaded_and_parked_in_the_sim() {
        let state = |connected: bool, loaded: Option<&str>, parked: bool| {
            serde_json::json!({
                "sim": { "connected": connected, "registration": loaded, "parked": parked },
                "airframe": { "registration": "PH-BCL" }
            })
        };
        let r = |st: Value| readiness_of(&st, "PHBCL");
        assert_eq!(r(state(false, None, true)), Readiness::Waiting("sim_closed"), "sim cerrado (el caso de la luz de mapa)");
        assert_eq!(r(state(true, None, true)), Readiness::Waiting("not_loaded"), "sim abierto con otro avión");
        assert_eq!(r(state(true, Some("C-GORP"), true)), Readiness::Waiting("not_loaded"));
        assert_eq!(r(state(true, Some("PH-BCL"), false)), Readiness::Waiting("not_parked"));
        assert_eq!(r(state(true, Some("PH-BCL"), true)), Readiness::Ready);
        assert_eq!(readiness_of(&serde_json::json!({}), "PHBCL"), Readiness::Waiting("sim_closed"));
    }

    #[test]
    fn work_saved_with_its_finish_time_is_not_charged_yet() {
        let later = (Utc::now() + chrono::Duration::hours(3)).to_rfc3339();
        assert!(not_done_yet(&later));
        assert!(!not_done_yet("2026-10-02T10:00:00Z"));
        assert!(!not_done_yet("no es una fecha"));
    }

    #[test]
    fn normalizes_techlog_type_suffix() {
        assert_eq!(normalize_reg("G-FENX-A320-CFM"), "GFENX");
        assert_eq!(normalize_reg("N657JB"), "N657JB");
        assert_eq!(normalize_reg("G-SMOL"), "GSMOL");
        assert_eq!(normalize_reg("EC-MAX"), "ECMAX");
    }

    #[test]
    fn strips_jsonc_comments_and_trailing_commas() {
        let src = "{\n // nota\n \"a\": \"http://x\", /* b */ \"c\": [1,2,],\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["a"], "http://x");
        assert_eq!(v["c"], serde_json::json!([1, 2]));
    }

    #[test]
    fn strip_jsonc_keeps_non_ascii() {
        let src = "{\n // 650 °C — nota\n \"name\": \"Válvula 2 °C — ñ \\\"ü\\\"\", /* año */ \"x\": [\"日本\",],\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["name"], "Válvula 2 °C — ñ \"ü\"");
        assert_eq!(v["x"], serde_json::json!(["日本"]));
        // Comentario sin cerrar al final: se descarta sin romper el UTF-8.
        assert_eq!(strip_jsonc("{\"a\": \"°\"} /* ñ"), "{\"a\": \"°\"} ");
    }

    #[test]
    fn wear_value_matches_techlog() {
        let mut c = Counters {
            flight_hours: 100.0,
            eng_hours: vec![10.0, 20.0],
            ..Default::default()
        };
        c.wear_extra.insert("EngHours:2".into(), 1.5);
        let now = Utc::now();
        assert_eq!(c.wear_value("flightHours", 1, now), 100.0);
        assert_eq!(c.wear_value("engHours", 2, now), 21.5);
        assert_eq!(c.wear_value("engHours", 1, now), 10.0);
    }

    /// Cobros: la 1ª pasada registra el historial sin cobrar; lo nuevo se
    /// cobra una sola vez con la tarifa de cada tipo de evento.
    #[tokio::test]
    async fn charges_once_after_baseline() {
        let root = std::env::temp_dir().join(format!("simfleet-tl-charges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("catalog")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(
            root.join("catalog/components.json"),
            r#"{ // piezas
              "components": [{"key":"BRAKE","name":"Brake {pos}","ata":32,"kind":"wear","positions":["1","2"],"life":0}],
              "consumables": [{"key":"ENG_OIL","name":"Engine {pos} oil","unit":"qt","positions":["1","2"],"capacity":20}],
            }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("catalog/mel.json"),
            r#"{"items":[{"id":"05-51-01","title":"Hard landing inspection","fixHours":2}]}"#,
        )
        .unwrap();
        std::fs::write(root.join("catalog/spares.json"), r#"{"commonAta":[32],"rareAta":[72]}"#).unwrap();
        std::fs::write(root.join("catalog/limits.json"), r#"{"checks":[]}"#).unwrap();
        let db = root.join("data/techlog.db");
        let mut tl = SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE airframe (id INTEGER PRIMARY KEY, registration TEXT, variant TEXT, engine TEXT, maintenance TEXT, counters TEXT, consumables TEXT, health TEXT, last_seen_utc TEXT)",
            "CREATE TABLE servicing (id INTEGER PRIMARY KEY, airframe_id INTEGER, utc TEXT, kind TEXT, before REAL, after REAL, unit TEXT)",
            "CREATE TABLE defect (id INTEGER PRIMARY KEY, airframe_id INTEGER, title TEXT, mel_id TEXT, rectified_utc TEXT, rectified_how TEXT)",
            "CREATE TABLE check_record (airframe_id INTEGER, key TEXT, last_done_utc TEXT, last_done_fh REAL, last_done_fc REAL)",
            "INSERT INTO airframe (id, registration, variant, engine) VALUES (1, 'EC-TST', 'A320', 'CFM')",
            "INSERT INTO servicing (airframe_id, utc, kind, before, after) VALUES (1, '2026-10-01T10:00:00Z', 'WASTE', 120, 0)",
            "INSERT INTO check_record VALUES (1, 'DAILY', '2026-10-01T09:00:00Z', 0, 0)",
        ] {
            sqlx::query(sql).execute(&mut tl).await.unwrap();
        }
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE techlog_seen (event_key TEXT PRIMARY KEY, source TEXT NOT NULL, registration TEXT NOT NULL, cost REAL NOT NULL DEFAULT 0, seen_at TEXT NOT NULL)",
            "CREATE TABLE maintenance_log (id INTEGER PRIMARY KEY, registration TEXT, component TEXT, cost REAL, serviced_at TEXT)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        // 1ª pasada: historial registrado, nada cobrado.
        assert_eq!(sync_charges_at(&pool, TlSource::Pmdg, &root, &db).await.unwrap(), 0);
        // Mantenimiento nuevo en TechLog.
        for sql in [
            "INSERT INTO servicing (airframe_id, utc, kind, before, after) VALUES (1, '2026-10-02T10:00:00Z', 'Replaced: Brake 2', 5, 100)",
            "INSERT INTO servicing (airframe_id, utc, kind, before, after) VALUES (1, '2026-10-02T10:05:00Z', 'ENG_OIL:1', 15, 20)",
            "INSERT INTO defect (airframe_id, title, mel_id, rectified_utc, rectified_how) VALUES (1, 'Hard landing inspection', '05-51-01', '2026-10-02T11:00:00Z', 'fixed')",
            "UPDATE check_record SET last_done_utc = '2026-10-02T12:00:00Z' WHERE key = 'DAILY'",
            // (24/10) Catering hecho por GSX: lo cobra el recibo de GSX.
            "INSERT INTO servicing (airframe_id, utc, kind, before, after, unit) VALUES (1, '2026-10-02T10:10:00Z', 'CATERING', 100, 100, '%, by GSX')",
        ] {
            sqlx::query(sql).execute(&mut tl).await.unwrap();
        }
        tl.close().await.unwrap();
        // Un cobro doble de antes (versión vieja) que debe desaparecer.
        sqlx::query("INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES ('ECTST', 'techlog:service:CATERING', 900, '2026-10-02T10:10:00Z')")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(sync_charges_at(&pool, TlSource::Pmdg, &root, &db).await.unwrap(), 4);
        let rows: Vec<(String, f64)> =
            sqlx::query_as("SELECT component, cost FROM maintenance_log ORDER BY component")
                .fetch_all(&pool)
                .await
                .unwrap();
        // (24/10) Precios reales, tarifa por defecto (110 USD/h, sin ubicación):
        // Daily 2,5 h × 1,36 de material; inspección 2 h; freno de carbono
        // del 737 por intercambio 48.000 + 3 h; 5 qt de aceite a 23,31 + 0,25 h.
        assert_eq!(
            rows,
            vec![
                ("techlog:check:DAILY".to_string(), 374.0),
                ("techlog:defect:Hard landing inspection".to_string(), 220.0),
                ("techlog:part_replace:Brake 2".to_string(), 48_330.0),
                ("techlog:service:ENG_OIL:1".to_string(), 144.0),
            ]
        );
        // 3ª pasada: nada nuevo → nada cobrado.
        assert_eq!(sync_charges_at(&pool, TlSource::Pmdg, &root, &db).await.unwrap(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dispatch_rules() {
        let mk = |status: &str, cat: &str| TlDefect {
            id: 1,
            ata: 0,
            title: "x".into(),
            detail: None,
            category: cat.into(),
            status: status.into(),
            mel_id: None,
            position: None,
            raised_utc: None,
            deferred_until: None,
            latest_deferral: None,
            can_defer: true,
            failure: None,
            placard: None,
            restrictions: None,
        };
        assert_eq!(dispatch_of(&[]).status, "GO");
        assert_eq!(dispatch_of(&[mk("Deferred", "C")]).status, "GO_MEL");
        assert_eq!(dispatch_of(&[mk("Open", "C")]).status, "NO_GO");
        assert_eq!(dispatch_of(&[mk("Open", "NoGo")]).status, "NO_GO");
    }

    #[test]
    fn parses_fenix_readings() {
        // Forma real de GET /api/fenix-readings (TechLog 2.2.0), sin y con valores.
        let idle = parse_readings(&serde_json::json!({ "groups": [
            { "name": "Engine oil", "items": [{ "label": "Quantity 1", "value": null, "unit": "qt", "digits": 1 }] }
        ]}));
        assert!(!idle.live);
        assert_eq!(idle.groups[0].items[0].unit, "qt");
        let live = parse_readings(&serde_json::json!({ "groups": [
            { "name": "Hydraulics", "items": [
                { "label": "Green pressure", "value": 3012.4, "unit": "psi", "digits": 0 },
                { "label": "Blue quantity", "value": null, "unit": "L", "digits": 1 }
            ]}
        ]}));
        assert!(live.live);
        assert_eq!(live.groups[0].items[0].value, Some(3012.4));
        assert_eq!(live.groups[0].items[1].value, None);
    }

    #[test]
    fn parses_techlog_cabin_page() {
        // Forma real de GET /api/cabin-systems (TechLog 2.1.2), sin y con Fenix.
        let idle = parse_cabin(&serde_json::json!({
            "live": false, "utc": null, "lights": null, "values": {},
            "canSetLights": false, "canSetPreselect": false
        }));
        assert!(!idle.live && idle.values.is_empty() && idle.lights.is_none());
        let live = parse_cabin(&serde_json::json!({
            "live": true, "utc": "2026-10-23T17:00:00Z", "lights": "Dim1",
            "values": { "cabWaterPct": 75.5, "cabDoorD1L": true, "cabSlideD1L": false, "x": "texto" },
            "canSetLights": true, "canSetPreselect": true
        }));
        assert!(live.live && live.can_set_lights && live.can_set_preselect);
        assert_eq!(live.lights.as_deref(), Some("Dim1"));
        assert_eq!(live.values.get("cabWaterPct"), Some(&75.5));
        assert_eq!(live.values.get("cabDoorD1L"), Some(&1.0));
        assert_eq!(live.values.get("cabSlideD1L"), Some(&0.0));
        assert!(!live.values.contains_key("x"));
    }
}
