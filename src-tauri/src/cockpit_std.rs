//! (02/11) Etapa 5b — Cockpit del Tech Log propio (aviones sin Fenix/PMDG
//! TechLog): perfiles de los interruptores ESTÁNDAR del simulador. El
//! watcher los lee cada segundo por SimConnect (`on_snapshot`) y manda las
//! órdenes de la cola (`take_queue`) como eventos estándar. Los aviones con
//! sistemas propios (FlyByWire, iniBuilds, iFly…) pueden no responder a
//! algunos: decisión del usuario, avisada en la propuesta.
//!
//! «Restaurar al cargar en frío y oscuro»: cuando el avión acaba de cargarse
//! (3 min) en tierra, con freno de parking y motores apagados, se pone UNA vez
//! su perfil «al cargar» (o el elegido si está marcado «restaurar al cargar»).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::airline_economy::normalize_reg;
use crate::techlog::{TlActionResult, TlSource};
use crate::techlog_pages::{
    TlCockpit, TlCockpitControl, TlCockpitPage, TlCockpitPanel, TlCockpitPosition, TlCockpitProfile, TlCockpitRequest,
    TlCockpitZone,
};

/// Cómo se mueve un interruptor.
#[derive(Debug, Clone, Copy)]
pub enum Set {
    /// Evento que recibe el valor (0/1).
    Value(&'static str),
    /// Evento que alterna: sólo se manda si el valor real es otro.
    Toggle(&'static str),
    /// APU: arrancar / apagar.
    Apu,
    /// Código del transpondedor (BCD16).
    Xpdr,
}

#[derive(Debug, Clone, Copy)]
pub struct Switch {
    pub id: &'static str,
    pub zone: &'static str,
    pub panel: &'static str,
    /// Nombre (inglés, como en la cabina); el front lo traduce por `id`.
    pub name: &'static str,
    pub var: &'static str,
    pub unit: &'static str,
    pub set: Set,
}

const fn sw(id: &'static str, zone: &'static str, panel: &'static str, name: &'static str, var: &'static str, set: Set) -> Switch {
    Switch { id, zone, panel, name, var, unit: "bool", set }
}

/// Los cuatro grupos elegidos por el usuario.
pub const SWITCHES: &[Switch] = &[
    sw("LIGHT_NAV", "overhead", "lights", "NAV", "LIGHT NAV", Set::Value("NAV_LIGHTS_SET")),
    sw("LIGHT_BEACON", "overhead", "lights", "BEACON", "LIGHT BEACON", Set::Value("BEACON_LIGHTS_SET")),
    sw("LIGHT_STROBE", "overhead", "lights", "STROBE", "LIGHT STROBE", Set::Value("STROBES_SET")),
    sw("LIGHT_TAXI", "overhead", "lights", "TAXI", "LIGHT TAXI", Set::Value("TAXI_LIGHTS_SET")),
    sw("LIGHT_LANDING", "overhead", "lights", "LANDING", "LIGHT LANDING", Set::Value("LANDING_LIGHTS_SET")),
    sw("LIGHT_LOGO", "overhead", "lights", "LOGO", "LIGHT LOGO", Set::Value("LOGO_LIGHTS_SET")),
    sw("LIGHT_WING", "overhead", "lights", "WING", "LIGHT WING", Set::Value("WING_LIGHTS_SET")),
    sw("LIGHT_CABIN", "overhead", "interior", "CABIN", "LIGHT CABIN", Set::Value("CABIN_LIGHTS_SET")),
    sw("LIGHT_PANEL", "overhead", "interior", "PANEL", "LIGHT PANEL", Set::Value("PANEL_LIGHTS_SET")),
    sw("BATTERY", "overhead", "elec", "BAT", "ELECTRICAL MASTER BATTERY", Set::Value("MASTER_BATTERY_SET")),
    sw("AVIONICS", "overhead", "elec", "AVIONICS", "AVIONICS MASTER SWITCH", Set::Value("AVIONICS_MASTER_SET")),
    sw("APU", "overhead", "elec", "APU", "APU SWITCH", Set::Apu),
    sw("FUEL_PUMP_1", "overhead", "fuel", "PUMP 1", "GENERAL ENG FUEL PUMP SWITCH:1", Set::Value("ELECT_FUEL_PUMP1_SET")),
    sw("FUEL_PUMP_2", "overhead", "fuel", "PUMP 2", "GENERAL ENG FUEL PUMP SWITCH:2", Set::Value("ELECT_FUEL_PUMP2_SET")),
    sw("SEATBELTS", "overhead", "signs", "SEAT BELTS", "CABIN SEATBELTS ALERT SWITCH", Set::Toggle("CABIN_SEATBELTS_ALERT_SWITCH_TOGGLE")),
    sw("NO_SMOKING", "overhead", "signs", "NO SMOKING", "CABIN NO SMOKING ALERT SWITCH", Set::Toggle("CABIN_NO_SMOKING_ALERT_SWITCH_TOGGLE")),
    sw("PITOT_HEAT", "overhead", "ice", "PROBE HEAT", "PITOT HEAT", Set::Value("PITOT_HEAT_SET")),
    sw("ENG_ANTI_ICE_1", "overhead", "ice", "ENG 1 ANTI ICE", "ENG ANTI ICE:1", Set::Value("ANTI_ICE_SET_ENG1")),
    sw("ENG_ANTI_ICE_2", "overhead", "ice", "ENG 2 ANTI ICE", "ENG ANTI ICE:2", Set::Value("ANTI_ICE_SET_ENG2")),
    sw("PARKING_BRAKE", "pedestal", "brake", "PARKING BRAKE", "BRAKE PARKING POSITION", Set::Value("PARKING_BRAKE_SET")),
    Switch { id: "XPDR_CODE", zone: "pedestal", panel: "xpdr", name: "XPDR", var: "TRANSPONDER CODE:1", unit: "Bco16", set: Set::Xpdr },
];

/// Después de los interruptores, lo que hace falta para «frío y oscuro».
pub const DETECT: &[(&str, &str)] = &[
    ("SIM ON GROUND", "bool"),
    ("GENERAL ENG COMBUSTION:1", "bool"),
    ("GENERAL ENG COMBUSTION:2", "bool"),
];

/// Variables a pedir al simulador, en orden (interruptores + detección).
pub fn sim_vars() -> Vec<(&'static str, &'static str)> {
    SWITCHES.iter().map(|s| (s.var, s.unit)).chain(DETECT.iter().copied()).collect()
}

fn bcd_to_dec(v: f64) -> f64 {
    let b = v.max(0.0) as u32;
    let d = |n: u32| (b >> (4 * n)) & 0xF;
    (d(3) * 1000 + d(2) * 100 + d(1) * 10 + d(0)) as f64
}

fn dec_to_bcd(v: f64) -> u32 {
    let n = v.round().clamp(0.0, 7777.0) as u32;
    ((n / 1000) % 10) << 12 | ((n / 100) % 10) << 8 | ((n / 10) % 10) << 4 | (n % 10)
}

// ---------------------------------------------------------------------------
// Estado compartido con el watcher
// ---------------------------------------------------------------------------

struct Live {
    at: Instant,
    at_utc: DateTime<Utc>,
    /// Un valor por interruptor (el transpondedor ya en decimal).
    values: Vec<f64>,
}

fn live() -> &'static Mutex<Option<Live>> {
    static L: OnceLock<Mutex<Option<Live>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(None))
}

/// (matrícula normalizada del avión cargado, desde cuándo).
fn aircraft() -> &'static Mutex<Option<(String, Instant)>> {
    static A: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

fn queue() -> &'static Mutex<Vec<(&'static str, u32)>> {
    static Q: OnceLock<Mutex<Vec<(&'static str, u32)>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Vec::new()))
}

static APPLYING: AtomicBool = AtomicBool::new(false);

fn applied_for() -> &'static Mutex<Option<String>> {
    static A: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

static POOL: OnceLock<SqlitePool> = OnceLock::new();

/// Al arrancar: la base para el perfil automático.
pub fn init(pool: SqlitePool) {
    let _ = POOL.set(pool);
}

/// El watcher conoce la matrícula del avión cargado.
pub fn set_aircraft(registration: Option<String>) {
    let reg = registration.map(|r| normalize_reg(&r)).filter(|r| !r.is_empty());
    let mut a = aircraft().lock().unwrap_or_else(|e| e.into_inner());
    if a.as_ref().map(|(r, _)| r) != reg.as_ref() {
        *a = reg.map(|r| (r, Instant::now()));
        *applied_for().lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// El watcher perdió el simulador.
pub fn disconnected() {
    *live().lock().unwrap_or_else(|e| e.into_inner()) = None;
    *aircraft().lock().unwrap_or_else(|e| e.into_inner()) = None;
    *applied_for().lock().unwrap_or_else(|e| e.into_inner()) = None;
    queue().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Órdenes pendientes para el simulador (las vacía).
pub fn take_queue() -> Vec<(&'static str, u32)> {
    std::mem::take(&mut *queue().lock().unwrap_or_else(|e| e.into_inner()))
}

/// Lectura de cada segundo: interruptores y luego `DETECT`.
pub fn on_snapshot(raw: &[f64]) {
    if raw.len() < SWITCHES.len() + DETECT.len() {
        return;
    }
    let values: Vec<f64> = SWITCHES
        .iter()
        .zip(raw)
        .map(|(s, v)| if matches!(s.set, Set::Xpdr) { bcd_to_dec(*v) } else { v.round() })
        .collect();
    let detect = &raw[SWITCHES.len()..];
    let on_ground = detect[0] >= 0.5;
    let engines_off = detect[1] < 0.5 && detect[2] < 0.5;
    let brake = index_of("PARKING_BRAKE").map(|i| values[i] >= 0.5).unwrap_or(false);
    *live().lock().unwrap_or_else(|e| e.into_inner()) = Some(Live { at: Instant::now(), at_utc: Utc::now(), values });

    // «Frío y oscuro» recién cargado: una vez por carga.
    let loaded = aircraft().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some((reg, since)) = loaded else { return };
    if since.elapsed() > Duration::from_secs(180) || !(on_ground && engines_off && brake) {
        return;
    }
    {
        let mut done = applied_for().lock().unwrap_or_else(|e| e.into_inner());
        if done.as_deref() == Some(reg.as_str()) || APPLYING.load(Ordering::Relaxed) {
            return;
        }
        *done = Some(reg.clone());
    }
    let Some(pool) = POOL.get().cloned() else { return };
    APPLYING.store(true, Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        match apply_on_load(&pool, &reg).await {
            Ok(Some((name, n))) => tracing::info!(target: "cockpit", "{reg}: perfil «{name}» puesto al cargar en frío y oscuro ({n} interruptores)"),
            Ok(None) => {}
            Err(e) => tracing::warn!(target: "cockpit", "{reg}: perfil al cargar: {e:#}"),
        }
        APPLYING.store(false, Ordering::Relaxed);
    });
}

fn index_of(id: &str) -> Option<usize> {
    SWITCHES.iter().position(|s| s.id == id)
}

/// Valores en vivo si el avión cargado es esta matrícula (lectura < 5 s).
fn live_for(reg: &str) -> Option<(DateTime<Utc>, Vec<f64>)> {
    let a = aircraft().lock().unwrap_or_else(|e| e.into_inner()).clone();
    if a.map(|(r, _)| r).as_deref() != Some(reg) {
        return None;
    }
    let l = live().lock().unwrap_or_else(|e| e.into_inner());
    l.as_ref()
        .filter(|l| l.at.elapsed() < Duration::from_secs(5))
        .map(|l| (l.at_utc, l.values.clone()))
}

/// Orden para llevar un interruptor de `now` a `target`.
fn command(s: &Switch, now: Option<f64>, target: f64) -> Option<(&'static str, u32)> {
    if now.is_some_and(|v| (v - target).abs() < 0.5) {
        return None;
    }
    match s.set {
        Set::Value(ev) => Some((ev, u32::from(target >= 0.5))),
        Set::Toggle(ev) => now.map(|_| (ev, 0)),
        Set::Apu => Some(if target >= 0.5 { ("APU_STARTER", 0) } else { ("APU_OFF_SWITCH", 0) }),
        Set::Xpdr => Some(("XPNDR_SET", dec_to_bcd(target))),
    }
}

// ---------------------------------------------------------------------------
// Perfiles (base de SimFleet)
// ---------------------------------------------------------------------------

struct Profile {
    id: i64,
    name: String,
    auto: bool,
    values: HashMap<String, f64>,
    saved_at: String,
    airport: Option<String>,
}

struct State {
    profile_id: Option<i64>,
    restore_on_load: bool,
    skip: HashSet<String>,
}

async fn profiles(pool: &SqlitePool, reg: &str) -> Result<Vec<Profile>> {
    Ok(sqlx::query("SELECT * FROM cockpit_std_profile WHERE registration = ?1 ORDER BY id")
        .bind(reg)
        .fetch_all(pool)
        .await?
        .iter()
        .map(|r| Profile {
            id: r.try_get("id").unwrap_or(0),
            name: r.try_get("name").unwrap_or_default(),
            auto: r.try_get::<i64, _>("auto").unwrap_or(0) != 0,
            values: r
                .try_get::<String, _>("switches")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            saved_at: r.try_get("saved_at").unwrap_or_default(),
            airport: r.try_get("saved_airport").ok().flatten(),
        })
        .collect())
}

async fn state(pool: &SqlitePool, reg: &str) -> Result<State> {
    let row = sqlx::query("SELECT * FROM cockpit_std_state WHERE registration = ?1")
        .bind(reg)
        .fetch_optional(pool)
        .await?;
    Ok(State {
        profile_id: row.as_ref().and_then(|r| r.try_get("profile_id").ok().flatten()),
        restore_on_load: row.as_ref().is_some_and(|r| r.try_get::<i64, _>("restore_on_load").unwrap_or(0) != 0),
        skip: row
            .as_ref()
            .and_then(|r| r.try_get::<String, _>("skip").ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    })
}

async fn save_state(pool: &SqlitePool, reg: &str, st: &State) -> Result<()> {
    let mut skip: Vec<&String> = st.skip.iter().collect();
    skip.sort();
    sqlx::query(
        "INSERT INTO cockpit_std_state (registration, profile_id, restore_on_load, skip) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(registration) DO UPDATE SET profile_id = excluded.profile_id,
           restore_on_load = excluded.restore_on_load, skip = excluded.skip",
    )
    .bind(reg)
    .bind(st.profile_id)
    .bind(st.restore_on_load)
    .bind(serde_json::to_string(&skip)?)
    .execute(pool)
    .await?;
    Ok(())
}

/// Aeropuerto donde está el avión (el último del FlightBook con esa matrícula).
async fn airport_of(pool: &SqlitePool, reg: &str) -> Option<String> {
    let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT aircraft_registration, destination_icao FROM flight_log
         WHERE aircraft_registration IS NOT NULL ORDER BY started_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .ok()?;
    rows.into_iter()
        .find(|(r, _)| r.as_deref().map(normalize_reg).as_deref() == Some(reg))
        .and_then(|(_, icao)| icao)
}

/// Órdenes para poner un perfil (sólo los interruptores que entran y difieren).
fn restore_commands(p: &Profile, st: &State, live: Option<&[f64]>, only_panel: Option<&str>) -> Vec<(&'static str, u32)> {
    SWITCHES
        .iter()
        .enumerate()
        .filter(|(_, s)| !st.skip.contains(s.id) && only_panel.map_or(true, |k| s.panel == k))
        .filter_map(|(i, s)| {
            let target = *p.values.get(s.id)?;
            command(s, live.and_then(|v| v.get(i).copied()), target)
        })
        .collect()
}

async fn apply_on_load(pool: &SqlitePool, reg: &str) -> Result<Option<(String, usize)>> {
    let st = state(pool, reg).await?;
    let all = profiles(pool, reg).await?;
    let pick = all
        .iter()
        .find(|p| p.auto)
        .or_else(|| st.restore_on_load.then(|| all.iter().find(|p| Some(p.id) == st.profile_id)).flatten());
    let Some(p) = pick else { return Ok(None) };
    let live = live_for(reg).map(|(_, v)| v);
    let cmds = restore_commands(p, &st, live.as_deref(), None);
    let n = cmds.len();
    queue().lock().unwrap_or_else(|e| e.into_inner()).extend(cmds);
    Ok(Some((p.name.clone(), n)))
}

// ---------------------------------------------------------------------------
// Página y órdenes (mismos tipos que el Cockpit de TechLog)
// ---------------------------------------------------------------------------

pub async fn page(pool: &SqlitePool, registration: &str) -> Result<TlCockpitPage> {
    let reg = normalize_reg(registration);
    let st = state(pool, &reg).await?;
    let all = profiles(pool, &reg).await?;
    let selected = all.iter().find(|p| Some(p.id) == st.profile_id).or(all.first());
    let live = live_for(&reg);
    let loaded = aircraft().lock().unwrap_or_else(|e| e.into_inner()).clone().map(|(r, _)| r);
    let mut zones: Vec<TlCockpitZone> = Vec::new();
    for (i, s) in SWITCHES.iter().enumerate() {
        let control = TlCockpitControl {
            id: s.id.into(),
            name: s.name.into(),
            kind: if matches!(s.set, Set::Xpdr) { "code".into() } else { "step".into() },
            max: None,
            positions: if matches!(s.set, Set::Xpdr) {
                Vec::new()
            } else {
                vec![
                    TlCockpitPosition { value: 0.0, label: "OFF".into() },
                    TlCockpitPosition { value: 1.0, label: "ON".into() },
                ]
            },
            saved: selected.and_then(|p| p.values.get(s.id).copied()),
            live: live.as_ref().and_then(|(_, v)| v.get(i).copied()),
            keep: !st.skip.contains(s.id),
            never: false,
        };
        let zi = match zones.iter().position(|z| z.key == s.zone) {
            Some(i) => i,
            None => {
                zones.push(TlCockpitZone { key: s.zone.into(), name: s.zone.into(), panels: Vec::new() });
                zones.len() - 1
            }
        };
        let z = &mut zones[zi];
        match z.panels.iter_mut().find(|p| p.key == s.panel) {
            Some(p) => p.controls.push(control),
            None => z.panels.push(TlCockpitPanel { key: s.panel.into(), name: s.panel.into(), controls: vec![control] }),
        }
    }
    Ok(TlCockpitPage {
        state: "live",
        source: Some(TlSource::Simfleet),
        active_registration: loaded.filter(|r| r != &reg),
        cockpit: Some(TlCockpit {
            registration: Some(registration.trim().to_uppercase()),
            connected: live.is_some(),
            busy: None,
            last: None,
            restore: st.restore_on_load,
            saved_utc: selected.map(|p| p.saved_at.clone()),
            saved_airport: selected.and_then(|p| p.airport.clone()),
            live_utc: live.map(|(t, _)| t.to_rfc3339()),
            zones,
            profiles: all.iter().map(|p| TlCockpitProfile { id: p.id, name: p.name.clone(), auto: p.auto }).collect(),
            profile_id: selected.map(|p| p.id),
        }),
    })
}

/// Órdenes del Cockpit propio. Los textos de vuelta son códigos que el front
/// traduce (`cockpit.std.*`).
pub async fn action(pool: &SqlitePool, req: TlCockpitRequest) -> Result<TlActionResult> {
    let reg = normalize_reg(&req.registration);
    let mut st = state(pool, &reg).await?;
    let all = profiles(pool, &reg).await?;
    let live = live_for(&reg).map(|(_, v)| v);
    let need_live = || live.clone().ok_or_else(|| anyhow!("not_in_sim"));
    let ok = |m: String| Ok(TlActionResult { ok: true, message: m, queued: false });
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(40).collect::<String>());
    match req.action.as_str() {
        "read" => ok(String::new()),
        "save" => {
            let values: HashMap<&str, f64> = SWITCHES.iter().map(|s| s.id).zip(need_live()?).collect();
            let json = serde_json::to_string(&values)?;
            let now = Utc::now().to_rfc3339();
            let airport = airport_of(pool, &reg).await;
            let id = match (req.profile, name) {
                (Some(p), _) => {
                    sqlx::query("UPDATE cockpit_std_profile SET switches = ?2, saved_at = ?3, saved_airport = ?4 WHERE id = ?1 AND registration = ?5")
                        .bind(p)
                        .bind(&json)
                        .bind(&now)
                        .bind(&airport)
                        .bind(&reg)
                        .execute(pool)
                        .await?;
                    p
                }
                (None, Some(n)) => sqlx::query(
                    "INSERT INTO cockpit_std_profile (registration, name, switches, saved_at, saved_airport) VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .bind(&reg)
                .bind(&n)
                .bind(&json)
                .bind(&now)
                .bind(&airport)
                .execute(pool)
                .await?
                .last_insert_rowid(),
                _ => return Err(anyhow!("falta el nombre del perfil")),
            };
            st.profile_id = Some(id);
            save_state(pool, &reg, &st).await?;
            ok("saved".into())
        }
        "restore" => {
            let live = need_live()?;
            let p = all
                .iter()
                .find(|p| Some(p.id) == req.profile.or(st.profile_id))
                .or(all.first())
                .ok_or_else(|| anyhow!("no_profile"))?;
            let cmds = restore_commands(p, &st, Some(&live), req.id.as_deref().filter(|i| !i.is_empty()));
            let n = cmds.len();
            queue().lock().unwrap_or_else(|e| e.into_inner()).extend(cmds);
            tracing::info!(target: "cockpit", "{reg}: restaurar «{}» → {n} interruptores", p.name);
            ok(format!("moved:{n}"))
        }
        "set" => {
            let live = need_live()?;
            let id = req.id.clone().unwrap_or_default();
            let i = index_of(&id).ok_or_else(|| anyhow!("mando desconocido: {id}"))?;
            let value = req.value.ok_or_else(|| anyhow!("falta el valor"))?;
            if let Some(cmd) = command(&SWITCHES[i], live.get(i).copied(), value) {
                queue().lock().unwrap_or_else(|e| e.into_inner()).push(cmd);
            }
            ok(String::new())
        }
        "keep" => {
            let id = req.id.clone().unwrap_or_default();
            if index_of(&id).is_none() {
                return Err(anyhow!("mando desconocido: {id}"));
            }
            if req.keep.unwrap_or(true) {
                st.skip.remove(&id);
            } else {
                st.skip.insert(id);
            }
            save_state(pool, &reg, &st).await?;
            ok(String::new())
        }
        "auto" => {
            st.restore_on_load = req.on.unwrap_or(false);
            save_state(pool, &reg, &st).await?;
            ok(String::new())
        }
        "select" | "profileAuto" | "rename" | "delete" => {
            let p = req.profile.ok_or_else(|| anyhow!("falta el perfil"))?;
            if !all.iter().any(|x| x.id == p) {
                return Err(anyhow!("no_profile"));
            }
            match req.action.as_str() {
                "select" => {
                    st.profile_id = Some(p);
                    save_state(pool, &reg, &st).await?;
                }
                "profileAuto" => {
                    // Sólo uno «al cargar» por avión.
                    let on = !all.iter().any(|x| x.id == p && x.auto);
                    sqlx::query("UPDATE cockpit_std_profile SET auto = 0 WHERE registration = ?1")
                        .bind(&reg)
                        .execute(pool)
                        .await?;
                    if on {
                        sqlx::query("UPDATE cockpit_std_profile SET auto = 1 WHERE id = ?1").bind(p).execute(pool).await?;
                    }
                }
                "rename" => {
                    let n = name.ok_or_else(|| anyhow!("falta el nombre del perfil"))?;
                    sqlx::query("UPDATE cockpit_std_profile SET name = ?2 WHERE id = ?1").bind(p).bind(n).execute(pool).await?;
                }
                _ => {
                    sqlx::query("DELETE FROM cockpit_std_profile WHERE id = ?1").bind(p).execute(pool).await?;
                    if st.profile_id == Some(p) {
                        st.profile_id = None;
                        save_state(pool, &reg, &st).await?;
                    }
                }
            }
            ok(String::new())
        }
        other => Err(anyhow!("acción desconocida: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transponder_codes_round_trip_as_bcd() {
        assert_eq!(dec_to_bcd(1200.0), 0x1200);
        assert_eq!(dec_to_bcd(7700.0), 0x7700);
        assert_eq!(bcd_to_dec(0x2000 as f64), 2000.0);
        assert_eq!(bcd_to_dec(dec_to_bcd(4521.0) as f64), 4521.0);
    }

    #[test]
    fn only_differing_switches_are_moved() {
        let s = |id: &str| *SWITCHES.iter().find(|s| s.id == id).unwrap();
        assert_eq!(command(&s("LIGHT_NAV"), Some(1.0), 1.0), None);
        assert_eq!(command(&s("LIGHT_NAV"), Some(0.0), 1.0), Some(("NAV_LIGHTS_SET", 1)));
        assert_eq!(command(&s("SEATBELTS"), Some(0.0), 1.0), Some(("CABIN_SEATBELTS_ALERT_SWITCH_TOGGLE", 0)));
        assert_eq!(command(&s("SEATBELTS"), None, 1.0), None, "sin lectura no se alterna a ciegas");
        assert_eq!(command(&s("APU"), Some(1.0), 0.0), Some(("APU_OFF_SWITCH", 0)));
        assert_eq!(command(&s("XPDR_CODE"), Some(2000.0), 1200.0), Some(("XPNDR_SET", 0x1200)));
        let p = Profile {
            id: 1,
            name: "Gate".into(),
            auto: true,
            values: [("LIGHT_NAV".to_string(), 1.0), ("BATTERY".to_string(), 1.0)].into_iter().collect(),
            saved_at: String::new(),
            airport: None,
        };
        let live = vec![0.0; SWITCHES.len()];
        let st = State { profile_id: Some(1), restore_on_load: false, skip: ["BATTERY".to_string()].into_iter().collect() };
        assert_eq!(restore_commands(&p, &st, Some(&live), None), vec![("NAV_LIGHTS_SET", 1)]);
        assert!(restore_commands(&p, &st, Some(&live), Some("elec")).is_empty());
    }

    #[test]
    fn the_snapshot_reads_every_switch_and_the_cold_and_dark_inputs() {
        assert_eq!(sim_vars().len(), SWITCHES.len() + DETECT.len());
        let mut raw = vec![0.0; SWITCHES.len() + DETECT.len()];
        raw[index_of("XPDR_CODE").unwrap()] = 0x1200 as f64;
        on_snapshot(&raw);
        let l = live().lock().unwrap();
        assert_eq!(l.as_ref().unwrap().values[index_of("XPDR_CODE").unwrap()], 1200.0);
    }
}
