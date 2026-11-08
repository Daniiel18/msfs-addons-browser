//! (01/11) Etapa 5a — pantallas Cockpit, Cabina y Vuelos de Fenix/PMDG
//! TechLog en Hangar → avión → Mantenimiento (las del riel de TechLog).
//!
//! · Cockpit y Cabina salen de la API en vivo, que sólo sirve el avión ACTIVO
//!   de TechLog (el del simulador). La última cabina vista de cada matrícula
//!   queda en los ajustes de SimFleet: si ese avión no es el activo se enseña
//!   esa, con su fecha.
//! · Vuelos sale de la COPIA de solo lectura de `techlog.db` (como el resto de
//!   la sincronía): cualquier avión, con TechLog abierto o cerrado. Borrar un
//!   vuelo y su JSON exacto sí piden TechLog abierto.
//! · Asientos bloqueados (aviso del pre-vuelo): defectos abiertos o diferidos
//!   de asiento con `seat_blocked`, también de la copia.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, Row, SqliteConnection, SqlitePool};

use crate::techlog::{self, normalize_reg, opt_str, TlActionResult, TlSource};
use crate::tl_i18n;

// ---------------------------------------------------------------------------
// Lectura tolerante (TechLog cambia de versión: nada de esto debe romper)
// ---------------------------------------------------------------------------

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k)
        .and_then(|x| x.as_f64().or_else(|| x.as_bool().map(|b| if b { 1.0 } else { 0.0 })))
}

fn int(v: &Value, k: &str) -> Option<i64> {
    v.get(k)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f.round() as i64)))
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

pub(crate) fn col_f64(r: &SqliteRow, c: &str) -> Option<f64> {
    // SQLite puede guardarlo como real, entero o texto.
    r.try_get::<Option<f64>, _>(c)
        .ok()
        .flatten()
        .or_else(|| r.try_get::<Option<i64>, _>(c).ok().flatten().map(|v| v as f64))
        .or_else(|| opt_str(r, c).and_then(|s| s.parse().ok()))
}

pub(crate) fn col_i64(r: &SqliteRow, c: &str) -> Option<i64> {
    r.try_get::<Option<i64>, _>(c)
        .ok()
        .flatten()
        .or_else(|| col_f64(r, c).map(|f| f.round() as i64))
}

fn col_json(r: &SqliteRow, c: &str) -> Option<Value> {
    opt_str(r, c).and_then(|s| serde_json::from_str(&s).ok())
}

// ---------------------------------------------------------------------------
// Dónde está el avión para TechLog
// ---------------------------------------------------------------------------

struct Where {
    source: TlSource,
    running: bool,
    /// Matrícula del avión activo de ese TechLog (como la escribe TechLog).
    active: Option<String>,
}

impl Where {
    fn is_active(&self, reg: &str) -> bool {
        self.active.as_deref().map(normalize_reg).as_deref() == Some(reg)
    }
    /// "live" | "not_active" | "closed".
    fn state(&self, reg: &str) -> &'static str {
        if !self.running {
            "closed"
        } else if self.is_active(reg) {
            "live"
        } else {
            "not_active"
        }
    }
    fn active_display(&self) -> Option<String> {
        self.active.as_deref().map(techlog::display_reg)
    }
}

/// TechLog que lleva esta matrícula (normalizada), si está abierto y cuál es
/// su avión activo. `None` si ningún TechLog la conoce.
async fn locate(reg: &str) -> Option<Where> {
    let source = *techlog::managed_registrations().await.get(reg)?;
    let running = techlog::is_running(source).await;
    let mut active = None;
    if running {
        if let Ok(st) = techlog::api_get(source, "/api/state").await {
            active = st.get("airframe").and_then(|a| text(a, "registration"));
        }
    }
    Some(Where { source, running, active })
}

/// Copia de la base de su TechLog con el `id` y la variante del avión.
async fn open_airframe(reg: &str) -> Result<Option<(TlSource, SqliteConnection, i64, String)>> {
    let Some(source) = techlog::managed_registrations().await.get(reg).copied() else {
        return Ok(None);
    };
    let Some(dir) = techlog::install_dir(source) else { return Ok(None) };
    let Some(db) = techlog::db_path(source, &dir) else { return Ok(None) };
    let mut conn = techlog::open_snapshot(source, &db).await?;
    let rows = sqlx::query("SELECT id, registration, variant FROM airframe")
        .fetch_all(&mut conn)
        .await?;
    let found = rows
        .iter()
        .find(|r| normalize_reg(&opt_str(r, "registration").unwrap_or_default()) == reg)
        .map(|r| (r.try_get::<i64, _>("id").unwrap_or(0), opt_str(r, "variant").unwrap_or_default()));
    match found {
        Some((id, variant)) => Ok(Some((source, conn, id, variant))),
        None => {
            conn.close().await.ok();
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Cockpit — perfiles del estado del cockpit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitPosition {
    pub value: f64,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitControl {
    pub id: String,
    pub name: String,
    /// "step" (posiciones con nombre) | "level" (de 0 a `max`).
    pub kind: String,
    pub max: Option<f64>,
    pub positions: Vec<TlCockpitPosition>,
    pub saved: Option<f64>,
    pub live: Option<f64>,
    /// Entra en el perfil (casilla «Keep» de TechLog).
    pub keep: bool,
    /// TechLog nunca lo ha leído: no se compara.
    pub never: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitPanel {
    pub key: String,
    pub name: String,
    pub controls: Vec<TlCockpitControl>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitZone {
    pub key: String,
    pub name: String,
    pub panels: Vec<TlCockpitPanel>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitProfile {
    pub id: i64,
    pub name: String,
    /// Se restaura solo al cargar el avión en frío y oscuro.
    pub auto: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpit {
    pub registration: Option<String>,
    /// El simulador tiene el avión: se puede leer y mover.
    pub connected: bool,
    /// Tarea en curso de TechLog ("Reading"…); `None` = libre.
    pub busy: Option<String>,
    /// Última frase de TechLog sobre el cockpit.
    pub last: Option<String>,
    /// «Restaurar al cargar en frío y oscuro» (general).
    pub restore: bool,
    pub saved_utc: Option<String>,
    pub saved_airport: Option<String>,
    pub live_utc: Option<String>,
    pub zones: Vec<TlCockpitZone>,
    pub profiles: Vec<TlCockpitProfile>,
    pub profile_id: Option<i64>,
}

fn parse_cockpit(v: &Value) -> TlCockpit {
    let control = |c: &Value| TlCockpitControl {
        id: text(c, "id").unwrap_or_default(),
        name: text(c, "name").unwrap_or_default(),
        kind: text(c, "kind").unwrap_or_else(|| "step".into()),
        max: num(c, "max"),
        positions: list(c, "positions")
            .iter()
            .map(|p| TlCockpitPosition {
                value: num(p, "value").unwrap_or(0.0),
                label: text(p, "label").unwrap_or_default(),
            })
            .collect(),
        saved: num(c, "saved"),
        live: num(c, "live"),
        keep: c.get("keep").and_then(|x| x.as_bool()).unwrap_or(true),
        never: c.get("never").is_some_and(|x| !x.is_null() && x != &Value::Bool(false)),
    };
    TlCockpit {
        registration: text(v, "registration"),
        connected: flag(v, "connected"),
        busy: match v.get("busy") {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Bool(true)) => Some(String::new()),
            _ => None,
        },
        last: text(v, "last").map(|s| tl_i18n::sentence(&s)),
        restore: flag(v, "restore"),
        saved_utc: text(v, "savedUtc"),
        saved_airport: text(v, "savedAirport"),
        live_utc: text(v, "liveUtc"),
        zones: list(v, "zones")
            .iter()
            .map(|z| TlCockpitZone {
                key: text(z, "key").unwrap_or_default(),
                name: text(z, "name").unwrap_or_default(),
                panels: list(z, "panels")
                    .iter()
                    .map(|p| TlCockpitPanel {
                        key: text(p, "key").unwrap_or_default(),
                        name: tl_i18n::name(&text(p, "name").unwrap_or_default()),
                        controls: list(p, "controls").iter().map(control).collect(),
                    })
                    .collect(),
            })
            .collect(),
        profiles: list(v, "profiles")
            .iter()
            .map(|p| TlCockpitProfile {
                id: int(p, "id").unwrap_or(0),
                name: text(p, "name").unwrap_or_default(),
                auto: flag(p, "auto"),
            })
            .collect(),
        profile_id: int(v, "profileId"),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitPage {
    /// "live" | "not_active" | "closed" | "unsupported" | "unmanaged"
    pub state: &'static str,
    pub source: Option<TlSource>,
    pub active_registration: Option<String>,
    pub cockpit: Option<TlCockpit>,
}

/// Cockpit de una matrícula: en vivo sólo si es el avión activo de su TechLog.
pub async fn cockpit(registration: &str) -> TlCockpitPage {
    let reg = normalize_reg(registration);
    let Some(w) = locate(&reg).await else {
        return TlCockpitPage { state: "unmanaged", source: None, active_registration: None, cockpit: None };
    };
    let mut page = TlCockpitPage {
        state: w.state(&reg),
        source: Some(w.source),
        active_registration: w.active_display(),
        cockpit: None,
    };
    if page.state == "live" {
        match techlog::api_get(w.source, "/api/cockpit").await {
            Ok(v) => page.cockpit = Some(parse_cockpit(&v)),
            Err(e) => {
                tracing::debug!(target: "techlog", "{} sin página de cockpit: {e}", w.source.id());
                page.state = "unsupported";
            }
        }
    }
    page
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCockpitRequest {
    pub registration: String,
    /// "read" | "save" | "restore" | "auto" | "set" | "keep" | "select" |
    /// "profileAuto" | "rename" | "delete"
    pub action: String,
    pub profile: Option<i64>,
    pub name: Option<String>,
    pub on: Option<bool>,
    /// Mando (`set`/`keep`) o panel (`restore` de un solo panel).
    pub id: Option<String>,
    pub value: Option<f64>,
    pub keep: Option<bool>,
}

/// Orden al cockpit del avión ACTIVO (las mismas que la página de TechLog).
pub async fn cockpit_action(req: TlCockpitRequest) -> Result<TlActionResult> {
    let reg = normalize_reg(&req.registration);
    let source = techlog::active_source_for(&reg)
        .await
        .ok_or_else(|| anyhow!("not_active"))?;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(40).collect::<String>());
    let id = || req.id.clone().filter(|i| !i.is_empty()).ok_or_else(|| anyhow!("falta el mando"));
    let profile = || req.profile.ok_or_else(|| anyhow!("falta el perfil"));
    let (path, body) = match req.action.as_str() {
        "read" => ("/api/cockpit/read".to_string(), json!({})),
        "save" => match (req.profile, name) {
            (Some(p), _) => ("/api/cockpit/save".into(), json!({ "profile": p })),
            (None, Some(n)) => ("/api/cockpit/save".into(), json!({ "name": n })),
            _ => return Err(anyhow!("falta el nombre del perfil")),
        },
        "restore" => match req.id.clone().filter(|i| !i.is_empty()) {
            Some(panel) => ("/api/cockpit/restore".into(), json!({ "id": panel })),
            None => ("/api/cockpit/restore".into(), json!({ "profile": req.profile })),
        },
        "auto" => ("/api/cockpit/auto".into(), json!({ "on": req.on.unwrap_or(false) })),
        "set" => {
            let value = req.value.ok_or_else(|| anyhow!("falta el valor"))?;
            ("/api/cockpit/set".into(), json!({ "id": id()?, "value": value }))
        }
        "keep" => ("/api/cockpit/keep".into(), json!({ "id": id()?, "keep": req.keep.unwrap_or(true) })),
        "select" | "profileAuto" | "delete" => {
            let verb = if req.action == "profileAuto" { "auto" } else { req.action.as_str() };
            (format!("/api/cockpit-profile/{verb}"), json!({ "profile": profile()? }))
        }
        "rename" => {
            let n = name.ok_or_else(|| anyhow!("falta el nombre del perfil"))?;
            ("/api/cockpit-profile/rename".into(), json!({ "profile": profile()?, "name": n }))
        }
        other => return Err(anyhow!("acción desconocida: {other}")),
    };
    let (ok, message) = techlog::api_post(source, &path, Some(body)).await?;
    tracing::info!(target: "techlog", "{} {reg} cockpit {}: ok={ok} {message}", source.id(), req.action);
    Ok(TlActionResult { ok, message, queued: false })
}

// ---------------------------------------------------------------------------
// Cabina — plano de asientos y equipos (galleys, baños, sistemas)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlSeat {
    pub seat: String,
    pub row: i64,
    pub letter: String,
    pub cycles: f64,
    /// Piezas rotas del asiento.
    pub broken: Vec<String>,
    /// Vida que le queda a su pieza más gastada (%).
    pub worst_remaining_pct: Option<f64>,
    pub blocked: bool,
    /// (02/11) Cabina propia: claves de lo roto y cada pieza con su orden.
    pub broken_keys: Vec<String>,
    pub items: Vec<TlSeatItem>,
}

/// (02/11) Pieza de un asiento de la Cabina propia (reemplazar = orden de pieza).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlSeatItem {
    pub id: i64,
    pub key: String,
    pub remaining_pct: f64,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlFitting {
    /// Id de pieza de TechLog: reparar / reemplazar = las órdenes de piezas.
    pub id: i64,
    pub key: String,
    pub name: String,
    pub position: Option<String>,
    pub ata: i64,
    pub system: String,
    pub kind: String,
    /// "Cycles" | "BlockHours" | "Days"…
    pub basis: String,
    pub life: f64,
    pub used: f64,
    pub remaining_pct: f64,
    /// "Ok" | "Degraded" | "Failed"
    pub status: String,
    pub defect_id: Option<i64>,
    pub mel: Option<String>,
    pub repairs: i64,
    pub repairs_left: Option<i64>,
    pub repair_to_pct: Option<f64>,
    pub worn: bool,
    /// Trabajo en curso en TechLog (tal cual lo da).
    pub work: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlCabinPlan {
    pub rows: i64,
    pub letters: String,
    pub seats: Vec<TlSeat>,
    pub fittings: Vec<TlFitting>,
    pub blocked_seats: i64,
}

fn parse_cabin_plan(v: &Value) -> TlCabinPlan {
    let seats: Vec<TlSeat> = list(v, "seats")
        .iter()
        .map(|s| TlSeat {
            seat: text(s, "seat").unwrap_or_default(),
            row: int(s, "row").unwrap_or(0),
            letter: text(s, "letter").unwrap_or_default(),
            cycles: num(s, "cycles").unwrap_or(0.0),
            broken: list(s, "broken")
                .iter()
                .filter_map(|b| match b {
                    Value::String(t) => Some(t.clone()),
                    o => text(o, "name").or_else(|| text(o, "title")),
                })
                .collect(),
            worst_remaining_pct: num(s, "worstRemainingPct"),
            blocked: flag(s, "blocked"),
            ..Default::default()
        })
        .collect();
    let fittings = list(v, "fittings")
        .iter()
        .map(|f| TlFitting {
            id: int(f, "id").unwrap_or(0),
            key: text(f, "key").unwrap_or_default(),
            name: text(f, "name").unwrap_or_default(),
            position: text(f, "position"),
            ata: int(f, "ata").unwrap_or(0),
            system: text(f, "system").unwrap_or_default(),
            kind: text(f, "kind").unwrap_or_default(),
            basis: text(f, "basis").unwrap_or_default(),
            life: num(f, "life").unwrap_or(0.0),
            used: num(f, "used").unwrap_or(0.0),
            remaining_pct: num(f, "remainingPct").unwrap_or(100.0),
            status: text(f, "status").unwrap_or_else(|| "Ok".into()),
            defect_id: int(f, "defectId"),
            mel: text(f, "mel"),
            repairs: int(f, "repairs").unwrap_or(0),
            repairs_left: int(f, "repairsLeft"),
            repair_to_pct: num(f, "repairToPct"),
            worn: flag(f, "worn"),
            work: v_some(f.get("work")),
        })
        .collect();
    let blocked_seats = int(v, "blockedSeats").unwrap_or_else(|| seats.iter().filter(|s| s.blocked).count() as i64);
    TlCabinPlan {
        rows: int(v, "rows").unwrap_or(0),
        letters: text(v, "letters").unwrap_or_default(),
        seats,
        fittings,
        blocked_seats,
    }
}

fn v_some(v: Option<&Value>) -> Option<Value> {
    v.filter(|x| !x.is_null()).cloned()
}

/// Nombres en el idioma de la app (la caché guarda los de TechLog).
pub(crate) fn localize_plan(p: &mut TlCabinPlan) {
    for s in &mut p.seats {
        for b in &mut s.broken {
            *b = tl_i18n::name(b);
        }
    }
    for f in &mut p.fittings {
        f.name = tl_i18n::name(&f.name);
        f.system = tl_i18n::name(&f.system);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlCabinPage {
    /// "live" | "not_active" | "closed" | "unsupported" | "unmanaged"
    pub state: &'static str,
    pub source: Option<TlSource>,
    pub active_registration: Option<String>,
    /// Fecha de la cabina guardada cuando no llega en vivo.
    pub saved_at: Option<String>,
    pub plan: Option<TlCabinPlan>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CabinCache {
    saved_at: String,
    plan: TlCabinPlan,
}

const CABIN_CACHE: &str = "techlog_cabin_plan:";

async fn cached_plan(pool: &SqlitePool, reg: &str) -> Option<CabinCache> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(format!("{CABIN_CACHE}{reg}"))
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    serde_json::from_str(&raw?).ok()
}

async fn save_plan(pool: &SqlitePool, reg: &str, plan: &TlCabinPlan) {
    // La página se refresca cada 15 s: sólo se escribe si la cabina cambió.
    static LAST: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let Ok(sig) = serde_json::to_string(plan) else { return };
    {
        let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        if last.get(reg) == Some(&sig) {
            return;
        }
        last.insert(reg.to_string(), sig);
    }
    let cache = CabinCache { saved_at: chrono::Utc::now().to_rfc3339(), plan: plan.clone() };
    let Ok(raw) = serde_json::to_string(&cache) else { return };
    let r = sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(format!("{CABIN_CACHE}{reg}"))
    .bind(raw)
    .execute(pool)
    .await;
    if let Err(e) = r {
        tracing::debug!(target: "techlog", "no se guardó la cabina de {reg}: {e}");
    }
}

/// Plano de cabina: en vivo si es el avión activo (y se guarda); si no, el
/// último visto.
pub async fn cabin_plan(pool: &SqlitePool, registration: &str) -> TlCabinPage {
    let reg = normalize_reg(registration);
    let Some(w) = locate(&reg).await else {
        return TlCabinPage {
            state: "unmanaged",
            source: None,
            active_registration: None,
            saved_at: None,
            plan: None,
        };
    };
    let mut page = TlCabinPage {
        state: w.state(&reg),
        source: Some(w.source),
        active_registration: w.active_display(),
        saved_at: None,
        plan: None,
    };
    if page.state == "live" {
        match techlog::api_get(w.source, "/api/cabin").await {
            Ok(v) => {
                let plan = parse_cabin_plan(&v);
                if !plan.seats.is_empty() || !plan.fittings.is_empty() {
                    save_plan(pool, &reg, &plan).await;
                }
                page.plan = Some(plan);
            }
            Err(e) => {
                tracing::debug!(target: "techlog", "{} sin plano de cabina: {e}", w.source.id());
                page.state = "unsupported";
            }
        }
    }
    if page.plan.is_none() {
        if let Some(c) = cached_plan(pool, &reg).await {
            page.saved_at = Some(c.saved_at);
            page.plan = Some(c.plan);
        }
    }
    if let Some(p) = page.plan.as_mut() {
        localize_plan(p);
    }
    page
}

// ---------------------------------------------------------------------------
// Asientos bloqueados (aviso del pre-vuelo)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlSeatInfo {
    /// Asientos de pasajero del avión.
    pub total: i64,
    /// Asientos bloqueados ("12C"…).
    pub blocked: Vec<String>,
}

/// Asiento bloqueado por un defecto: `seat_blocked` puede traer el asiento
/// o sólo marcarlo (entonces es su `position`).
fn blocked_seat(seat_blocked_text: Option<String>, seat_blocked_int: Option<i64>, position: Option<String>) -> Option<String> {
    let mark = seat_blocked_text
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false"))
        .map(Some)
        .or_else(|| seat_blocked_int.filter(|n| *n != 0).map(|_| None))?;
    let seat = match mark {
        Some(s) if s.chars().any(|c| c.is_ascii_alphabetic()) && s.chars().any(|c| c.is_ascii_digit()) => s,
        _ => position.unwrap_or_default(),
    };
    Some(seat.trim().to_uppercase())
}

pub async fn seat_info(pool: &SqlitePool, registration: &str) -> Result<Option<TlSeatInfo>> {
    let reg = normalize_reg(registration);
    let Some((_, mut conn, airframe, variant)) = open_airframe(&reg).await? else {
        return Ok(None);
    };
    let rows = sqlx::query(
        "SELECT position, seat_blocked FROM defect
         WHERE airframe_id = ?1 AND status IN ('Open', 'Deferred') AND seat_blocked IS NOT NULL",
    )
    .bind(airframe)
    .fetch_all(&mut conn)
    .await
    .unwrap_or_default();
    conn.close().await.ok();
    let mut blocked: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            blocked_seat(
                opt_str(r, "seat_blocked"),
                r.try_get::<Option<i64>, _>("seat_blocked").ok().flatten(),
                opt_str(r, "position"),
            )
        })
        .filter(|s| !s.is_empty())
        .collect();
    blocked.sort();
    blocked.dedup();
    // Plazas: las del último plano visto; si no, las del catálogo de SimFleet.
    let total = match cached_plan(pool, &reg).await {
        Some(c) if !c.plan.seats.is_empty() => c.plan.seats.len() as i64,
        _ => {
            let p = crate::mx::catalog::detect_profile(&[Some(variant.as_str())]);
            crate::mx::catalog::catalog(p.family).seats(&p.variant).round() as i64
        }
    };
    Ok(Some(TlSeatInfo { total, blocked }))
}

// ---------------------------------------------------------------------------
// Vuelos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlLandingSummary {
    pub airport: Option<String>,
    pub runway: Option<String>,
    pub vs_fpm: Option<f64>,
    pub peak_g: Option<f64>,
    pub ias: Option<f64>,
    pub bounces: Option<i64>,
    pub stable: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlFlightRow {
    pub id: i64,
    /// "closed" | "open"…
    pub status: String,
    pub opened_utc: Option<String>,
    pub out_utc: Option<String>,
    pub in_utc: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub block_min: Option<f64>,
    pub air_min: Option<f64>,
    pub score: Option<f64>,
    pub flags: Vec<String>,
    pub pax: Option<i64>,
    pub distance_nm: Option<f64>,
    pub max_alt_ft: Option<f64>,
    pub landing: Option<TlLandingSummary>,
    /// Eventos de aviso o alerta (no informativos).
    pub warnings: i64,
    /// Defectos y restricciones que dejó el vuelo.
    pub effects: i64,
}

/// Toma principal: la última que no fue un toque y despegue.
fn main_landing(ls: &[Value]) -> Option<&Value> {
    ls.iter().rev().find(|l| !flag(l, "touchAndGo")).or_else(|| ls.last())
}

fn landing_summary(l: &Value) -> TlLandingSummary {
    TlLandingSummary {
        airport: text(l, "airport"),
        runway: text(l, "runway"),
        vs_fpm: num(l, "vsFpm"),
        peak_g: num(l, "peakG"),
        ias: num(l, "ias"),
        bounces: int(l, "bounces"),
        stable: l.get("stable").and_then(|x| x.as_bool()),
    }
}

fn flight_row(r: &SqliteRow) -> TlFlightRow {
    let landings = col_json(r, "landings")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    TlFlightRow {
        id: r.try_get("id").unwrap_or(0),
        status: opt_str(r, "status").unwrap_or_default().to_lowercase(),
        opened_utc: opt_str(r, "opened_utc"),
        out_utc: opt_str(r, "out_utc"),
        in_utc: opt_str(r, "in_utc"),
        from: opt_str(r, "from_icao"),
        to: opt_str(r, "to_icao"),
        block_min: col_f64(r, "block_min"),
        air_min: col_f64(r, "air_min"),
        score: col_f64(r, "score"),
        flags: col_json(r, "flags")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .unwrap_or_default(),
        pax: col_i64(r, "pax"),
        distance_nm: col_f64(r, "distance_nm"),
        max_alt_ft: col_f64(r, "max_alt"),
        landing: main_landing(&landings).map(landing_summary),
        warnings: col_i64(r, "n_warn").unwrap_or(0),
        effects: col_i64(r, "n_eff").unwrap_or(0),
    }
}

const FLIGHT_COLS: &str = "f.*,
    (SELECT COUNT(*) FROM flight_event e WHERE e.flight_id = f.id
       AND e.severity IS NOT NULL AND e.severity != 'Info') AS n_warn,
    (SELECT COUNT(*) FROM flight_effect x WHERE x.flight_id = f.id) AS n_eff";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlFlightsPage {
    pub source: TlSource,
    pub app_name: String,
    pub flights: Vec<TlFlightRow>,
}

/// Vuelos de un avión en su TechLog (más recientes primero).
pub async fn flights(registration: &str, limit: i64) -> Result<Option<TlFlightsPage>> {
    let reg = normalize_reg(registration);
    let Some((source, mut conn, airframe, _)) = open_airframe(&reg).await? else {
        return Ok(None);
    };
    let flights = flight_rows(&mut conn, airframe, limit).await;
    conn.close().await.ok();
    Ok(Some(TlFlightsPage { source, app_name: source.app_name().into(), flights: flights? }))
}

async fn flight_rows(conn: &mut SqliteConnection, airframe: i64, limit: i64) -> Result<Vec<TlFlightRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {FLIGHT_COLS} FROM flight f WHERE f.airframe_id = ?1 ORDER BY f.opened_utc DESC LIMIT ?2"
    ))
    .bind(airframe)
    .bind(limit.clamp(1, 500))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.iter().map(flight_row).collect())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlEvent {
    pub id: i64,
    pub utc: Option<String>,
    /// Segundos desde la apertura del vuelo.
    pub t: Option<f64>,
    pub phase: Option<String>,
    pub alt_ft: Option<f64>,
    pub kind: String,
    /// "Info" | "Caution" | "Warning"
    pub severity: String,
    pub ata: Option<i64>,
    pub text: String,
    pub value: Option<f64>,
    pub limit: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlEffect {
    pub defect_id: Option<i64>,
    pub ata: Option<i64>,
    pub system: String,
    pub title: String,
    pub origin: Option<String>,
    pub restriction: Option<String>,
    pub complied: Option<bool>,
    pub compliance_note: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlWear {
    pub item: String,
    pub amount: f64,
    pub unit: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlReportCounts {
    pub good: i64,
    pub low: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlReportGroup {
    pub name: String,
    pub parts: i64,
    pub used_pct: f64,
    pub max_used_pct: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlReportItem {
    pub item: String,
    pub left_pct: f64,
    pub used_pct: f64,
    /// "good" | "low" | "failed"…
    pub stage: String,
}

/// Informe de vida de las piezas antes y después del vuelo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TlReport {
    pub before: TlReportCounts,
    pub after: TlReportCounts,
    pub health_before: f64,
    pub health_after: f64,
    pub groups: Vec<TlReportGroup>,
    pub aged_most: Vec<TlReportItem>,
    pub crossed: Vec<TlReportItem>,
}

/// Traza del vuelo (1 Hz en TechLog), reducida para dibujar el perfil.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlTrack {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<f64>>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlFlightDetail {
    pub source: Option<TlSource>,
    pub flight: TlFlightRow,
    pub off_utc: Option<String>,
    pub on_utc: Option<String>,
    pub fuel_out_kg: Option<f64>,
    pub fuel_off_kg: Option<f64>,
    pub fuel_on_kg: Option<f64>,
    pub fuel_in_kg: Option<f64>,
    pub zfw_kg: Option<f64>,
    pub tow_kg: Option<f64>,
    pub lw_kg: Option<f64>,
    /// Despegue y tomas tal cual los guarda TechLog.
    pub takeoff: Option<Value>,
    pub landings: Vec<Value>,
    pub summary: Option<String>,
    pub score_notes: Vec<String>,
    pub events: Vec<TlEvent>,
    pub effects: Vec<TlEffect>,
    pub wear: Vec<TlWear>,
    pub report: Option<TlReport>,
    pub track: Option<TlTrack>,
    /// Hay registro de texto del vuelo.
    pub has_log: bool,
    /// (02/11) Etapa 5b, vuelos del Tech Log propio: combustible gastado
    /// (SimFleet no guarda OUT/OFF/ON/IN) y puntos perdidos por regla.
    pub fuel_burn_kg: Option<f64>,
    pub score_items: Vec<TlScoreItem>,
}

/// (02/11) Regla del FlightBook en la que se perdieron puntos.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlScoreItem {
    pub rule_id: String,
    pub label: String,
    pub lost: f64,
}

pub(crate) const TRACK_MAX_ROWS: usize = 900;

/// Traza gzip de `flight_track` → columnas + filas (como mucho 900).
fn decode_track(gz: &[u8]) -> Option<TlTrack> {
    let mut txt = String::new();
    flate2::read::GzDecoder::new(gz).read_to_string(&mut txt).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    let columns: Vec<String> = list(&v, "columns")
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();
    let rows = list(&v, "rows");
    if columns.is_empty() || rows.is_empty() {
        return None;
    }
    Some(TlTrack { columns, rows: downsample(rows, TRACK_MAX_ROWS) })
}

fn downsample(rows: &[Value], max: usize) -> Vec<Vec<Option<f64>>> {
    let step = rows.len().div_ceil(max.max(1)).max(1);
    let row = |r: &Value| -> Vec<Option<f64>> {
        r.as_array()
            .map(|a| a.iter().map(|x| x.as_f64()).collect())
            .unwrap_or_default()
    };
    let mut out: Vec<Vec<Option<f64>>> = rows.iter().step_by(step).map(row).collect();
    if (rows.len() - 1) % step != 0 {
        if let Some(last) = rows.last() {
            out.push(row(last));
        }
    }
    out
}

pub(crate) fn localize_detail(d: &mut TlFlightDetail) {
    let s = |x: &mut Option<String>| {
        if let Some(v) = x.as_mut() {
            *v = tl_i18n::sentence(v);
        }
    };
    s(&mut d.summary);
    for n in &mut d.score_notes {
        *n = tl_i18n::sentence(n);
    }
    for e in &mut d.events {
        e.text = tl_i18n::sentence(&e.text);
    }
    for x in &mut d.effects {
        x.title = tl_i18n::sentence(&x.title);
        x.system = tl_i18n::name(&x.system);
        s(&mut x.restriction);
        s(&mut x.compliance_note);
        s(&mut x.outcome);
    }
    for w in &mut d.wear {
        w.item = tl_i18n::name(&w.item);
        w.unit = tl_i18n::name(&w.unit);
        s(&mut w.detail);
    }
    if let Some(r) = d.report.as_mut() {
        for g in &mut r.groups {
            g.name = tl_i18n::name(&g.name);
        }
        for i in r.aged_most.iter_mut().chain(r.crossed.iter_mut()) {
            i.item = tl_i18n::name(&i.item);
        }
    }
}

/// Detalle de un vuelo de ese avión (informe, cronología, despegue y tomas,
/// sistemas, desgaste y perfil).
pub async fn flight_detail(registration: &str, flight_id: i64) -> Result<Option<TlFlightDetail>> {
    let reg = normalize_reg(registration);
    let Some((source, mut conn, airframe, _)) = open_airframe(&reg).await? else {
        return Ok(None);
    };
    let d = detail_from(&mut conn, airframe, flight_id).await;
    conn.close().await.ok();
    Ok(d?.map(|mut d| {
        d.source = Some(source);
        localize_detail(&mut d);
        d
    }))
}

async fn detail_from(conn: &mut SqliteConnection, airframe: i64, flight_id: i64) -> Result<Option<TlFlightDetail>> {
    let Some(r) = sqlx::query(&format!(
        "SELECT {FLIGHT_COLS} FROM flight f WHERE f.id = ?1 AND f.airframe_id = ?2"
    ))
    .bind(flight_id)
    .bind(airframe)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let events = sqlx::query("SELECT * FROM flight_event WHERE flight_id = ?1 ORDER BY id")
        .bind(flight_id)
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_default()
        .iter()
        .map(|e| TlEvent {
            id: e.try_get("id").unwrap_or(0),
            utc: opt_str(e, "utc"),
            t: col_f64(e, "t"),
            phase: opt_str(e, "phase"),
            alt_ft: col_f64(e, "alt_ft"),
            kind: opt_str(e, "type").unwrap_or_default(),
            severity: opt_str(e, "severity").unwrap_or_else(|| "Info".into()),
            ata: col_i64(e, "ata"),
            text: opt_str(e, "text").unwrap_or_default(),
            value: col_f64(e, "value"),
            limit: col_f64(e, "limit_value"),
        })
        .collect();
    let effects = sqlx::query("SELECT * FROM flight_effect WHERE flight_id = ?1 ORDER BY id")
        .bind(flight_id)
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_default()
        .iter()
        .map(|x| TlEffect {
            defect_id: col_i64(x, "defect_id"),
            ata: col_i64(x, "ata"),
            system: opt_str(x, "system").unwrap_or_default(),
            title: opt_str(x, "title").unwrap_or_default(),
            origin: opt_str(x, "origin"),
            restriction: opt_str(x, "restriction"),
            complied: col_i64(x, "complied").map(|n| n != 0),
            compliance_note: opt_str(x, "compliance_note"),
            outcome: opt_str(x, "outcome"),
        })
        .collect();
    let wear = sqlx::query("SELECT * FROM flight_wear WHERE flight_id = ?1")
        .bind(flight_id)
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_default()
        .iter()
        .map(|w| TlWear {
            item: opt_str(w, "item").unwrap_or_default(),
            amount: col_f64(w, "amount").unwrap_or(0.0),
            unit: opt_str(w, "unit").unwrap_or_default(),
            detail: opt_str(w, "detail"),
        })
        .filter(|w| w.amount.abs() > 0.0001)
        .collect();
    let report: Option<TlReport> = sqlx::query_scalar::<_, String>("SELECT value FROM setting WHERE key = ?1")
        .bind(format!("flight-report:{flight_id}"))
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok());
    let track = sqlx::query_scalar::<_, Vec<u8>>("SELECT data FROM flight_track WHERE flight_id = ?1")
        .bind(flight_id)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten()
        .and_then(|gz| decode_track(&gz));

    let d = TlFlightDetail {
        source: None,
        flight: flight_row(&r),
        off_utc: opt_str(&r, "off_utc"),
        on_utc: opt_str(&r, "on_utc"),
        fuel_out_kg: col_f64(&r, "fuel_out"),
        fuel_off_kg: col_f64(&r, "fuel_off"),
        fuel_on_kg: col_f64(&r, "fuel_on"),
        fuel_in_kg: col_f64(&r, "fuel_in"),
        zfw_kg: col_f64(&r, "zfw"),
        tow_kg: col_f64(&r, "tow"),
        lw_kg: col_f64(&r, "lw"),
        takeoff: col_json(&r, "takeoff").filter(|v| v.is_object()),
        landings: col_json(&r, "landings")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default(),
        summary: opt_str(&r, "summary"),
        score_notes: col_json(&r, "score_notes")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
            .unwrap_or_default(),
        events,
        effects,
        wear,
        report,
        track,
        has_log: opt_str(&r, "log_path").is_some_and(|p| std::path::Path::new(&p).is_file()),
        ..Default::default()
    };
    Ok(Some(d))
}

/// Registro de texto de un vuelo (su archivo; si no, la API).
async fn flight_log_text(reg: &str, flight_id: i64) -> Result<Option<String>> {
    let Some((source, mut conn, airframe, _)) = open_airframe(reg).await? else {
        return Ok(None);
    };
    let path: Option<String> = sqlx::query_scalar("SELECT log_path FROM flight WHERE id = ?1 AND airframe_id = ?2")
        .bind(flight_id)
        .bind(airframe)
        .fetch_optional(&mut conn)
        .await?
        .flatten();
    conn.close().await.ok();
    if let Some(txt) = path.and_then(|p| std::fs::read_to_string(p).ok()) {
        return Ok(Some(txt));
    }
    if techlog::is_running(source).await {
        if let Ok(txt) = techlog::api_get_text(source, &format!("/api/flights/{flight_id}/log")).await {
            return Ok(Some(txt));
        }
    }
    Ok(None)
}

/// JSON del vuelo: el exacto de TechLog si está abierto; si no, el detalle.
async fn flight_json(reg: &str, flight_id: i64) -> Result<Option<String>> {
    let Some(source) = techlog::managed_registrations().await.get(reg).copied() else {
        return Ok(None);
    };
    // Que el vuelo sea de ESTE avión antes de pedírselo a TechLog.
    let Some(detail) = flight_detail(reg, flight_id).await? else {
        return Ok(None);
    };
    if techlog::is_running(source).await {
        // Tal cual lo exporta TechLog (sin reordenar sus claves).
        if let Ok(txt) = techlog::api_get_text(source, &format!("/api/flights/{flight_id}/export")).await {
            if serde_json::from_str::<Value>(&txt).is_ok() {
                return Ok(Some(txt));
            }
        }
    }
    Ok(Some(serde_json::to_string_pretty(&detail)?))
}

/// Guarda el registro (`kind = "log"`) o el JSON (`"json"`) en `path`.
pub async fn flight_save(registration: &str, flight_id: i64, kind: &str, path: &str) -> Result<()> {
    let reg = normalize_reg(registration);
    let content = match kind {
        "log" => flight_log_text(&reg, flight_id).await?,
        "json" => flight_json(&reg, flight_id).await?,
        other => return Err(anyhow!("formato desconocido: {other}")),
    };
    let content = content.ok_or_else(|| anyhow!("not_found"))?;
    std::fs::write(path, content)?;
    tracing::info!(target: "techlog", "vuelo #{flight_id} de {reg} guardado ({kind}) en {path}");
    Ok(())
}

/// Borra un vuelo en TechLog (irreversible; TechLog tiene que estar abierto).
pub async fn flight_delete(registration: &str, flight_id: i64) -> Result<TlActionResult> {
    let reg = normalize_reg(registration);
    let Some((source, mut conn, airframe, _)) = open_airframe(&reg).await? else {
        return Err(anyhow!("not_found"));
    };
    let mine: Option<i64> = sqlx::query_scalar("SELECT id FROM flight WHERE id = ?1 AND airframe_id = ?2")
        .bind(flight_id)
        .bind(airframe)
        .fetch_optional(&mut conn)
        .await?;
    conn.close().await.ok();
    if mine.is_none() {
        return Err(anyhow!("not_found"));
    }
    if !techlog::is_running(source).await {
        return Err(anyhow!("closed"));
    }
    let (ok, message) = techlog::api_post(source, &format!("/api/flights/{flight_id}/delete"), None).await?;
    tracing::info!(target: "techlog", "{} {reg} vuelo #{flight_id} borrado: ok={ok} {message}", source.id());
    Ok(TlActionResult { ok, message, queued: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_landing_skips_touch_and_goes() {
        let ls = vec![
            json!({ "index": 1, "vsFpm": -300, "touchAndGo": false }),
            json!({ "index": 2, "vsFpm": -900, "touchAndGo": true }),
        ];
        assert_eq!(int(main_landing(&ls).unwrap(), "index"), Some(1));
        let only_tg = vec![json!({ "index": 1, "touchAndGo": true })];
        assert_eq!(int(main_landing(&only_tg).unwrap(), "index"), Some(1));
        assert!(main_landing(&[]).is_none());
    }

    #[test]
    fn the_track_is_thinned_but_keeps_its_ends() {
        let rows: Vec<Value> = (0..2500).map(|i| json!([i, 1.0, null])).collect();
        let out = downsample(&rows, 900);
        assert!(out.len() <= 901, "{} filas", out.len());
        assert_eq!(out.first().unwrap()[0], Some(0.0));
        assert_eq!(out.last().unwrap()[0], Some(2499.0));
        assert_eq!(out[0][2], None);
        let short: Vec<Value> = (0..10).map(|i| json!([i])).collect();
        assert_eq!(downsample(&short, 900).len(), 10);
    }

    #[test]
    fn a_gzip_track_is_decoded() {
        use std::io::Write;
        let raw = json!({ "columns": ["t", "alt"], "rows": [[0, 100], [1, 120.5]] }).to_string();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(raw.as_bytes()).unwrap();
        let t = decode_track(&gz.finish().unwrap()).unwrap();
        assert_eq!(t.columns, vec!["t", "alt"]);
        assert_eq!(t.rows, vec![vec![Some(0.0), Some(100.0)], vec![Some(1.0), Some(120.5)]]);
        assert!(decode_track(b"no es gzip").is_none());
    }

    #[test]
    fn blocked_seats_come_from_the_mark_or_the_position() {
        assert_eq!(blocked_seat(Some("12C".into()), None, Some("2F".into())), Some("12C".into()));
        assert_eq!(blocked_seat(None, Some(1), Some("2f".into())), Some("2F".into()));
        assert_eq!(blocked_seat(Some("1".into()), None, Some("7A".into())), Some("7A".into()));
        assert_eq!(blocked_seat(None, Some(0), Some("2F".into())), None);
        assert_eq!(blocked_seat(Some("".into()), None, Some("2F".into())), None);
    }

    #[test]
    fn the_cockpit_and_cabin_pages_parse_techlog_json() {
        let c = parse_cockpit(&json!({
            "registration": "TL-DEMO", "connected": false, "busy": null, "restore": true,
            "zones": [{ "key": "overhead", "name": "Overhead", "panels": [{ "key": "irs", "name": "IRS",
                "controls": [{ "id": "IRS_1", "name": "IRS L", "kind": "step", "never": null,
                    "positions": [{ "value": 0, "label": "OFF" }, { "value": 1, "label": "NAV" }],
                    "saved": 1, "live": null, "keep": true }] }] }],
            "profiles": [{ "id": 3, "name": "Turnaround", "auto": true }], "profileId": 3
        }));
        assert!(c.restore && !c.connected && c.busy.is_none());
        let ctl = &c.zones[0].panels[0].controls[0];
        assert_eq!((ctl.saved, ctl.live, ctl.never), (Some(1.0), None, false));
        assert_eq!(ctl.positions[1].label, "NAV");
        assert_eq!((c.profiles[0].id, c.profile_id), (3, Some(3)));

        let p = parse_cabin_plan(&json!({
            "rows": 31, "letters": "ABCDEF", "blockedSeats": 1,
            "seats": [{ "seat": "1A", "row": 1, "letter": "A", "cycles": 8091.8, "broken": ["Tray table"],
                "worstRemainingPct": 3.6, "blocked": true }],
            "fittings": [{ "id": 522, "name": "Cabin crew seat AFT L", "remainingPct": 66.5, "status": "Ok",
                "defectId": null, "repairToPct": 70, "work": null }]
        }));
        assert_eq!((p.rows, p.letters.as_str(), p.blocked_seats), (31, "ABCDEF", 1));
        assert_eq!(p.seats[0].broken, vec!["Tray table"]);
        assert_eq!((p.fittings[0].id, p.fittings[0].repair_to_pct, p.fittings[0].work.is_none()), (522, Some(70.0), true));
    }

    /// Contra una COPIA de `techlog.db` (nunca la de TechLog):
    /// `TL_DB=<copia> cargo test --lib techlog_pages_real_copy -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn techlog_pages_real_copy() {
        use sqlx::ConnectOptions;
        let Ok(path) = std::env::var("TL_DB") else { return };
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .connect()
            .await
            .unwrap();
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM airframe").fetch_all(&mut conn).await.unwrap();
        for af in ids {
            let rows = flight_rows(&mut conn, af, 50).await.unwrap();
            for r in &rows {
                println!(
                    "airframe {af} #{} {:?}→{:?} score {:?} block {:?} warn {} eff {} landing {:?}",
                    r.id, r.from, r.to, r.score, r.block_min, r.warnings, r.effects, r.landing
                );
            }
            if let Some(first) = rows.first() {
                let d = detail_from(&mut conn, af, first.id).await.unwrap().unwrap();
                let rep = d.report.as_ref().map(|r| (r.health_before, r.health_after, r.groups.len()));
                println!(
                    "detalle #{}: {} eventos, {} efectos, {} desgastes, informe {:?}, traza {:?}, despegue {}, tomas {}, registro {}",
                    first.id,
                    d.events.len(),
                    d.effects.len(),
                    d.wear.len(),
                    rep,
                    d.track.as_ref().map(|t| (t.columns.len(), t.rows.len())),
                    d.takeoff.is_some(),
                    d.landings.len(),
                    d.has_log
                );
                assert!(d.track.as_ref().map_or(true, |t| t.rows.len() <= TRACK_MAX_ROWS + 1));
            }
        }
    }
}
