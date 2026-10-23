//! (v8.3 · Etapa 6.2a) Vuelos reales de OpenSky con la cuenta del usuario.
//!
//! Uso personal: las claves (API client de OpenSky) van cifradas dentro de la
//! build (`secrets.local.toml` → `build.rs`, nunca en el repo) y las llamadas
//! salen desde el PC del usuario — no hay servidor en medio.
//!
//! Cada 30 min, si hay claves, se piden a `/flights/all` las ventanas de 2 h
//! UTC que falten de los 2 últimos días UTC CERRADOS y se guardan sólo los
//! vuelos cuyo indicativo es de una aerolínea de Finanzas (`real_flight`).
//!
//! Medido el 22/10/2026 con la cuenta real: el día UTC en curso viene
//! incompleto (≈ 1/3 de los vuelos; OpenSky los completa en un lote
//! nocturno) y cada ventana cuesta 30 créditos aunque la doc diga 4 para las
//! últimas 24 h. Por eso sólo se piden días cerrados (6 h después de la
//! medianoche UTC) y el costo real se toma de `X-Rate-Limit-Remaining`.
//! Régimen normal: 12 ventanas/día = 360 créditos por PC (cuota: 4.000/día,
//! compartida con el otro PC → tope diario por PC).

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TOKEN_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";
const API: &str = "https://opensky-network.org/api";
/// Ventana máxima de `/flights/all`.
const WINDOW_S: i64 = 7_200;
const DAY_S: i64 = 86_400;
/// Un día UTC se da por completo 6 h después de su medianoche siguiente
/// (margen para el lote nocturno de OpenSky).
const FINAL_MARGIN_S: i64 = 6 * 3_600;
/// Días UTC cerrados que se recuperan si SimFleet estuvo cerrado (decisión: 2).
const BACKFILL_DAYS: i64 = 2;
/// Costo observado de una ventana (la doc dice 4 para < 24 h; cobran 30).
const WINDOW_COST: i64 = 30;
/// Tope de créditos por día UTC en ESTE PC (la cuota se comparte).
const DAILY_BUDGET: i64 = 1_200;
/// Los vuelos guardados se conservan este tiempo.
const RETENTION_S: i64 = 35 * 86_400;
const LOOP_EVERY: Duration = Duration::from_secs(30 * 60);

// ───────────────────────────── Claves ─────────────────────────────

/// Descifra un valor de `build.rs::seal`: `semilla:cifrado` (sin código) o
/// `v2:sal:check:cifrado` (con código de activación; `None` si el código no
/// es el correcto).
fn unseal(sealed: &str, code: Option<&str>) -> Option<String> {
    if let Some(rest) = sealed.strip_prefix("v2:") {
        let mut it = rest.splitn(3, ':');
        let salt = u64::from_str_radix(it.next()?, 16).ok()?;
        let check = u64::from_str_radix(it.next()?, 16).ok()?;
        let plain = xor_hex(it.next()?, stretch(code?, salt))?;
        return (fnv64(plain.as_bytes()) == check).then_some(plain);
    }
    let (seed, hex) = sealed.split_once(':')?;
    xor_hex(hex, u64::from_str_radix(seed, 16).ok()?)
}

fn xor_hex(hex: &str, mut state: u64) -> Option<String> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .ok()
                .map(|b| b ^ (splitmix64(&mut state) >> 24) as u8)
        })
        .collect();
    String::from_utf8(bytes?).ok().filter(|s| !s.is_empty())
}

/// Código de activación normalizado: sólo letras y dígitos, en mayúsculas
/// (se puede escribir con o sin guiones). Igual que en `build.rs`.
fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

/// Clave a partir del código + sal (estirada). Igual que en `build.rs`.
fn stretch(code: &str, salt: u64) -> u64 {
    let code = normalize_code(code);
    let mut s = salt;
    for _ in 0..20_000 {
        for b in code.bytes() {
            s = (s ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        splitmix64(&mut s);
    }
    s
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn splitmix64(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// (v8.3) Código de activación: lo escribe el usuario en Ajustes y se guarda
// en `settings` (clave local, no se sincroniza con Drive). Sin él, una build
// con claves `v2` no puede usar la cuenta de OpenSky.
const KEY_ACTIVATION: &str = "opensky_activation";
type CredsCache = Mutex<Option<(Option<String>, Option<(String, String)>)>>;
static ACTIVATION: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static CREDS: OnceLock<CredsCache> = OnceLock::new();

fn current_activation() -> Option<String> {
    ACTIVATION.get_or_init(|| Mutex::new(None)).lock().ok().and_then(|g| g.clone())
}

fn store_activation(code: Option<String>) {
    if let Ok(mut g) = ACTIVATION.get_or_init(|| Mutex::new(None)).lock() {
        *g = code;
    }
}

/// Carga el código guardado (al arrancar).
pub async fn load_activation(pool: &SqlitePool) {
    let code: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(KEY_ACTIVATION)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    store_activation(code);
}

/// La build trae claves que piden código de activación.
pub fn needs_activation() -> bool {
    option_env!("SIMFLEET_OPENSKY_CLIENT_ID_X").is_some_and(|s| s.starts_with("v2:"))
}

/// (client_id, client_secret) incrustados en esta build, si los hay y (con
/// claves `v2`) si el código de activación es el correcto. Cacheado por código.
fn embedded_credentials() -> Option<(String, String)> {
    let code = current_activation();
    let cache = CREDS.get_or_init(|| Mutex::new(None));
    if let Ok(g) = cache.lock() {
        if let Some((c, creds)) = g.as_ref() {
            if *c == code {
                return creds.clone();
            }
        }
    }
    let creds = (|| {
        let id = unseal(option_env!("SIMFLEET_OPENSKY_CLIENT_ID_X")?, code.as_deref())?;
        let secret = unseal(option_env!("SIMFLEET_OPENSKY_CLIENT_SECRET_X")?, code.as_deref())?;
        Some((id, secret))
    })();
    if let Ok(mut g) = cache.lock() {
        *g = Some((code, creds.clone()));
    }
    creds
}

pub fn is_configured() -> bool {
    embedded_credentials().is_some()
}

/// (v8.3) Prueba y guarda el código de activación. Error si no abre las claves.
pub async fn set_activation(pool: &SqlitePool, code: &str) -> Result<()> {
    let blob = option_env!("SIMFLEET_OPENSKY_CLIENT_ID_X").ok_or_else(|| anyhow!(crate::tr!("opensky.err.notConfigured")))?;
    let norm = normalize_code(code);
    if norm.is_empty() || unseal(blob, Some(&norm)).is_none() {
        return Err(anyhow!(crate::tr!("opensky.err.badCode")));
    }
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(KEY_ACTIVATION)
        .bind(&norm)
        .execute(pool)
        .await?;
    store_activation(Some(norm));
    forget_token();
    tracing::info!(target: "opensky", "código de activación aceptado");
    Ok(())
}

// ───────────────────────────── API ─────────────────────────────

static TOKEN: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();

fn token_cell() -> &'static Mutex<Option<(String, Instant)>> {
    TOKEN.get_or_init(|| Mutex::new(None))
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

async fn token(http: &reqwest::Client) -> Result<String> {
    if let Ok(g) = token_cell().lock() {
        if let Some((t, exp)) = g.as_ref() {
            if *exp > Instant::now() + Duration::from_secs(60) {
                return Ok(t.clone());
            }
        }
    }
    let (id, secret) = embedded_credentials().ok_or_else(|| anyhow!("sin claves de OpenSky en esta build"))?;
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", id.as_str()),
            ("client_secret", secret.as_str()),
        ])
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("token de OpenSky")?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Nunca se registra el cuerpo de la petición (lleva el secreto).
        return Err(anyhow!("OpenSky rechazó las claves (HTTP {status})"));
    }
    let t: TokenResp = serde_json::from_str(&body).context("respuesta de token ilegible")?;
    let ttl = Duration::from_secs(if t.expires_in > 0 { t.expires_in } else { 1_800 });
    if let Ok(mut g) = token_cell().lock() {
        *g = Some((t.access_token.clone(), Instant::now() + ttl));
    }
    Ok(t.access_token)
}

fn forget_token() {
    if let Ok(mut g) = token_cell().lock() {
        *g = None;
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OsFlight {
    icao24: String,
    first_seen: i64,
    last_seen: i64,
    #[serde(default)]
    est_departure_airport: Option<String>,
    #[serde(default)]
    est_arrival_airport: Option<String>,
    #[serde(default)]
    callsign: Option<String>,
}

struct WindowResp {
    flights: Vec<OsFlight>,
    remaining: Option<i64>,
}

#[derive(Debug)]
enum FetchError {
    /// Cuota agotada: esperar `retry_after` segundos.
    RateLimited(Option<u64>),
    Other(anyhow::Error),
}

async fn fetch_window(http: &reqwest::Client, begin: i64, end: i64) -> Result<WindowResp, FetchError> {
    for attempt in 0..2 {
        let tok = token(http).await.map_err(FetchError::Other)?;
        let resp = http
            .get(format!("{API}/flights/all"))
            .query(&[("begin", begin), ("end", end)])
            .bearer_auth(&tok)
            .timeout(Duration::from_secs(90))
            .send()
            .await
            .map_err(|e| FetchError::Other(e.into()))?;
        let status = resp.status();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<i64>().ok())
        };
        let remaining = header("X-Rate-Limit-Remaining");
        let retry_after = header("X-Rate-Limit-Retry-After-Seconds").map(|s| s.max(0) as u64);
        match status.as_u16() {
            401 if attempt == 0 => {
                forget_token();
                continue;
            }
            404 => return Ok(WindowResp { flights: Vec::new(), remaining }),
            429 => return Err(FetchError::RateLimited(retry_after)),
            s if !(200..300).contains(&s) => {
                return Err(FetchError::Other(anyhow!("OpenSky respondió HTTP {status}")));
            }
            _ => {}
        }
        let body = resp.text().await.map_err(|e| FetchError::Other(e.into()))?;
        let flights: Vec<OsFlight> =
            serde_json::from_str(&body).map_err(|e| FetchError::Other(anyhow!("respuesta ilegible: {e}")))?;
        return Ok(WindowResp { flights, remaining });
    }
    Err(FetchError::Other(anyhow!("OpenSky no aceptó el token")))
}

// ───────────────────────── Estado en vivo (6.2d) ─────────────────────────

/// Estado en vivo de un avión según OpenSky (`/states/all?icao24=`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveState {
    pub on_ground: bool,
    pub callsign: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Último contacto (Unix, s).
    pub last_contact: i64,
}

/// Un estado más viejo que esto no cuenta como "en el aire ahora".
const LIVE_STALE_S: i64 = 15 * 60;
const LIVE_CACHE: Duration = Duration::from_secs(300);
type LiveCache = Mutex<std::collections::HashMap<String, (Instant, Option<LiveState>)>>;
static LIVE: OnceLock<LiveCache> = OnceLock::new();

/// Lee el estado de un vector de `/states/all` (índices de la doc).
fn parse_state(body: &str) -> Option<LiveState> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = v.get("states")?.as_array()?.first()?.as_array()?;
    Some(LiveState {
        callsign: s.get(1).and_then(|x| x.as_str()).map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
        last_contact: s.get(4).and_then(|x| x.as_i64()).unwrap_or(0),
        lon: s.get(5).and_then(|x| x.as_f64()),
        lat: s.get(6).and_then(|x| x.as_f64()),
        on_ground: s.get(8).and_then(|x| x.as_bool()).unwrap_or(true),
    })
}

/// Estado en vivo del avión `icao24` (cacheado 5 min). `None` si OpenSky no
/// lo ve ahora (o el último contacto es viejo).
pub async fn live_state(http: &reqwest::Client, icao24: &str) -> Result<Option<LiveState>> {
    let hex = icao24.trim().to_ascii_lowercase();
    let cell = LIVE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(g) = cell.lock() {
        if let Some((t, st)) = g.get(&hex) {
            if t.elapsed() < LIVE_CACHE {
                return Ok(st.clone());
            }
        }
    }
    let tok = token(http).await?;
    let resp = http
        .get(format!("{API}/states/all"))
        .query(&[("icao24", hex.as_str())])
        .bearer_auth(&tok)
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    if resp.status().as_u16() == 401 {
        forget_token();
    }
    let body = resp.error_for_status()?.text().await?;
    let st = parse_state(&body).filter(|s| Utc::now().timestamp() - s.last_contact <= LIVE_STALE_S);
    if let Ok(mut g) = cell.lock() {
        g.insert(hex, (Instant::now(), st.clone()));
    }
    Ok(st)
}

// ───────────────────────────── Lógica pura ─────────────────────────────

/// Momento desde el que la ventana que empieza en `begin` está completa:
/// 6 h después de la medianoche UTC que cierra su día.
fn final_at(begin: i64) -> i64 {
    (begin.div_euclid(DAY_S) + 1) * DAY_S + FINAL_MARGIN_S
}

/// Ventanas de 2 h de los últimos `BACKFILL_DAYS` días UTC cerrados que aún
/// no se bajaron (completas), de la más nueva a la más vieja.
fn pending_windows(now: i64, done: &HashSet<i64>) -> Vec<i64> {
    // Último día cerrado: el anterior al día en que cae `now - margen`.
    let last_day = (now - FINAL_MARGIN_S).div_euclid(DAY_S) - 1;
    let newest_begin = (last_day + 1) * DAY_S - WINDOW_S;
    let oldest_begin = (last_day - BACKFILL_DAYS + 1) * DAY_S;
    let mut out = Vec::new();
    let mut begin = newest_begin;
    while begin >= oldest_begin {
        if !done.contains(&begin) {
            out.push(begin);
        }
        begin -= WINDOW_S;
    }
    out
}

/// Ventanas ya bajadas COMPLETAS (pedidas cuando su día ya estaba cerrado).
/// Una pedida antes de tiempo se vuelve a pedir.
async fn done_windows(pool: &SqlitePool) -> Result<HashSet<i64>> {
    let rows = sqlx::query("SELECT begin_ts, fetched_at FROM opensky_window WHERE status = 'ok'")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let begin: i64 = r.get("begin_ts");
            let fetched = DateTime::parse_from_rfc3339(&r.get::<String, _>("fetched_at")).ok()?;
            (fetched.timestamp() >= final_at(begin)).then_some(begin)
        })
        .collect())
}

/// (6.2c) Días UTC (inicio, Unix s) con sus 12 ventanas bajadas completas,
/// del más viejo al más nuevo. Sólo esos días cuentan en Finanzas.
pub async fn complete_days(pool: &SqlitePool) -> Result<Vec<i64>> {
    let mut per_day: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for begin in done_windows(pool).await? {
        *per_day.entry(begin.div_euclid(DAY_S) * DAY_S).or_default() += 1;
    }
    let mut days: Vec<i64> = per_day
        .into_iter()
        .filter(|(_, n)| *n >= DAY_S / WINDOW_S)
        .map(|(d, _)| d)
        .collect();
    days.sort_unstable();
    Ok(days)
}

/// Créditos reales de una petición: lo que bajó `X-Rate-Limit-Remaining`
/// respecto a la petición anterior de esta misma sincronía.
fn actual_cost(prev: Option<i64>, now: Option<i64>) -> i64 {
    match (prev, now) {
        (Some(p), Some(n)) if p >= n && p - n <= 500 => p - n,
        _ => WINDOW_COST,
    }
}

/// Aerolínea (ICAO) de un indicativo si está en `airlines`: 3 letras + algo
/// más ("WJA650" → "WJA"). Matrículas y privados quedan fuera.
fn airline_of(callsign: &str, airlines: &HashSet<String>) -> Option<String> {
    let cs = callsign.trim().to_ascii_uppercase();
    let prefix: String = cs.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = &cs[prefix.len()..];
    (prefix.len() == 3 && !rest.is_empty() && rest.starts_with(|c: char| c.is_ascii_digit()) && airlines.contains(&prefix))
        .then_some(prefix)
}

fn clean_icao(s: Option<String>) -> Option<String> {
    s.map(|x| x.trim().to_ascii_uppercase()).filter(|x| x.len() == 4)
}

// ───────────────────────────── Sincronía ─────────────────────────────

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub windows: i64,
    pub kept: i64,
    pub credits: i64,
    /// Por qué paró antes de terminar (cuota, tope diario, error…).
    pub stopped: Option<String>,
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// ICAO de las aerolíneas de Finanzas (las que operan solas).
async fn finance_airlines(pool: &SqlitePool) -> Result<HashSet<String>> {
    let load = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT airline_icao FROM airline_economy WHERE airline_icao IS NOT NULL AND airline_icao <> ''",
        )
        .fetch_all(pool)
        .await
    };
    let mut icaos = load().await?;
    if icaos.is_empty() {
        // Finanzas aún no se abrió en esta instalación: se calcula una vez.
        crate::airline_economy::recompute(pool).await?;
        icaos = load().await?;
    }
    Ok(icaos.into_iter().map(|s| s.trim().to_ascii_uppercase()).collect())
}

async fn credits_today(pool: &SqlitePool, now: DateTime<Utc>) -> Result<i64> {
    let day = now.format("%Y-%m-%d").to_string();
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(credits), 0) FROM opensky_window WHERE substr(fetched_at, 1, 10) = ?",
    )
    .bind(day)
    .fetch_one(pool)
    .await?)
}

#[allow(clippy::too_many_arguments)]
async fn record_window(
    pool: &SqlitePool,
    begin: i64,
    status: &str,
    credits: i64,
    total: i64,
    kept: i64,
    remaining: Option<i64>,
    error: Option<String>,
) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO opensky_window
            (begin_ts, end_ts, fetched_at, status, credits, total_flights, kept_flights, remaining, error)
         VALUES (?,?,?,?,?,?,?,?,?)",
    )
    .bind(begin)
    .bind(begin + WINDOW_S)
    .bind(Utc::now().to_rfc3339())
    .bind(status)
    .bind(credits)
    .bind(total)
    .bind(kept)
    .bind(remaining)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Baja las ventanas que falten. Una sola sincronía a la vez.
pub async fn sync(pool: &SqlitePool, http: &reqwest::Client) -> Result<SyncReport> {
    if !is_configured() {
        return Err(anyhow!(crate::tr!("opensky.err.notConfigured")));
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(anyhow!(crate::tr!("opensky.err.busy")));
    }
    let res = sync_inner(pool, http).await;
    RUNNING.store(false, Ordering::SeqCst);
    res
}

async fn sync_inner(pool: &SqlitePool, http: &reqwest::Client) -> Result<SyncReport> {
    let airlines = finance_airlines(pool).await?;
    let mut report = SyncReport::default();
    if airlines.is_empty() {
        report.stopped = Some("no_airlines".into());
        return Ok(report);
    }
    let mine = crate::my_fleet::my_icao24(pool).await.unwrap_or_default();
    let now = Utc::now();
    let done = done_windows(pool).await?;
    let mut spent = credits_today(pool, now).await?;
    // Saldo tras la petición anterior (sólo de esta sincronía: el otro PC
    // también gasta de la misma cuota entre medias).
    let mut prev_remaining: Option<i64> = None;
    for begin in pending_windows(now.timestamp(), &done) {
        if spent + WINDOW_COST > DAILY_BUDGET {
            report.stopped = Some("daily_budget".into());
            break;
        }
        match fetch_window(http, begin, begin + WINDOW_S).await {
            Ok(w) => {
                let cost = actual_cost(prev_remaining, w.remaining);
                prev_remaining = w.remaining.or(prev_remaining.map(|p| p - cost));
                let total = w.flights.len() as i64;
                let mut kept = 0i64;
                let mut tx = pool.begin().await?;
                for f in w.flights {
                    let hex = f.icao24.trim().to_ascii_lowercase();
                    let cs = f.callsign.as_deref().map(|c| c.trim().to_ascii_uppercase()).unwrap_or_default();
                    // (6.2d) También los vuelos de los aviones del usuario,
                    // aunque su aerolínea no esté en Finanzas.
                    let airline = match airline_of(&cs, &airlines) {
                        Some(a) => a,
                        None if mine.contains_key(&hex) => cs.chars().take(3).collect(),
                        None => continue,
                    };
                    let res = sqlx::query(
                        "INSERT OR IGNORE INTO real_flight
                            (icao24, first_seen, last_seen, callsign, airline_icao, dep_icao, arr_icao, window_begin)
                         VALUES (?,?,?,?,?,?,?,?)",
                    )
                    .bind(&hex)
                    .bind(f.first_seen)
                    .bind(f.last_seen)
                    .bind(&cs)
                    .bind(&airline)
                    .bind(clean_icao(f.est_departure_airport))
                    .bind(clean_icao(f.est_arrival_airport))
                    .bind(begin)
                    .execute(&mut *tx)
                    .await?;
                    kept += res.rows_affected() as i64;
                }
                tx.commit().await?;
                record_window(pool, begin, "ok", cost, total, kept, w.remaining, None).await?;
                spent += cost;
                report.windows += 1;
                report.kept += kept;
                report.credits += cost;
                tracing::info!(target: "opensky", "ventana {begin}: {total} vuelos, {kept} de tus aerolíneas · {cost} créditos · quedan {:?}", w.remaining);
            }
            Err(FetchError::RateLimited(retry)) => {
                tracing::warn!(target: "opensky", "cuota de OpenSky agotada (reintentar en {retry:?} s)");
                report.stopped = Some("rate_limited".into());
                break;
            }
            Err(FetchError::Other(e)) => {
                tracing::warn!(target: "opensky", "ventana {begin}: {e:#}");
                record_window(pool, begin, "error", 0, 0, 0, None, Some(format!("{e:#}"))).await?;
                report.stopped = Some(format!("{e:#}"));
                break;
            }
        }
    }
    let cutoff = now.timestamp() - RETENTION_S;
    sqlx::query("DELETE FROM real_flight WHERE first_seen < ?").bind(cutoff).execute(pool).await?;
    sqlx::query("DELETE FROM opensky_window WHERE begin_ts < ?").bind(cutoff).execute(pool).await?;
    Ok(report)
}

/// Una vuelta completa: vuelos reales, base de aviones (una vez por PC) y
/// aviones nuevos vía adsbdb.
pub async fn run_cycle(pool: &SqlitePool, http: &reqwest::Client) {
    match sync(pool, http).await {
        Ok(r) if r.windows > 0 || r.stopped.is_some() => {
            tracing::info!(target: "opensky", "sincronía: {} ventanas, {} vuelos, {} créditos, paró: {:?}", r.windows, r.kept, r.credits, r.stopped)
        }
        Ok(_) => {}
        Err(e) => tracing::debug!(target: "opensky", "sincronía: {e:#}"),
    }
    if let Err(e) = crate::aircraft_db::ensure_db(pool, http).await {
        tracing::warn!(target: "aircraft_db", "base de aviones: {e:#}");
    }
    if let Err(e) = crate::aircraft_db::enrich_unknown(pool, http).await {
        tracing::debug!(target: "aircraft_db", "adsbdb: {e:#}");
    }
}

/// Sincronía en segundo plano cada 30 min. Si la build trae claves con
/// código de activación, espera a que el usuario lo escriba en Ajustes.
pub fn spawn_loop(pool: SqlitePool, http: reqwest::Client) {
    if option_env!("SIMFLEET_OPENSKY_CLIENT_ID_X").is_none() {
        tracing::info!(target: "opensky", "sin claves de OpenSky en esta build: vuelos reales desactivados");
        return;
    }
    // (6.2d) Aterrizajes en vivo de aviones reservados mientras volaban.
    {
        let (pool, http) = (pool.clone(), http.clone());
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(600)).await;
                if !is_configured() {
                    continue;
                }
                if let Err(e) = crate::ferry::poll_reservation_landings(&pool, &http).await {
                    tracing::debug!(target: "ferry", "aterrizajes de reservas: {e:#}");
                }
            }
        });
    }
    tauri::async_runtime::spawn(async move {
        load_activation(&pool).await;
        if !is_configured() && needs_activation() {
            tracing::info!(target: "opensky", "vuelos reales en espera del código de activación (Ajustes)");
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if is_configured() {
                run_cycle(&pool, &http).await;
            }
            tokio::time::sleep(LOOP_EVERY).await;
        }
    });
}

// ───────────────────────────── Estado ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlineCount {
    pub icao: String,
    pub flights: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSkyStatus {
    /// La build trae claves de OpenSky.
    pub configured: bool,
    /// (v8.3) La build trae claves pero falta (o no sirve) el código de
    /// activación: Ajustes lo pide.
    pub needs_code: bool,
    pub running: bool,
    pub last_fetch_at: Option<String>,
    pub last_error: Option<String>,
    /// Ventanas de los 2 últimos días UTC cerrados: bajadas / pendientes.
    pub windows_done: i64,
    pub windows_pending: i64,
    /// Vuelos reales guardados del último día UTC cerrado / en total.
    pub flights_last_day: i64,
    /// Ese día (AAAA-MM-DD, UTC).
    pub last_day: Option<String>,
    pub flights_total: i64,
    pub credits_today: i64,
    pub daily_budget: i64,
    /// Créditos que le quedan a la cuenta hoy (según OpenSky).
    pub remaining: Option<i64>,
    /// Aerolíneas con más vuelos ese día.
    pub airlines: Vec<AirlineCount>,
    /// (6.2b) Aviones en la base local, versión de la base de OpenSky y
    /// consultas a adsbdb hoy.
    pub aircraft_db_rows: i64,
    pub aircraft_db_version: Option<String>,
    pub adsbdb_today: i64,
}

pub async fn status(pool: &SqlitePool) -> Result<OpenSkyStatus> {
    let now = Utc::now();
    let ts = now.timestamp();
    let pending = pending_windows(ts, &done_windows(pool).await?).len() as i64;
    let per_day = DAY_S / WINDOW_S;
    let last = sqlx::query(
        "SELECT fetched_at, status, remaining, error FROM opensky_window ORDER BY fetched_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let remaining = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT remaining FROM opensky_window WHERE remaining IS NOT NULL ORDER BY fetched_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    // Último día UTC cerrado (el día en curso aún viene incompleto).
    let last_day_start = ((ts - FINAL_MARGIN_S).div_euclid(DAY_S) - 1) * DAY_S;
    let flights_last_day: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM real_flight WHERE window_begin >= ? AND window_begin < ?",
    )
    .bind(last_day_start)
    .bind(last_day_start + DAY_S)
    .fetch_one(pool)
    .await?;
    let flights_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM real_flight").fetch_one(pool).await?;
    let airlines = sqlx::query(
        "SELECT airline_icao, COUNT(*) AS n FROM real_flight WHERE window_begin >= ? AND window_begin < ?
          GROUP BY airline_icao ORDER BY n DESC LIMIT 8",
    )
    .bind(last_day_start)
    .bind(last_day_start + DAY_S)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| AirlineCount { icao: r.get("airline_icao"), flights: r.get("n") })
    .collect();
    let (last_fetch_at, last_error) = match last {
        Some(r) => (
            Some(r.get::<String, _>("fetched_at")),
            (r.get::<String, _>("status") == "error").then(|| r.get::<Option<String>, _>("error")).flatten(),
        ),
        None => (None, None),
    };
    let (aircraft_db_rows, aircraft_db_version, adsbdb_today) = crate::aircraft_db::db_status(pool).await;
    Ok(OpenSkyStatus {
        aircraft_db_rows,
        aircraft_db_version,
        adsbdb_today,
        configured: is_configured(),
        needs_code: needs_activation() && !is_configured(),
        running: RUNNING.load(Ordering::SeqCst),
        last_fetch_at,
        last_error,
        windows_done: BACKFILL_DAYS * per_day - pending,
        windows_pending: pending,
        flights_last_day,
        last_day: DateTime::from_timestamp(last_day_start, 0).map(|d| d.format("%Y-%m-%d").to_string()),
        flights_total,
        credits_today: credits_today(pool, now).await?,
        daily_budget: DAILY_BUDGET,
        remaining,
        airlines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copia de `build.rs::seal` con semilla fija, para probar el descifrado.
    fn seal_with(seed: u64, value: &str) -> String {
        let mut state = seed;
        let cipher: String = value
            .bytes()
            .map(|b| format!("{:02x}", b ^ (splitmix64(&mut state) >> 24) as u8))
            .collect();
        format!("{seed:016x}:{cipher}")
    }

    #[test]
    fn sealed_secret_roundtrips_and_is_not_plain() {
        let secret = "s3cr3t-ÄPI_client";
        let sealed = seal_with(0xDEAD_BEEF_1234_5678, secret);
        assert!(!sealed.contains("s3cr3t"));
        assert_eq!(unseal(&sealed, None).as_deref(), Some(secret));
        assert_eq!(unseal("zz:00", None), None);
        assert_eq!(unseal("0011223344556677:abc", None), None);
    }

    /// Copia de `build.rs::seal` con código de activación y sal fija.
    fn seal_v2(salt: u64, code: &str, value: &str) -> String {
        let mut state = stretch(code, salt);
        let cipher: String = value
            .bytes()
            .map(|b| format!("{:02x}", b ^ (splitmix64(&mut state) >> 24) as u8))
            .collect();
        format!("v2:{salt:016x}:{:016x}:{cipher}", fnv64(value.as_bytes()))
    }

    #[test]
    fn activation_code_is_required_for_v2_secrets() {
        let sealed = seal_v2(0x1234_5678_9ABC_DEF0, "SFK7-QM2P-X9TD", "noxful-api-client");
        assert!(!sealed.contains("noxful"));
        assert_eq!(unseal(&sealed, Some("SFK7-QM2P-X9TD")).as_deref(), Some("noxful-api-client"));
        // Con o sin guiones, en minúsculas: mismo código.
        assert_eq!(unseal(&sealed, Some("sfk7qm2px9td")).as_deref(), Some("noxful-api-client"));
        assert_eq!(unseal(&sealed, Some("SFK7-QM2P-X9TE")), None, "código incorrecto");
        assert_eq!(unseal(&sealed, None), None, "sin código");
    }

    #[test]
    fn only_closed_utc_days_are_requested() {
        let day = |d: i64| 1_792_540_800 + d * DAY_S; // 2026-10-21 00:00 UTC + d días
        // 22/10 23:13 UTC → días cerrados: 20/10 y 21/10 (24 ventanas).
        let now = day(1) + 23 * 3_600 + 13 * 60;
        let w = pending_windows(now, &HashSet::new());
        assert_eq!(w.len(), 24);
        assert_eq!(w[0], day(1) - WINDOW_S, "la más nueva: 21/10 22:00");
        assert_eq!(*w.last().unwrap(), day(-1), "la más vieja: 20/10 00:00");
        assert!(w.windows(2).all(|p| p[0] - p[1] == WINDOW_S));
        assert!(w.iter().all(|b| final_at(*b) <= now));
        // 23/10 03:00 UTC: el 22/10 aún no está completo (margen de 6 h).
        assert_eq!(pending_windows(day(2) + 3 * 3_600, &HashSet::new())[0], day(1) - WINDOW_S);
        // 23/10 07:00 UTC: ya entra el 22/10.
        assert_eq!(pending_windows(day(2) + 7 * 3_600, &HashSet::new())[0], day(2) - WINDOW_S);
        // Las ya bajadas no se repiten.
        let done: HashSet<i64> = w.iter().take(5).copied().collect();
        assert_eq!(pending_windows(now, &done).len(), 19);
        // Recuperar 2 días entra en el tope diario.
        assert!(w.len() as i64 * WINDOW_COST <= DAILY_BUDGET);
    }

    #[test]
    fn parses_live_state_vector() {
        let body = r#"{"time":1792700000,"states":[["a1b2c3","JBU123  ","United States",1792699990,1792699995,-73.5,40.7,10668.0,false,230.1,45.0,0.0,null,10900.0,"1234",false,0]]}"#;
        let s = parse_state(body).unwrap();
        assert!(!s.on_ground);
        assert_eq!(s.callsign.as_deref(), Some("JBU123"));
        assert_eq!(s.last_contact, 1_792_699_995);
        assert_eq!((s.lat, s.lon), (Some(40.7), Some(-73.5)));
        assert!(parse_state(r#"{"time":1,"states":null}"#).is_none());
    }

    #[test]
    fn real_cost_comes_from_the_remaining_header() {
        assert_eq!(actual_cost(Some(3_970), Some(3_940)), 30);
        assert_eq!(actual_cost(None, Some(3_940)), WINDOW_COST);
        assert_eq!(actual_cost(Some(3_000), Some(3_500)), WINDOW_COST, "se renovó la cuota");
        assert_eq!(actual_cost(Some(3_970), None), WINDOW_COST);
    }

    #[test]
    fn only_airline_callsigns_of_finances() {
        let set: HashSet<String> = ["WJA", "JBU"].iter().map(|s| s.to_string()).collect();
        assert_eq!(airline_of("WJA650  ", &set).as_deref(), Some("WJA"));
        assert_eq!(airline_of("jbu1234", &set).as_deref(), Some("JBU"));
        assert_eq!(airline_of("DAL115", &set), None, "no está en Finanzas");
        assert_eq!(airline_of("N657JB", &set), None, "matrícula");
        assert_eq!(airline_of("WJA", &set), None, "sin número");
        assert_eq!(airline_of("WJAX12", &set), None);
    }
}
