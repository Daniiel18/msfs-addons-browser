//! (19/10) Etapa 3A — reglas y persistencia del Tech Log propio de SimFleet.
//!
//! Mismo modelo que Fenix/PMDG TechLog (ver ANALISIS-TECHLOG.md):
//!   · cada pieza tiene su vida, fijada al instalarla con un factor
//!     determinista 0.7–1.3 por matrícula (SHA-256, como TechLog);
//!   · % restante = 1 − uso desde la instalación ÷ (vida ÷ ritmo de desgaste);
//!   · ≤ 15 % → degradada (defecto), ≤ 0 % → fallada, ≤ 3 % en su momento de
//!     estrés (arranque, despegue, tren, flaps, toque, reversas) → falla ahí;
//!   · los abusos (excedencias, aterrizajes duros) consumen vida;
//!   · defectos con su ítem MEL (NO-GO / A–D), diferibles en su ventana;
//!   · checks DAILY / WEEKLY / A / C; vencido → NO-GO.
//! Lo que se hace en el Hangar (reparar, reemplazar, servicio, check) se
//! cobra en Finanzas (`maintenance_log`, componente `mx:<tipo>:<detalle>`).

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration as CDur, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use super::catalog::{catalog, detect_profile, Catalog, CompDef, Family, Profile};
use super::recorder::{Effect, FlightEvent, FlightRecord, LandingRec, Target};
use crate::airline_economy::normalize_reg;
use crate::techlog::{
    TechLogAircraft, TechLogFlight, TlActionRequest, TlActionResult, TlCheck, TlCondition,
    TlConsumable, TlDefect, TlDispatch, TlFlightEvent, TlFlightWear, TlPart, TlSource,
};

/// Ventana de diferido por categoría MEL (días).
fn defer_days(category: &str) -> Option<i64> {
    match category {
        "A" => Some(3),
        "B" => Some(3),
        "C" => Some(10),
        "D" => Some(120),
        _ => None,
    }
}

fn mel_category(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "nogo" | "no-go" => "NoGo".into(),
        "a" => "A".into(),
        "b" => "B".into(),
        "c" => "C".into(),
        "d" => "D".into(),
        other => other.to_uppercase(),
    }
}

const DEGRADED_AT_PCT: f64 = 15.0;
const STRESS_MARGIN_PCT: f64 = 3.0;

// ---------------------------------------------------------------------------
// Modelo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Counters {
    pub flight_hours: f64,
    pub block_hours: f64,
    pub cycles: f64,
    pub landings: f64,
    pub pax_cycles: f64,
    pub eng_hours: [f64; 2],
    pub eng_cycles: [f64; 2],
    pub eng_starts: [f64; 2],
    pub apu_hours: f64,
    pub apu_starts: f64,
    pub rev_deploys: [f64; 2],
    pub wear_extra: HashMap<String, f64>,
}

fn day_number(now: DateTime<Utc>) -> f64 {
    let epoch = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now);
    (now - epoch).num_seconds() as f64 / 86_400.0
}

impl Counters {
    /// Igual que `Counters.WearValue` de TechLog.
    pub fn wear_value(&self, basis: &str, engine: usize, now: DateTime<Utc>) -> f64 {
        let e = engine.clamp(1, 2) - 1;
        let (v, name) = match basis {
            "flightHours" => (self.flight_hours, "FlightHours"),
            "blockHours" => (self.block_hours, "BlockHours"),
            "cycles" => (self.cycles, "Cycles"),
            "landings" => (self.landings, "Landings"),
            "paxCycles" => (self.pax_cycles, "PaxCycles"),
            "engHours" => (self.eng_hours[e], "EngHours"),
            "engCycles" => (self.eng_cycles[e], "EngCycles"),
            "engStarts" => (self.eng_starts[e], "EngStarts"),
            "apuHours" => (self.apu_hours, "ApuHours"),
            "apuStarts" => (self.apu_starts, "ApuStarts"),
            "revDeploys" => (self.rev_deploys[e], "RevDeploys"),
            "days" => return day_number(now),
            _ => (self.flight_hours, "FlightHours"),
        };
        v + self
            .wear_extra
            .get(&format!("{name}:{}", e + 1))
            .copied()
            .unwrap_or(0.0)
    }
}

/// Factor 0..1 determinista, igual que `LifeFactor.Unit` de TechLog.
fn unit(reg: &str, key: &str, pos: &str, serial: i64, salt: &str) -> f64 {
    let s = format!("{}|{key}|{pos}|{serial}|{salt}", reg.to_uppercase());
    let h = Sha256::digest(s.as_bytes());
    u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as f64 / 4_294_967_296.0
}

fn life_factor(reg: &str, key: &str, pos: &str, serial: i64) -> f64 {
    0.7 + 0.6 * unit(reg, key, pos, serial, "life")
}

fn engine_index(pos: &str) -> usize {
    pos.chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .map(|d| (d as usize).clamp(1, 2))
        .unwrap_or(1)
}

#[derive(Debug, Clone)]
struct Airframe {
    reg: String,
    family: Family,
    variant: String,
    engine: String,
    counters: Counters,
    consumables: HashMap<String, f64>,
    egt: Vec<f64>,
}

impl Airframe {
    fn cat(&self) -> &'static Catalog {
        catalog(self.family)
    }
}

#[derive(Debug, Clone)]
struct Comp {
    id: i64,
    key: String,
    pos: String,
    serial: i64,
    basis_at_install: f64,
    life: f64,
    wear: f64,
    status: String,
    defect_id: Option<i64>,
    repairs: i64,
}

struct Rates {
    life: f64,
    cabin: f64,
}

async fn rates(pool: &SqlitePool) -> Rates {
    let get = |k: &'static str| async move {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
            .bind(k)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<f64>().ok())
    };
    Rates {
        life: get("mx_wear_rate").await.unwrap_or(3.0).max(0.05),
        cabin: get("mx_cabin_wear_rate").await.unwrap_or(2.0).max(0.05),
    }
}

fn remaining(c: &Comp, def: &CompDef, a: &Airframe, r: &Rates, now: DateTime<Utc>) -> f64 {
    if def.kind() == "wear" {
        return (100.0 - c.wear).clamp(0.0, 100.0);
    }
    let rate = if def.kind() == "cabin" { r.cabin } else { r.life };
    let span = c.life / rate;
    if span <= 0.0 {
        return 0.0;
    }
    let used = a.counters.wear_value(def.basis(), engine_index(&c.pos), now) - c.basis_at_install;
    (100.0 * (1.0 - used / span)).clamp(0.0, 100.0)
}

/// Fija el % restante de una pieza (reparación) moviendo su base.
fn set_remaining(c: &mut Comp, def: &CompDef, a: &Airframe, r: &Rates, pct: f64, now: DateTime<Utc>) {
    if def.kind() == "wear" {
        c.wear = 100.0 - pct;
        return;
    }
    let rate = if def.kind() == "cabin" { r.cabin } else { r.life };
    let span = c.life / rate;
    let cur = a.counters.wear_value(def.basis(), engine_index(&c.pos), now);
    c.basis_at_install = cur - (1.0 - pct / 100.0) * span;
}

/// Consume una fracción de vida (abuso).
fn consume(c: &mut Comp, def: &CompDef, r: &Rates, share: f64) {
    if def.kind() == "wear" {
        c.wear = (c.wear + share * 100.0).min(100.0);
    } else {
        let rate = if def.kind() == "cabin" { r.cabin } else { r.life };
        c.basis_at_install -= share * c.life / rate;
    }
}

// ---------------------------------------------------------------------------
// Acceso a datos
// ---------------------------------------------------------------------------

async fn load_airframe(pool: &SqlitePool, reg: &str) -> Result<Option<Airframe>> {
    let row = sqlx::query("SELECT * FROM mx_airframe WHERE registration = ?1")
        .bind(reg)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| Airframe {
        reg: reg.to_string(),
        family: Family::parse(&r.try_get::<String, _>("family").unwrap_or_default()),
        variant: r.try_get("variant").unwrap_or_default(),
        engine: r.try_get("engine").unwrap_or_default(),
        counters: r
            .try_get::<String, _>("counters")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        consumables: r
            .try_get::<String, _>("consumables")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        egt: r
            .try_get::<String, _>("egt_margin")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    }))
}

async fn save_airframe(pool: &SqlitePool, a: &Airframe) -> Result<()> {
    sqlx::query(
        "UPDATE mx_airframe SET counters = ?2, consumables = ?3, egt_margin = ?4, last_seen_at = ?5 WHERE registration = ?1",
    )
    .bind(&a.reg)
    .bind(serde_json::to_string(&a.counters)?)
    .bind(serde_json::to_string(&a.consumables)?)
    .bind(serde_json::to_string(&a.egt)?)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Avión nuevo ("todos nuevos"): piezas con vida propia, consumibles llenos
/// y checks recién hechos.
async fn create_airframe(pool: &SqlitePool, reg: &str, p: &Profile, title: Option<&str>) -> Result<Airframe> {
    let now = Utc::now();
    let cat = catalog(p.family);
    let eng = cat.engine(&p.engine);
    let mut consumables = HashMap::new();
    for c in &cat.consumables {
        for pos in c.positions_or_single() {
            let key = if pos.is_empty() { c.key.clone() } else { format!("{}:{pos}", c.key) };
            consumables.insert(key, if c.fills { 0.0 } else { c.capacity(&p.engine) });
        }
    }
    let margin = if eng.egt_margin_new_c > 0.0 { eng.egt_margin_new_c } else { 80.0 };
    let a = Airframe {
        reg: reg.to_string(),
        family: p.family,
        variant: p.variant.clone(),
        engine: p.engine.clone(),
        counters: Counters::default(),
        consumables,
        egt: vec![margin, margin],
    };
    sqlx::query(
        "INSERT OR IGNORE INTO mx_airframe (registration, family, variant, engine, title, created_at, counters, consumables, egt_margin)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )
    .bind(reg)
    .bind(p.family.id())
    .bind(&p.variant)
    .bind(&p.engine)
    .bind(title)
    .bind(now.to_rfc3339())
    .bind(serde_json::to_string(&a.counters)?)
    .bind(serde_json::to_string(&a.consumables)?)
    .bind(serde_json::to_string(&a.egt)?)
    .execute(pool)
    .await?;
    for def in cat.components.iter().filter(|d| d.applies_to(&p.variant)) {
        for pos in def.positions_or_single() {
            let life = (def.life * life_factor(reg, &def.key, &pos, 0) * 10.0).round() / 10.0;
            let basis = a.counters.wear_value(def.basis(), engine_index(&pos), now);
            sqlx::query(
                "INSERT OR IGNORE INTO mx_component (registration, key, position, serial, installed_at, basis_at_install, life)
                 VALUES (?1,?2,?3,0,?4,?5,?6)",
            )
            .bind(reg)
            .bind(&def.key)
            .bind(&pos)
            .bind(now.to_rfc3339())
            .bind(basis)
            .bind(life)
            .execute(pool)
            .await?;
        }
    }
    for chk in &cat.limits.checks {
        sqlx::query(
            "INSERT OR IGNORE INTO mx_check (registration, key, last_done_at, last_done_fh, last_done_fc) VALUES (?1,?2,?3,0,0)",
        )
        .bind(reg)
        .bind(&chk.key)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
    }
    tracing::info!(
        target: "mx",
        "avión nuevo en el Tech Log de SimFleet: {reg} ({} {} {})",
        p.family.id(),
        p.variant,
        p.engine
    );
    Ok(a)
}

async fn load_components(pool: &SqlitePool, reg: &str) -> Result<Vec<Comp>> {
    let rows = sqlx::query("SELECT * FROM mx_component WHERE registration = ?1 ORDER BY id")
        .bind(reg)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| Comp {
            id: r.try_get("id").unwrap_or(0),
            key: r.try_get("key").unwrap_or_default(),
            pos: r.try_get("position").unwrap_or_default(),
            serial: r.try_get("serial").unwrap_or(0),
            basis_at_install: r.try_get("basis_at_install").unwrap_or(0.0),
            life: r.try_get("life").unwrap_or(0.0),
            wear: r.try_get("wear").unwrap_or(0.0),
            status: r.try_get("status").unwrap_or_else(|_| "Ok".into()),
            defect_id: r.try_get("defect_id").ok().flatten(),
            repairs: r.try_get("repairs").unwrap_or(0),
        })
        .collect())
}

async fn save_component(pool: &SqlitePool, c: &Comp) -> Result<()> {
    sqlx::query(
        "UPDATE mx_component SET serial = ?2, installed_at = COALESCE(?3, installed_at), basis_at_install = ?4, life = ?5, wear = ?6, status = ?7, defect_id = ?8, repairs = ?9 WHERE id = ?1",
    )
    .bind(c.id)
    .bind(c.serial)
    .bind(None::<String>)
    .bind(c.basis_at_install)
    .bind(c.life)
    .bind(c.wear)
    .bind(&c.status)
    .bind(c.defect_id)
    .bind(c.repairs)
    .execute(pool)
    .await?;
    Ok(())
}

/// Matrícula → perfil (por el último vuelo con esa matrícula o una pista).
async fn profile_for(pool: &SqlitePool, reg: &str, hint: Option<&str>) -> (Profile, Option<String>) {
    let rows = sqlx::query(
        "SELECT aircraft_registration, aircraft_title, aircraft_atc_type FROM flight_log
         WHERE aircraft_registration IS NOT NULL ORDER BY started_at DESC LIMIT 500",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for r in rows {
        let rr: Option<String> = r.try_get("aircraft_registration").ok().flatten();
        if rr.as_deref().map(normalize_reg).as_deref() == Some(reg) {
            let title: Option<String> = r.try_get("aircraft_title").ok().flatten();
            let atc: Option<String> = r.try_get("aircraft_atc_type").ok().flatten();
            return (detect_profile(&[title.as_deref(), atc.as_deref(), hint]), title);
        }
    }
    (detect_profile(&[hint]), hint.map(String::from))
}

async fn ensure_airframe(pool: &SqlitePool, reg: &str, profile: Option<Profile>, hint: Option<&str>) -> Result<Airframe> {
    if let Some(a) = load_airframe(pool, reg).await? {
        return Ok(a);
    }
    let (detected, title) = profile_for(pool, reg, hint).await;
    let p = profile.unwrap_or(detected);
    create_airframe(pool, reg, &p, title.as_deref()).await
}

/// Abre un defecto (salvo que ya haya uno abierto/diferido igual). Devuelve su id.
#[allow(clippy::too_many_arguments)]
async fn raise(
    pool: &SqlitePool,
    cat: &Catalog,
    reg: &str,
    mel: &str,
    pos: Option<&str>,
    source: &str,
    component_key: Option<&str>,
    detail: &str,
    flight_id: Option<i64>,
) -> Result<Option<i64>> {
    let pos_s = pos.unwrap_or("");
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM mx_defect WHERE registration = ?1 AND mel_id = ?2 AND IFNULL(position,'') = ?3 AND status IN ('Open','Deferred')",
    )
    .bind(reg)
    .bind(mel)
    .bind(pos_s)
    .fetch_optional(pool)
    .await?;
    if existing.is_some() {
        return Ok(None);
    }
    let (title, ata, category) = match cat.mel_item(mel) {
        Some(m) => (
            m.title.replace("{pos}", pos_s).trim().to_string(),
            m.ata,
            mel_category(&m.category),
        ),
        None => (detail.to_string(), 0, "C".to_string()),
    };
    let id = sqlx::query(
        "INSERT INTO mx_defect (registration, ata, title, detail, source, component_key, position, mel_id, category, status, raised_at, raised_flight_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'Open',?10,?11)",
    )
    .bind(reg)
    .bind(ata)
    .bind(&title)
    .bind(detail)
    .bind(source)
    .bind(component_key)
    .bind(pos)
    .bind(mel)
    .bind(&category)
    .bind(Utc::now().to_rfc3339())
    .bind(flight_id)
    .execute(pool)
    .await?
    .last_insert_rowid();
    tracing::info!(target: "mx", "{reg}: defecto abierto {mel} ({category}) — {title}");
    Ok(Some(id))
}

async fn rectify(pool: &SqlitePool, id: i64, how: &str) -> Result<()> {
    sqlx::query("UPDATE mx_defect SET status = 'Rectified', rectified_at = ?2, rectified_how = ?3 WHERE id = ?1")
        .bind(id)
        .bind(Utc::now().to_rfc3339())
        .bind(how)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cobro en Finanzas al nivel de mantenimiento de la aerolínea.
async fn charge(pool: &SqlitePool, reg: &str, kind: &str, detail: &str, base: f64) -> Result<f64> {
    if base <= 0.0 {
        return Ok(0.0);
    }
    let mult = crate::aircraft_maintenance::level_mult_for_registration(pool, reg).await;
    let cost = (base * mult).round();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES (?1,?2,?3,?4)")
        .bind(reg)
        .bind(format!("mx:{kind}:{detail}"))
        .bind(cost)
        .bind(&now)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO mx_servicing (registration, at, kind, detail, cost) VALUES (?1,?2,?3,?4,?5)")
        .bind(reg)
        .bind(&now)
        .bind(kind)
        .bind(detail)
        .bind(cost)
        .execute(pool)
        .await?;
    Ok(cost)
}

// ---------------------------------------------------------------------------
// Cierre de vuelo
// ---------------------------------------------------------------------------

struct WearLog(Vec<TlFlightWear>);
impl WearLog {
    fn add(&mut self, item: String, amount: f64, unit: &str) {
        if amount.abs() < 0.0001 {
            return;
        }
        if let Some(w) = self.0.iter_mut().find(|w| w.item == item && w.unit == unit) {
            w.amount += amount;
        } else {
            self.0.push(TlFlightWear { item, amount, unit: unit.to_string() });
        }
    }
}

/// Daños de un aterrizaje (umbrales de TechLog / DamageAssessment).
fn landing_effects(
    l: &LandingRec,
    cat: &Catalog,
    variant: &str,
    events: &mut Vec<FlightEvent>,
) -> Vec<Effect> {
    let lim = cat.variant(variant);
    let c = &cat.limits.common;
    let over = lim.mlw_kg > 0.0 && l.weight_kg > lim.mlw_kg + 100.0;
    let (hard, hard_g) = if over { (360.0, 1.7) } else { (c.hard_landing_fpm, c.hard_landing_g) };
    let (sev, sev_g) = if over { (600.0, 2.6) } else { (c.severe_hard_landing_fpm, c.severe_hard_landing_g) };
    let (ext, ext_g) = if over { (1000.0, 3.0) } else { (c.extreme_landing_fpm, c.extreme_landing_g) };
    let text = format!(
        "{:.0} fpm, {:.2} g{}",
        -l.sink_fpm,
        l.peak_g,
        if over { format!(", {:.0} kg (MLW {:.0})", l.weight_kg, lim.mlw_kg) } else { String::new() }
    );
    let mut fx = Vec::new();
    let mut ev = |kind: &str, sev: &str, t: String| {
        events.push(FlightEvent { phase: "LANDING".into(), kind: kind.into(), severity: sev.into(), text: t });
    };
    let raise = |fx: &mut Vec<Effect>, mel: &'static str, d: &str| fx.push(Effect::Raise { mel, pos: None, detail: d.to_string() });
    if l.sink_fpm >= ext || l.peak_g >= ext_g {
        let t = format!("Landing beyond the gear's strength: {text}. Gear structure damaged, tyres burst.");
        raise(&mut fx, "32-10-01", &t);
        raise(&mut fx, "05-51-02", &t);
        raise(&mut fx, "32-11-01", &t);
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 1.0, why: "extreme landing".into() });
        fx.push(Effect::Tyres { main: true, pct: 100.0, why: "burst on landing".into() });
        ev("LANDING", "Warning", t);
    } else if l.sink_fpm >= sev || l.peak_g >= sev_g {
        let t = format!("Severe hard landing: {text}");
        raise(&mut fx, "05-51-02", &t);
        raise(&mut fx, "32-11-01", &t);
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.8, why: "severe hard landing".into() });
        fx.push(Effect::Tyres { main: true, pct: 40.0, why: "severe hard landing".into() });
        ev("LANDING", "Warning", t);
    } else if l.sink_fpm >= hard || l.peak_g >= hard_g {
        let t = format!("Hard landing: {text}. Phase 1 inspection.");
        raise(&mut fx, "05-51-01", &t);
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.25, why: "hard landing".into() });
        fx.push(Effect::Tyres { main: true, pct: 12.0, why: "hard landing".into() });
        ev("LANDING", "Warning", t);
    } else if l.sink_fpm >= 400.0 {
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.03, why: "firm landing".into() });
        ev("LANDING", "Caution", format!("Firm landing: {text}"));
    } else {
        ev("LANDING", "Info", format!("Landing: {text}"));
    }
    if over {
        let t = format!("Landed above MLW: {:.0} kg (MLW {:.0})", l.weight_kg, lim.mlw_kg);
        raise(&mut fx, "05-51-07", &t);
        ev("OVERWEIGHT", "Warning", t);
    }
    let crab = l.crab_deg.abs();
    if l.gs_kt > 60.0 && crab >= 10.0 {
        let t = format!("Touchdown with {crab:.0}° of crab: side-load inspection");
        raise(&mut fx, "05-51-12", &t);
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.25, why: "side load".into() });
        ev("SIDE_LOAD", "Warning", t);
    } else if l.gs_kt > 60.0 && crab >= 5.0 {
        fx.push(Effect::Tyres { main: true, pct: 6.0, why: "scrubbed sideways".into() });
        ev("SIDE_SCRUB", "Caution", format!("Touchdown with {crab:.0}° of crab: tyres scrubbed"));
    }
    if lim.tailstrike_pitch > 0.0 && l.pitch_up_deg >= lim.tailstrike_pitch {
        let t = format!("Tail strike on landing: pitch {:.1}° (limit {:.1}°)", l.pitch_up_deg, lim.tailstrike_pitch);
        raise(&mut fx, "05-51-06", &t);
        ev("TAILSTRIKE", "Warning", t);
    }
    if l.bounces > 0 {
        fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.03 * l.bounces as f64, why: "bounced landing".into() });
        ev("BOUNCE", "Caution", format!("Bounced landing ({})", l.bounces));
    }
    if l.derot_deg_s >= 6.0 {
        let t = format!("Nose gear slammed down at {:.1}°/s", l.derot_deg_s);
        raise(&mut fx, "05-51-13", &t);
        fx.push(Effect::Tyres { main: false, pct: 35.0, why: "nose slam".into() });
        ev("NOSE_SLAM", "Warning", t);
    } else if l.derot_deg_s >= 4.0 {
        fx.push(Effect::Tyres { main: false, pct: 5.0, why: "firm nose touchdown".into() });
    }
    fx
}

fn is_gear_part(def: &CompDef) -> bool {
    def.key.starts_with("GEAR_") || def.key == "LGCIU"
}

fn targets<'a>(t: &Target, pos: &Option<String>, comps: &'a mut [Comp], cat: &Catalog) -> Vec<&'a mut Comp> {
    comps
        .iter_mut()
        .filter(|c| {
            let Some(def) = cat.component(&c.key) else { return false };
            let key_ok = match t {
                Target::Key(k) => c.key == *k,
                Target::Gear => is_gear_part(def),
                Target::HotSection => matches!(c.key.as_str(), "ENG_HPT" | "ENG_NGV" | "ENG_COMBUSTOR"),
            };
            key_ok && pos.as_ref().is_none_or(|p| &c.pos == p)
        })
        .collect()
}

/// Aplica un vuelo cerrado al Tech Log. `landing_fpm` = FPM final del
/// watcher (más preciso que el del registrador) para el último aterrizaje.
pub async fn commit_flight(pool: &SqlitePool, mut rec: FlightRecord, landing_fpm: Option<i64>) -> Result<()> {
    let row = sqlx::query("SELECT aircraft_registration, aircraft_title, aircraft_atc_type FROM flight_log WHERE id = ?1")
        .bind(rec.flight_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(()) };
    let raw_reg: Option<String> = row.try_get("aircraft_registration").ok().flatten();
    let reg = raw_reg.as_deref().map(normalize_reg).unwrap_or_default();
    if reg.is_empty() {
        tracing::info!(target: "mx", "vuelo {} sin matrícula: no se registra en el Tech Log", rec.flight_id);
        return Ok(());
    }
    if crate::techlog::managed_registrations().await.contains_key(&reg) {
        tracing::info!(target: "mx", "{reg}: lo gestiona Fenix/PMDG TechLog (fuente de verdad)");
        return Ok(());
    }
    let title: Option<String> = row.try_get("aircraft_title").ok().flatten();
    let atc: Option<String> = row.try_get("aircraft_atc_type").ok().flatten();
    let profile = rec
        .profile
        .clone()
        .unwrap_or_else(|| detect_profile(&[title.as_deref(), atc.as_deref()]));
    let mut a = ensure_airframe(pool, &reg, Some(profile), title.as_deref()).await?;
    let cat = a.cat();
    let r = rates(pool).await;
    let now = Utc::now();
    let mut comps = load_components(pool, &reg).await?;
    let mut wear = WearLog(Vec::new());
    let mut events = std::mem::take(&mut rec.events);

    if let (Some(fpm), Some(last)) = (landing_fpm, rec.landings.last_mut()) {
        if fpm < 0 {
            last.sink_fpm = (-fpm) as f64;
        }
    }

    // 1) Contadores.
    let seats = cat.seats(&a.variant);
    let pax = (seats * 0.8).round();
    {
        let c = &mut a.counters;
        c.block_hours += rec.block_h;
        c.flight_hours += rec.air_h;
        c.cycles += rec.takeoffs as f64;
        c.landings += rec.landings.len() as f64;
        c.pax_cycles += pax * rec.landings.len() as f64;
        for i in 0..2 {
            c.eng_hours[i] += rec.eng_h[i];
            c.eng_starts[i] += rec.eng_starts[i] as f64;
            if rec.eng_ran[i] && rec.takeoffs > 0 {
                c.eng_cycles[i] += 1.0;
            }
            c.rev_deploys[i] += rec.rev_deploys[i] as f64;
            if rec.workload_extra_h[i] > 0.0 {
                *c.wear_extra.entry(format!("EngHours:{}", i + 1)).or_default() += rec.workload_extra_h[i];
                wear.add(format!("Engine {} workload", i + 1), rec.workload_extra_h[i], "h");
            }
        }
        c.apu_hours += rec.apu_h;
        c.apu_starts += rec.apu_starts as f64;
    }

    // 2) Margen EGT por despegue (TOGA gasta más que FLEX).
    let flex = rec.takeoffs.saturating_sub(rec.toga_takeoffs) as f64;
    for (i, m) in a.egt.iter_mut().enumerate() {
        if rec.eng_ran.get(i).copied().unwrap_or(false) {
            *m = (*m - 0.2 * rec.toga_takeoffs as f64 - 0.04 * flex).max(0.0);
        }
    }

    // 3) Efectos: excedencias + aterrizajes.
    let mut effects = std::mem::take(&mut rec.effects);
    for l in &rec.landings {
        effects.extend(landing_effects(l, cat, &a.variant, &mut events));
    }
    let mut raises: Vec<(&'static str, Option<String>, String)> = Vec::new();
    for e in effects {
        match e {
            Effect::Raise { mel, pos, detail } => raises.push((mel, pos, detail)),
            Effect::Abuse { target, pos, share, why } => {
                for c in targets(&target, &pos, &mut comps, cat) {
                    if let Some(def) = cat.component(&c.key) {
                        consume(c, def, &r, share);
                        wear.add(def.name_for(&c.pos), share * 100.0, "% of life");
                        let _ = why.as_str();
                    }
                }
            }
            Effect::Tyres { main, pct, why: _ } => {
                let key = if main { "TYRE_MAIN" } else { "TYRE_NOSE" };
                for c in comps.iter_mut().filter(|c| c.key == key) {
                    c.wear = (c.wear + pct).min(100.0);
                    if let Some(def) = cat.component(key) {
                        wear.add(def.name_for(&c.pos), pct, "% worn");
                    }
                }
            }
        }
    }

    // 4) Desgaste normal de neumáticos y frenos (WearRules de TechLog).
    for l in &rec.landings {
        let sev = l.brake_energy_mj / 25.0;
        let wet = if l.wet { 1.15 } else { 1.0 };
        for c in comps.iter_mut() {
            let Some(def) = cat.component(&c.key) else { continue };
            let pct = match c.key.as_str() {
                "TYRE_MAIN" | "TYRE_NOSE" => {
                    let nose = c.key == "TYRE_NOSE";
                    let base = if nose { 250.0 } else { 300.0 };
                    let k = 1.0
                        + (l.gs_kt - 135.0).max(0.0) / 40.0
                        + (l.sink_fpm - 300.0).max(0.0) / 600.0
                        + if nose { 0.0 } else { (sev - 1.0).max(0.0) * 0.3 };
                    100.0 / base * k * r.life * wet
                }
                "BRAKE" => 0.05 * (0.4 + 0.6 * sev.clamp(0.0, 4.0)) * r.life * wet,
                _ => continue,
            };
            c.wear = (c.wear + pct).min(100.0);
            wear.add(def.name_for(&c.pos), pct, "% worn");
        }
    }
    if rec.taxi_m > 0.0 {
        let pct = rec.taxi_m / 1000.0 * 0.015 * r.life;
        for c in comps.iter_mut().filter(|c| c.key == "TYRE_MAIN" || c.key == "TYRE_NOSE") {
            c.wear = (c.wear + pct).min(100.0);
        }
    }

    // 5) Consumibles.
    for cdef in &cat.consumables {
        for pos in cdef.positions_or_single() {
            let basis = match cdef.usage_basis.as_deref() {
                Some("engHours") => rec.eng_h[engine_index(&pos) - 1],
                Some("apuHours") => rec.apu_h,
                Some("blockHours") => rec.block_h,
                Some("flightHours") => rec.air_h,
                _ => 0.0,
            };
            let used = cdef.use_per_unit * basis + cdef.use_per_pax_hour * pax * rec.block_h;
            let key = if pos.is_empty() { cdef.key.clone() } else { format!("{}:{pos}", cdef.key) };
            let cap = cdef.capacity(&a.engine);
            let level = a.consumables.get(&key).copied().unwrap_or(if cdef.fills { 0.0 } else { cap });
            if cdef.key == "ENG_OIL" {
                if let Some(crit) = cdef.critical {
                    let i = engine_index(&pos) - 1;
                    if level < crit && rec.eng_h[i] > 0.05 {
                        raises.push(("72-00-06", Some(pos.clone()), format!("Engine {pos} run with {level:.1} qt of oil: bearings failed")));
                    }
                }
            }
            if used <= 0.0 {
                continue;
            }
            let new = if cdef.fills { (level + used).min(cap) } else { (level - used).max(0.0) };
            a.consumables.insert(key, (new * 10.0).round() / 10.0);
            wear.add(cdef.name_for(&pos), if cdef.fills { used } else { -used }, &cdef.unit);
        }
    }

    // 6) Fallos bajo carga (≤ 3 %) en los momentos de estrés del vuelo.
    let mut failures: Vec<(usize, String)> = Vec::new();
    for (moment, engine) in &rec.stress {
        for (idx, c) in comps.iter().enumerate() {
            if c.status == "Failed" {
                continue;
            }
            let Some(def) = cat.component(&c.key) else { continue };
            if def.stress.as_deref() != Some(moment.as_str()) {
                continue;
            }
            if let Some(e) = engine {
                if def.basis().starts_with("eng") && engine_index(&c.pos) != *e {
                    continue;
                }
            }
            if remaining(c, def, &a, &r, now) <= STRESS_MARGIN_PCT && !failures.iter().any(|(i, _)| *i == idx) {
                failures.push((idx, format!("gave way at {moment}")));
            }
        }
    }

    // 7) Vida: fallo (≤ 0 %) y degradación (≤ 15 %).
    for (idx, c) in comps.iter().enumerate() {
        if c.status == "Failed" || failures.iter().any(|(i, _)| *i == idx) {
            continue;
        }
        let Some(def) = cat.component(&c.key) else { continue };
        let rem = remaining(c, def, &a, &r, now);
        if rem <= 0.0 {
            failures.push((idx, if def.kind() == "wear" { "worn to limits".into() } else { "end of its life".into() }));
        }
    }
    for (idx, why) in failures {
        let c = &mut comps[idx];
        let Some(def) = cat.component(&c.key) else { continue };
        let (mel, source) = match c.key.as_str() {
            "TYRE_MAIN" => ("32-41-01", "Wear"),
            "TYRE_NOSE" => ("32-41-02", "Wear"),
            _ => (def.mel.as_str(), if def.kind() == "wear" { "Wear" } else if def.kind() == "cabin" { "Cabin" } else { "Life" }),
        };
        let detail = format!(
            "{}: {why}.",
            def.symptom.as_deref().map(|s| s.replace("{pos}", &c.pos)).unwrap_or_else(|| def.name_for(&c.pos))
        );
        if let Some(prev) = c.defect_id.take() {
            rectify(pool, prev, "superseded by the failure").await?;
        }
        let pos = (!c.pos.is_empty()).then_some(c.pos.as_str());
        c.defect_id = raise(pool, cat, &reg, mel, pos, source, Some(&c.key), &detail, Some(rec.flight_id)).await?;
        c.status = "Failed".into();
        events.push(FlightEvent { phase: "FLIGHT".into(), kind: "DEFECT".into(), severity: "Warning".into(), text: format!("{} failed: {why}", def.name_for(&c.pos)) });
    }
    for c in comps.iter_mut() {
        if c.status != "Ok" {
            continue;
        }
        let Some(def) = cat.component(&c.key) else { continue };
        if def.degraded_mel.is_none() && def.degraded_symptom.is_none() {
            continue;
        }
        let rem = remaining(c, def, &a, &r, now);
        if rem <= DEGRADED_AT_PCT {
            let mel = def.degraded_mel.clone().unwrap_or_else(|| def.mel.clone());
            let detail = format!(
                "{}. The part is near the end of its life ({rem:.0}% left); replacing it clears the fault.",
                def.degraded_symptom.as_deref().map(|s| s.replace("{pos}", &c.pos)).unwrap_or_else(|| format!("{} degraded", def.name_for(&c.pos)))
            );
            let pos = (!c.pos.is_empty()).then_some(c.pos.as_str());
            c.defect_id = raise(pool, cat, &reg, &mel, pos, "Life", Some(&c.key), &detail, Some(rec.flight_id)).await?;
            c.status = "Degraded".into();
        }
    }

    // 8) Defectos de excedencias / aterrizajes.
    for (mel, pos, detail) in raises {
        let src = if mel.starts_with("05-51") || mel.starts_with("32-1") { "Landing" } else { "Exceedance" };
        raise(pool, cat, &reg, mel, pos.as_deref(), src, None, &detail, Some(rec.flight_id)).await?;
    }
    for (i, m) in a.egt.clone().iter().enumerate() {
        if *m < 10.0 {
            let pos = (i + 1).to_string();
            raise(pool, cat, &reg, "72-00-04", Some(&pos), "Wear", Some("EGT_MARGIN"), &format!("EGT margin {m:.0} °C."), Some(rec.flight_id)).await?;
        }
    }

    // 9) Violación de despacho: despegó con ítems NO-GO abiertos o checks
    //    vencidos (como TechLog: flag + evento "Departed NO-GO: …").
    let mut violation = false;
    if rec.takeoffs > 0 {
        let started: Option<String> = sqlx::query_scalar("SELECT started_at FROM flight_log WHERE id = ?1")
            .bind(rec.flight_id)
            .fetch_optional(pool)
            .await?
            .flatten();
        if let Some(start) = started.as_deref().and_then(parse_any_utc) {
            let mut items: Vec<String> = Vec::new();
            for d in sqlx::query("SELECT title, raised_at, rectified_at FROM mx_defect WHERE registration = ?1 AND category = 'NoGo'")
                .bind(&reg)
                .fetch_all(pool)
                .await?
            {
                let raised = d.try_get::<String, _>("raised_at").ok().as_deref().and_then(parse_any_utc);
                let fixed = d.try_get::<Option<String>, _>("rectified_at").ok().flatten().as_deref().and_then(parse_any_utc);
                if raised.is_some_and(|t| t < start) && fixed.is_none_or(|t| t > start) {
                    items.push(d.try_get("title").unwrap_or_default());
                }
            }
            for st in check_states(pool, &a).await? {
                if st.due.is_some_and(|d| d < start) {
                    items.push(format!("{} overdue", st.name));
                }
            }
            items.dedup();
            if !items.is_empty() {
                violation = true;
                let more = if items.len() > 3 { format!(" and {} more", items.len() - 3) } else { String::new() };
                let text = format!("Departed NO-GO: {}{more}", items.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
                events.push(FlightEvent { phase: "TAKEOFF".into(), kind: "DISPATCH_VIOLATION".into(), severity: "Warning".into(), text });
            }
        }
    }

    // 10) Guardar todo.
    for c in &comps {
        save_component(pool, c).await?;
    }
    save_airframe(pool, &a).await?;
    housekeeping(pool, &a).await?;
    let warnings = events.iter().filter(|e| e.severity == "Warning").count();
    let summary = format!(
        "{:.1} h block, {} landing(s){}",
        rec.block_h,
        rec.landings.len(),
        if warnings > 0 { format!(", {warnings} warning(s)") } else { String::new() }
    );
    sqlx::query(
        "INSERT OR REPLACE INTO mx_flight (flight_id, registration, summary, events, wear, dispatch_violation, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
    )
    .bind(rec.flight_id)
    .bind(&reg)
    .bind(&summary)
    .bind(serde_json::to_string(&events)?)
    .bind(serde_json::to_string(&wear.0)?)
    .bind(violation)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    tracing::info!(target: "mx", "vuelo {} aplicado al Tech Log de {reg}: {summary}", rec.flight_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Mantenimiento de calendario: checks vencidos y diferidos que vencen
// ---------------------------------------------------------------------------

fn check_mel(key: &str) -> &'static str {
    match key {
        "DAILY" => "05-20-01",
        "WEEKLY" => "05-20-02",
        "A" => "05-20-03",
        _ => "05-20-04",
    }
}

struct CheckState {
    key: String,
    name: String,
    last: Option<DateTime<Utc>>,
    due: Option<DateTime<Utc>>,
    hours_left: Option<f64>,
    overdue: bool,
}

async fn check_states(pool: &SqlitePool, a: &Airframe) -> Result<Vec<CheckState>> {
    let rows = sqlx::query("SELECT * FROM mx_check WHERE registration = ?1")
        .bind(&a.reg)
        .fetch_all(pool)
        .await?;
    let now = Utc::now();
    Ok(a.cat()
        .limits
        .checks
        .iter()
        .map(|def| {
            let row = rows.iter().find(|r| r.try_get::<String, _>("key").ok().as_deref() == Some(def.key.as_str()));
            let last = row
                .and_then(|r| r.try_get::<String, _>("last_done_at").ok())
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc));
            let fh0: f64 = row.and_then(|r| r.try_get("last_done_fh").ok()).unwrap_or(0.0);
            let fc0: f64 = row.and_then(|r| r.try_get("last_done_fc").ok()).unwrap_or(0.0);
            let interval_h = def.days.map(|d| d * 24.0).or(def.hours);
            let due = last.zip(interval_h).map(|(t, h)| t + CDur::minutes((h * 60.0) as i64));
            let mut left = due.map(|d| (d - now).num_minutes() as f64 / 60.0);
            if let Some(fh) = def.flight_hours {
                let l = fh0 + fh - a.counters.flight_hours;
                left = Some(left.map_or(l, |x| x.min(l)));
            }
            let fc_over = def.cycles.is_some_and(|fc| a.counters.cycles >= fc0 + fc);
            CheckState {
                key: def.key.clone(),
                name: def.name.clone(),
                last,
                due,
                hours_left: left,
                overdue: left.is_some_and(|l| l <= 0.0) || fc_over,
            }
        })
        .collect())
}

/// Checks vencidos → defecto NO-GO; diferidos que llegan a su fecha → los
/// arregla mantenimiento (y se cobra), como en TechLog.
async fn housekeeping(pool: &SqlitePool, a: &Airframe) -> Result<()> {
    let cat = a.cat();
    for st in check_states(pool, a).await? {
        if st.overdue {
            raise(pool, cat, &a.reg, check_mel(&st.key), None, "Check", Some(&format!("CHECK:{}", st.key)), &format!("{} overdue.", st.name), None).await?;
        }
    }
    let today = Utc::now().date_naive();
    let rows = sqlx::query("SELECT id, deferred_until FROM mx_defect WHERE registration = ?1 AND status = 'Deferred'")
        .bind(&a.reg)
        .fetch_all(pool)
        .await?;
    for r in rows {
        let id: i64 = r.try_get("id").unwrap_or(0);
        let until: Option<String> = r.try_get("deferred_until").ok().flatten();
        if until
            .as_deref()
            .and_then(|u| NaiveDate::parse_from_str(u, "%Y-%m-%d").ok())
            .is_some_and(|d| d <= today)
        {
            fix_defect(pool, a, id, "fixed by maintenance as scheduled").await?;
        }
    }
    Ok(())
}

async fn in_flight(pool: &SqlitePool, reg: &str) -> bool {
    let rows = sqlx::query(
        "SELECT aircraft_registration FROM flight_log WHERE ended_at IS NULL AND started_at >= datetime('now', '-1 day')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.iter().any(|r| {
        r.try_get::<Option<String>, _>("aircraft_registration")
            .ok()
            .flatten()
            .is_some_and(|x| normalize_reg(&x) == reg)
    })
}

// ---------------------------------------------------------------------------
// Acciones
// ---------------------------------------------------------------------------

fn part_base_cost(cat: &Catalog, def: &CompDef) -> f64 {
    crate::techlog::part_cost_by_ata(Some(def.ata), &cat.spares.common_ata, &cat.spares.rare_ata)
}

async fn replace_part(pool: &SqlitePool, a: &Airframe, c: &mut Comp, def: &CompDef) -> Result<f64> {
    let now = Utc::now();
    c.serial += 1;
    c.life = (def.life * life_factor(&a.reg, &def.key, &c.pos, c.serial) * 10.0).round() / 10.0;
    c.basis_at_install = a.counters.wear_value(def.basis(), engine_index(&c.pos), now);
    c.wear = 0.0;
    c.repairs = 0;
    c.status = "Ok".into();
    if let Some(d) = c.defect_id.take() {
        rectify(pool, d, "part replaced").await?;
    }
    sqlx::query("UPDATE mx_component SET installed_at = ?2 WHERE id = ?1")
        .bind(c.id)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
    save_component(pool, c).await?;
    charge(pool, &a.reg, "part_replace", &def.name_for(&c.pos), part_base_cost(a.cat(), def)).await
}

async fn repair_part(pool: &SqlitePool, a: &Airframe, r: &Rates, c: &mut Comp, def: &CompDef) -> Result<Option<f64>> {
    let steps = &a.cat().limits.common.repair_to_pct;
    if def.kind() == "wear" || c.repairs as usize >= steps.len() {
        return Ok(None);
    }
    let to = steps[c.repairs as usize];
    set_remaining(c, def, a, r, to, Utc::now());
    c.repairs += 1;
    c.status = "Ok".into();
    if let Some(d) = c.defect_id.take() {
        rectify(pool, d, &format!("part repaired to {to:.0}%")).await?;
    }
    save_component(pool, c).await?;
    Ok(Some(charge(pool, &a.reg, "part_repair", &def.name_for(&c.pos), (part_base_cost(a.cat(), def) * 0.3).round()).await?))
}

async fn perform_check(pool: &SqlitePool, a: &Airframe, key: &str) -> Result<f64> {
    sqlx::query(
        "INSERT INTO mx_check (registration, key, last_done_at, last_done_fh, last_done_fc) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(registration, key) DO UPDATE SET last_done_at = excluded.last_done_at, last_done_fh = excluded.last_done_fh, last_done_fc = excluded.last_done_fc",
    )
    .bind(&a.reg)
    .bind(key)
    .bind(Utc::now().to_rfc3339())
    .bind(a.counters.flight_hours)
    .bind(a.counters.cycles)
    .execute(pool)
    .await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM mx_defect WHERE registration = ?1 AND component_key = ?2 AND status IN ('Open','Deferred')",
    )
    .bind(&a.reg)
    .bind(format!("CHECK:{key}"))
    .fetch_all(pool)
    .await?;
    for id in ids {
        rectify(pool, id, "check performed").await?;
    }
    charge(pool, &a.reg, "check", key, crate::techlog::check_cost(key)).await
}

/// Rectifica un defecto: si es de una pieza, la repara (si se puede) o la
/// reemplaza; si es de un check, lo hace; si no, horas de taller.
async fn fix_defect(pool: &SqlitePool, a: &Airframe, id: i64, how: &str) -> Result<f64> {
    let row = sqlx::query("SELECT * FROM mx_defect WHERE id = ?1 AND registration = ?2")
        .bind(id)
        .bind(&a.reg)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("defecto no encontrado"))?;
    let comp_key: Option<String> = row.try_get("component_key").ok().flatten();
    let pos: Option<String> = row.try_get("position").ok().flatten();
    let mel: Option<String> = row.try_get("mel_id").ok().flatten();
    let cat = a.cat();
    if let Some(k) = comp_key.as_deref().and_then(|k| k.strip_prefix("CHECK:")) {
        return perform_check(pool, a, k).await;
    }
    if let Some(k) = comp_key.as_deref().filter(|k| cat.component(k).is_some()) {
        let def = cat.component(k).unwrap();
        let mut comps = load_components(pool, &a.reg).await?;
        if let Some(c) = comps.iter_mut().find(|c| c.key == k && Some(c.pos.as_str()) == pos.as_deref().or(Some(""))) {
            let r = rates(pool).await;
            let cost = if c.status == "Failed" || def.kind() == "wear" {
                replace_part(pool, a, c, def).await?
            } else {
                match repair_part(pool, a, &r, c, def).await? {
                    Some(cost) => cost,
                    None => replace_part(pool, a, c, def).await?,
                }
            };
            rectify(pool, id, how).await?;
            return Ok(cost);
        }
    }
    let hours = mel
        .as_deref()
        .and_then(|m| cat.mel_item(m))
        .and_then(|m| m.fix_hours)
        .unwrap_or(1.0)
        .max(0.5);
    let title: String = row.try_get("title").unwrap_or_default();
    rectify(pool, id, how).await?;
    charge(pool, &a.reg, "defect", &title, (hours * crate::techlog::LABOR_PER_HOUR).round()).await
}

async fn defer_defect(pool: &SqlitePool, a: &Airframe, id: i64) -> Result<bool> {
    let row = sqlx::query("SELECT category, status FROM mx_defect WHERE id = ?1 AND registration = ?2")
        .bind(id)
        .bind(&a.reg)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(false) };
    let cat: String = row.try_get("category").unwrap_or_default();
    let status: String = row.try_get("status").unwrap_or_default();
    let Some(days) = defer_days(&cat) else { return Ok(false) };
    if status != "Open" {
        return Ok(false);
    }
    let until = (Utc::now().date_naive() + CDur::days(days)).format("%Y-%m-%d").to_string();
    sqlx::query("UPDATE mx_defect SET status = 'Deferred', deferred_until = ?2 WHERE id = ?1")
        .bind(id)
        .bind(&until)
        .execute(pool)
        .await?;
    Ok(true)
}

async fn service(pool: &SqlitePool, a: &mut Airframe, kind: &str) -> Result<f64> {
    let key = kind.split(':').next().unwrap_or(kind);
    let cdef = a.cat().consumable(key).ok_or_else(|| anyhow!("consumible desconocido"))?;
    let cap = cdef.capacity(&a.engine);
    let before = a.consumables.get(kind).copied().unwrap_or(if cdef.fills { 0.0 } else { cap });
    let after = if cdef.fills { 0.0 } else { cap };
    a.consumables.insert(kind.to_string(), after);
    save_airframe(pool, a).await?;
    charge(pool, &a.reg, "service", kind, crate::techlog::consumable_cost(key, (after - before).abs())).await
}

pub async fn action(pool: &SqlitePool, req: TlActionRequest) -> Result<TlActionResult> {
    let reg = normalize_reg(&req.registration);
    if in_flight(pool, &reg).await {
        return Err(anyhow!("in_flight"));
    }
    let mut a = ensure_airframe(pool, &reg, None, None).await?;
    let cat = a.cat();
    let r = rates(pool).await;
    let id = || req.id.ok_or_else(|| anyhow!("falta id"));
    let (ok, message) = match req.action.as_str() {
        "fix" => {
            let cost = fix_defect(pool, &a, id()?, "fixed by the pilot").await?;
            (true, format!("Fixed ({cost:.0} USD)."))
        }
        "defer" => {
            if defer_defect(pool, &a, id()?).await? {
                (true, "Deferred under the MEL.".into())
            } else {
                (false, "This item cannot be deferred.".into())
            }
        }
        "deferAll" => {
            let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM mx_defect WHERE registration = ?1 AND status = 'Open'")
                .bind(&reg)
                .fetch_all(pool)
                .await?;
            let mut n = 0;
            for i in ids {
                if defer_defect(pool, &a, i).await? {
                    n += 1;
                }
            }
            (true, format!("{n} item(s) deferred."))
        }
        "partFix" | "partReplace" => {
            let ids = req.ids.clone().or_else(|| req.id.map(|i| vec![i])).unwrap_or_default();
            let mut comps = load_components(pool, &reg).await?;
            let mut total = 0.0;
            let mut refused = 0;
            for c in comps.iter_mut().filter(|c| ids.contains(&c.id)) {
                let Some(def) = cat.component(&c.key) else { continue };
                if req.action == "partReplace" {
                    total += replace_part(pool, &a, c, def).await?;
                } else {
                    match repair_part(pool, &a, &r, c, def).await? {
                        Some(cost) => total += cost,
                        None => refused += 1,
                    }
                }
            }
            if refused > 0 && total == 0.0 {
                (false, "This part cannot be repaired any more: replace it.".into())
            } else {
                (true, format!("Done ({total:.0} USD)."))
            }
        }
        "service" => {
            let kind = req.key.clone().ok_or_else(|| anyhow!("falta el servicio"))?;
            let cost = service(pool, &mut a, &kind).await?;
            (true, format!("Serviced ({cost:.0} USD)."))
        }
        "check" => {
            let key = req.key.clone().ok_or_else(|| anyhow!("falta el check"))?;
            let cost = perform_check(pool, &a, &key).await?;
            (true, format!("{key} check done ({cost:.0} USD)."))
        }
        "callMaintenance" => {
            let mut total = 0.0;
            let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM mx_defect WHERE registration = ?1 AND status IN ('Open','Deferred')")
                .bind(&reg)
                .fetch_all(pool)
                .await?;
            for i in ids {
                total += fix_defect(pool, &a, i, "fixed by maintenance").await?;
            }
            let kinds: Vec<String> = consumable_views(&a)
                .into_iter()
                .filter(|c| c.status != "ok")
                .map(|c| c.service_kind)
                .collect();
            for k in kinds {
                total += service(pool, &mut a, &k).await?;
            }
            for st in check_states(pool, &a).await? {
                if st.overdue {
                    total += perform_check(pool, &a, &st.key).await?;
                }
            }
            (true, format!("Maintenance done ({total:.0} USD)."))
        }
        other => return Err(anyhow!("acción desconocida: {other}")),
    };
    tracing::info!(target: "mx", "{reg} {}: ok={ok} {message}", req.action);
    Ok(TlActionResult { ok, message, queued: false })
}

// ---------------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------------

fn consumable_views(a: &Airframe) -> Vec<TlConsumable> {
    let mut out = Vec::new();
    for c in &a.cat().consumables {
        for p in c.positions_or_single() {
            let ck = if p.is_empty() { c.key.clone() } else { format!("{}:{p}", c.key) };
            let cap = c.capacity(&a.engine);
            let level = a.consumables.get(&ck).copied().unwrap_or(if c.fills { 0.0 } else { cap });
            let (min, low) = (c.min_dispatch(&a.engine), c.low(&a.engine));
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
            out.push(TlConsumable {
                key: c.key.clone(),
                position: (!p.is_empty()).then(|| p.clone()),
                name: c.name_for(&p),
                unit: c.unit.clone(),
                level: (level * 10.0).round() / 10.0,
                capacity: cap,
                min_dispatch: min,
                low,
                fills: c.fills,
                status: status.into(),
                service_kind: ck,
                service_label: c.service_label.clone().unwrap_or_else(|| if c.fills { "Empty".into() } else { "Fill".into() }),
            });
        }
    }
    out
}

/// Estado del avión en el Tech Log propio (lo crea nuevo si no existe).
pub async fn aircraft_view(pool: &SqlitePool, registration: &str, title_hint: Option<&str>) -> Result<Option<TechLogAircraft>> {
    let reg = normalize_reg(registration);
    if reg.is_empty() {
        return Ok(None);
    }
    let a = ensure_airframe(pool, &reg, None, title_hint).await?;
    housekeeping(pool, &a).await?;
    let cat = a.cat();
    let r = rates(pool).await;
    let now = Utc::now();
    let today = now.date_naive();

    let rows = sqlx::query("SELECT * FROM mx_defect WHERE registration = ?1 AND status IN ('Open','Deferred') ORDER BY raised_at")
        .bind(&reg)
        .fetch_all(pool)
        .await?;
    let defects: Vec<TlDefect> = rows
        .iter()
        .map(|d| {
            let category: String = d.try_get("category").unwrap_or_default();
            let status: String = d.try_get("status").unwrap_or_default();
            let latest = defer_days(&category)
                .filter(|_| status == "Open")
                .map(|days| (today + CDur::days(days)).format("%Y-%m-%d").to_string());
            TlDefect {
                id: d.try_get("id").unwrap_or(0),
                ata: d.try_get("ata").unwrap_or(0),
                title: d.try_get("title").unwrap_or_default(),
                detail: d.try_get("detail").ok().flatten(),
                can_defer: latest.is_some(),
                category,
                status,
                mel_id: d.try_get("mel_id").ok().flatten(),
                position: d.try_get::<Option<String>, _>("position").ok().flatten().filter(|p| !p.is_empty()),
                raised_utc: d.try_get("raised_at").ok(),
                deferred_until: d.try_get("deferred_until").ok().flatten(),
                latest_deferral: latest,
                failure: None,
                placard: None,
                restrictions: None,
            }
        })
        .collect();

    let comps = load_components(pool, &reg).await?;
    let worn_below = cat.limits.common.worn_below_pct;
    let mut condition = TlCondition::default();
    let mut parts = Vec::new();
    for c in &comps {
        let Some(def) = cat.component(&c.key) else { continue };
        let rem = remaining(c, def, &a, &r, now);
        condition.count(&c.status, rem < worn_below, rem);
        if c.status != "Ok" || rem < worn_below {
            parts.push(TlPart {
                id: c.id,
                key: c.key.clone(),
                name: def.name_for(&c.pos),
                position: (!c.pos.is_empty()).then(|| c.pos.clone()),
                ata: def.ata,
                system: def.system.clone(),
                kind: def.kind().to_string(),
                remaining_pct: (rem * 10.0).round() / 10.0,
                status: c.status.clone(),
                repairs: c.repairs,
            });
        }
    }
    parts.sort_by(|x, y| x.remaining_pct.total_cmp(&y.remaining_pct));

    let checks = check_states(pool, &a)
        .await?
        .into_iter()
        .map(|s| TlCheck {
            key: s.key,
            name: s.name,
            last_done_utc: s.last.map(|d| d.to_rfc3339()),
            due_utc: s.due.map(|d| d.to_rfc3339()),
            overdue: s.overdue,
            hours_left: s.hours_left.map(|h| (h * 10.0).round() / 10.0),
        })
        .collect();

    let dispatch = {
        let mut reasons = Vec::new();
        for d in defects.iter().filter(|d| d.status == "Open" && d.category == "NoGo") {
            reasons.push(format!("NO-GO: {}", d.title));
        }
        for d in defects.iter().filter(|d| d.status == "Open" && d.category != "NoGo") {
            reasons.push(format!("Open, fix or defer: {}", d.title));
        }
        if !reasons.is_empty() {
            TlDispatch { status: "NO_GO".into(), reasons }
        } else {
            let n = defects.iter().filter(|d| d.status == "Deferred").count();
            if n == 0 {
                TlDispatch { status: "GO".into(), reasons }
            } else {
                TlDispatch { status: "GO_MEL".into(), reasons: vec![format!("{n} item(s) deferred under the MEL")] }
            }
        }
    };

    let last_seen: Option<String> = sqlx::query_scalar("SELECT last_seen_at FROM mx_airframe WHERE registration = ?1")
        .bind(&reg)
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(Some(TechLogAircraft {
        source: TlSource::Simfleet,
        app_name: TlSource::Simfleet.app_name().into(),
        tablet_url: String::new(),
        running: true,
        active: !in_flight(pool, &reg).await,
        airframe_id: 0,
        registration: reg.clone(),
        variant: a.variant.clone(),
        engine: a.engine.clone(),
        maintenance: "standard".into(),
        flight_hours: (a.counters.flight_hours * 10.0).round() / 10.0,
        cycles: a.counters.cycles,
        egt_margin_c: a.egt.clone(),
        dispatch,
        defects,
        parts,
        condition,
        consumables: consumable_views(&a),
        checks,
        last_seen_utc: last_seen,
    }))
}

/// Registro de mantenimiento de un vuelo del FlightBook.
pub async fn flight_view(pool: &SqlitePool, flight_id: i64) -> Result<Option<TechLogFlight>> {
    let row = sqlx::query("SELECT * FROM mx_flight WHERE flight_id = ?1")
        .bind(flight_id)
        .fetch_optional(pool)
        .await?;
    let Some(r) = row else { return Ok(None) };
    let events: Vec<FlightEvent> = r
        .try_get::<String, _>("events")
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
        .map(|v| {
            v.into_iter()
                .filter_map(|e| {
                    Some(FlightEvent {
                        phase: e.get("phase")?.as_str()?.to_string(),
                        kind: e.get("kind")?.as_str()?.to_string(),
                        severity: e.get("severity")?.as_str()?.to_string(),
                        text: e.get("text")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let wear: Vec<TlFlightWear> = r
        .try_get::<String, _>("wear")
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
        .map(|v| {
            v.into_iter()
                .filter_map(|w| {
                    Some(TlFlightWear {
                        item: w.get("item")?.as_str()?.to_string(),
                        amount: w.get("amount")?.as_f64()?,
                        unit: w.get("unit")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(TechLogFlight {
        source: TlSource::Simfleet,
        app_name: TlSource::Simfleet.app_name().into(),
        flight_id,
        status: "Closed".into(),
        score: None,
        score_notes: Vec::new(),
        summary: r.try_get("summary").ok().flatten(),
        events: events
            .into_iter()
            .map(|e| TlFlightEvent { phase: Some(e.phase), kind: e.kind, severity: e.severity, text: e.text })
            .collect(),
        wear,
        dispatch_violation: r.try_get::<i64, _>("dispatch_violation").unwrap_or(0) != 0,
    }))
}

/// Fecha/hora en RFC 3339 o en el formato SQLite "AAAA-MM-DD HH:MM:SS" (UTC).
fn parse_any_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

// ---------------------------------------------------------------------------
// (3B) Ritmo de desgaste, mantenimiento nocturno de la aerolínea y avisos
// ---------------------------------------------------------------------------

pub async fn get_wear_rates(pool: &SqlitePool) -> (f64, f64) {
    let r = rates(pool).await;
    (r.life, r.cabin)
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cambia el ritmo de desgaste SIN saltos: cada pieza conserva su % restante
/// (se rebasa su punto de instalación, como `RebaseWearRates` de TechLog).
pub async fn set_wear_rates(pool: &SqlitePool, life: f64, cabin: f64) -> Result<()> {
    let old = rates(pool).await;
    let new = Rates { life: life.max(0.05), cabin: cabin.max(0.05) };
    let now = Utc::now();
    let regs: Vec<String> = sqlx::query_scalar("SELECT registration FROM mx_airframe")
        .fetch_all(pool)
        .await?;
    for reg in regs {
        let Some(a) = load_airframe(pool, &reg).await? else { continue };
        let cat = a.cat();
        for mut c in load_components(pool, &reg).await? {
            let Some(def) = cat.component(&c.key) else { continue };
            if def.kind() == "wear" {
                continue;
            }
            let rem = remaining(&c, def, &a, &old, now);
            set_remaining(&mut c, def, &a, &new, rem, now);
            save_component(pool, &c).await?;
        }
    }
    set_setting(pool, "mx_wear_rate", &new.life.to_string()).await?;
    set_setting(pool, "mx_cabin_wear_rate", &new.cabin.to_string()).await?;
    tracing::info!(target: "mx", "ritmo de desgaste: piezas ×{} · cabina ×{}", new.life, new.cabin);
    Ok(())
}

/// Mantenimiento nocturno de la aerolínea (nivel premium): arregla todo lo
/// pendiente, renueva las piezas a punto de fallar, repone consumibles y hace
/// los checks vencidos o que vencen en 24 h. Todo se cobra. Devuelve cuántos
/// aviones atendió.
pub async fn nightly(pool: &SqlitePool) -> Result<usize> {
    let regs: Vec<String> = sqlx::query_scalar("SELECT registration FROM mx_airframe")
        .fetch_all(pool)
        .await?;
    let mut served = 0;
    for reg in regs {
        let (level, _, _) = crate::aircraft_maintenance::airline_for_registration(pool, &reg).await;
        if level < 2 || in_flight(pool, &reg).await {
            continue;
        }
        let Some(mut a) = load_airframe(pool, &reg).await? else { continue };
        let cat = a.cat();
        let r = rates(pool).await;
        let now = Utc::now();
        let mut total = 0.0;
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM mx_defect WHERE registration = ?1 AND status IN ('Open','Deferred')")
            .bind(&reg)
            .fetch_all(pool)
            .await?;
        for id in ids {
            total += fix_defect(pool, &a, id, "fixed overnight by the airline").await?;
        }
        for mut c in load_components(pool, &reg).await? {
            let Some(def) = cat.component(&c.key) else { continue };
            let rem = remaining(&c, def, &a, &r, now);
            let due = c.status != "Ok" || rem <= STRESS_MARGIN_PCT || (def.kind() == "wear" && rem <= 10.0);
            if due {
                total += replace_part(pool, &a, &mut c, def).await?;
            }
        }
        let kinds: Vec<String> = consumable_views(&a)
            .into_iter()
            .filter(|c| c.status != "ok")
            .map(|c| c.service_kind)
            .collect();
        for k in kinds {
            total += service(pool, &mut a, &k).await?;
        }
        for st in check_states(pool, &a).await? {
            if st.overdue || st.hours_left.is_some_and(|h| h <= 24.0) {
                total += perform_check(pool, &a, &st.key).await?;
            }
        }
        if total > 0.0 {
            served += 1;
            tracing::info!(target: "mx", "{reg}: mantenimiento nocturno de la aerolínea ({total:.0} USD)");
        }
    }
    Ok(served)
}

/// Avisos de mantenimiento para la campana: NO-GO (rojo) y lo que vence
/// pronto (ámbar) — checks en < 24 h, consumibles bajos, piezas degradadas.
/// Incluye los aviones de Fenix/PMDG TechLog con NO-GO.
pub async fn alerts(pool: &SqlitePool) -> Result<Vec<crate::aircraft_maintenance::MaintAlert>> {
    use crate::aircraft_maintenance::MaintAlert;
    let mut out = Vec::new();
    let managed = crate::techlog::managed_registrations().await;
    // Las matrículas se guardan normalizadas; la campana las enseña como en el FlightBook.
    let mut shown: HashMap<String, String> = HashMap::new();
    let raw: Vec<String> = sqlx::query_scalar(
        "SELECT aircraft_registration FROM flight_log WHERE aircraft_registration IS NOT NULL
         GROUP BY aircraft_registration ORDER BY MAX(started_at) DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for r in raw {
        shown.entry(normalize_reg(&r)).or_insert_with(|| r.trim().to_uppercase());
    }
    let display = |reg: &str| shown.get(&normalize_reg(reg)).cloned().unwrap_or_else(|| reg.to_string());
    let rows = sqlx::query("SELECT registration, title FROM mx_airframe")
        .fetch_all(pool)
        .await?;
    for row in rows {
        let reg: String = row.try_get("registration").unwrap_or_default();
        if managed.contains_key(&reg) {
            continue;
        }
        let title: Option<String> = row.try_get("title").ok().flatten();
        let Some(v) = aircraft_view(pool, &reg, None).await? else { continue };
        let mut soon: Vec<String> = Vec::new();
        for c in v.checks.iter().filter(|c| !c.overdue && c.hours_left.is_some_and(|h| h <= 24.0)) {
            soon.push(format!("{} due in {:.0} h", c.name, c.hours_left.unwrap_or(0.0).max(0.0)));
        }
        for c in v.consumables.iter().filter(|c| c.status != "ok") {
            soon.push(format!("{} {}", c.name, if c.status == "below_min" { "below dispatch minimum" } else { "low" }));
        }
        for p in v.parts.iter().filter(|p| p.status != "Ok") {
            soon.push(format!("{} {}", p.name, p.status.to_lowercase()));
        }
        let (severity, label, count) = if v.dispatch.status == "NO_GO" {
            let n = v.dispatch.reasons.len();
            ("due", v.dispatch.reasons.first().cloned().unwrap_or_default(), n)
        } else if !soon.is_empty() {
            ("soon", soon[0].clone(), soon.len())
        } else {
            continue;
        };
        let (_, airline_key, airline_name) = crate::aircraft_maintenance::airline_for_registration(pool, &reg).await;
        out.push(MaintAlert {
            registration: display(&reg),
            model: title.or(Some(v.variant.clone())),
            airline_key,
            airline_name,
            component: "techlog".into(),
            wear_pct: if severity == "due" { 100.0 } else { 80.0 } + 5.0 * count as f64,
            severity: severity.into(),
            due_count: count as i64,
            label: Some(format!("{} · {label}", v.app_name)),
        });
    }
    for (reg, _) in managed {
        let Ok(Some(v)) = crate::techlog::aircraft(&reg).await else { continue };
        if v.dispatch.status != "NO_GO" {
            continue;
        }
        let n = v.dispatch.reasons.len();
        let (_, airline_key, airline_name) = crate::aircraft_maintenance::airline_for_registration(pool, &reg).await;
        out.push(MaintAlert {
            registration: display(&v.registration),
            model: Some(v.variant.clone()),
            airline_key,
            airline_name,
            component: "techlog".into(),
            wear_pct: 100.0 + 5.0 * n as f64,
            severity: "due".into(),
            due_count: n as i64,
            label: Some(format!("{} · {}", v.app_name, v.dispatch.reasons.first().cloned().unwrap_or_default())),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn life_factor_matches_techlog_range_and_is_stable() {
        let a = life_factor("N321US", "ELEV_SERVO", "L", 0);
        let b = life_factor("N321US", "ELEV_SERVO", "L", 0);
        assert_eq!(a, b);
        assert!((0.7..=1.3).contains(&a));
        assert_ne!(a, life_factor("N321US", "ELEV_SERVO", "L", 1));
    }

    /// Ciclo completo sin simulador: vuelo con aterrizaje duro y overspeed →
    /// alta del avión, defectos NO-GO, desgaste; reparar desde el Hangar
    /// cobra en Finanzas.
    #[tokio::test]
    async fn full_cycle_flight_fix_and_charge() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in include_str!("../../migrations/065_mx_techlog.sql").split(';') {
            if stmt.lines().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with("--")) {
                sqlx::query(stmt).execute(&pool).await.unwrap();
            }
        }
        for sql in [
            "CREATE TABLE flight_log (id INTEGER PRIMARY KEY, aircraft_registration TEXT, aircraft_title TEXT, aircraft_atc_type TEXT, started_at TEXT, ended_at TEXT)",
            "CREATE TABLE maintenance_log (id INTEGER PRIMARY KEY, registration TEXT, component TEXT, cost REAL, serviced_at TEXT)",
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            "INSERT INTO flight_log VALUES (1, 'EC-TST', 'iFly 737-MAX8 Test', 'B38M', '2026-10-19T10:00:00Z', '2026-10-19T12:00:00Z')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let rec = FlightRecord {
            flight_id: 1,
            profile: None,
            block_h: 2.0,
            air_h: 1.6,
            eng_h: [2.0, 2.0],
            eng_starts: [1, 1],
            eng_ran: [true, true],
            takeoffs: 1,
            toga_takeoffs: 1,
            landings: vec![LandingRec { sink_fpm: 700.0, peak_g: 1.5, weight_kg: 60_000.0, gs_kt: 135.0, brake_energy_mj: 20.0, ..Default::default() }],
            effects: vec![Effect::Raise { mel: "05-51-03", pos: None, detail: "VMO exceeded: 362 kt (limit 340)".into() }],
            ..Default::default()
        };
        commit_flight(&pool, rec, Some(-700)).await.unwrap();

        let v = aircraft_view(&pool, "EC-TST", None).await.unwrap().unwrap();
        assert_eq!(v.variant, "737-8");
        assert_eq!(v.engine, "LEAP-1B");
        assert_eq!(v.flight_hours, 1.6);
        assert_eq!(v.dispatch.status, "NO_GO");
        let mels: Vec<String> = v.defects.iter().filter_map(|d| d.mel_id.clone()).collect();
        assert!(mels.contains(&"05-51-01".to_string()), "{mels:?}");
        assert!(mels.contains(&"05-51-03".to_string()), "{mels:?}");
        assert!(v.defects.iter().all(|d| d.category == "NoGo"));
        let oil = v.consumables.iter().find(|c| c.service_kind == "ENG_OIL:1").unwrap();
        assert!(oil.level < oil.capacity, "el aceite debe bajar");
        let tyre_wear: f64 = sqlx::query_scalar("SELECT MAX(wear) FROM mx_component WHERE key = 'TYRE_MAIN'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(tyre_wear > 12.0, "neumáticos: {tyre_wear}");
        let flight = flight_view(&pool, 1).await.unwrap().unwrap();
        assert!(flight.events.iter().any(|e| e.text.starts_with("Hard landing")));
        assert!(!flight.dispatch_violation);

        // (3B) Segundo vuelo con los NO-GO aún abiertos → violación de despacho.
        let later = (Utc::now() + CDur::minutes(5)).to_rfc3339();
        sqlx::query("INSERT INTO flight_log VALUES (2, 'EC-TST', 'iFly 737-MAX8 Test', 'B38M', ?1, ?1)")
            .bind(&later)
            .execute(&pool)
            .await
            .unwrap();
        let rec2 = FlightRecord {
            flight_id: 2,
            block_h: 1.0,
            air_h: 0.8,
            eng_h: [1.0, 1.0],
            eng_ran: [true, true],
            takeoffs: 1,
            landings: vec![LandingRec { sink_fpm: 150.0, peak_g: 1.2, weight_kg: 55_000.0, gs_kt: 130.0, ..Default::default() }],
            ..Default::default()
        };
        commit_flight(&pool, rec2, Some(-150)).await.unwrap();
        let f2 = flight_view(&pool, 2).await.unwrap().unwrap();
        assert!(f2.dispatch_violation, "debe marcar violación");
        assert!(f2.events.iter().any(|e| e.kind == "DISPATCH_VIOLATION" && e.text.starts_with("Departed NO-GO")));

        // (3B) Cambiar el ritmo de desgaste no hace saltar el % restante.
        let before = aircraft_view(&pool, "EC-TST", None).await.unwrap().unwrap();
        let comps_before = load_components(&pool, "ECTST").await.unwrap();
        let a0 = load_airframe(&pool, "ECTST").await.unwrap().unwrap();
        let r0 = rates(&pool).await;
        let probe = comps_before.iter().find(|c| c.key == "THS_ACTUATOR").unwrap().clone();
        let def = catalog(Family::B737).component("THS_ACTUATOR").cloned();
        set_wear_rates(&pool, 5.0, 3.0).await.unwrap();
        assert_eq!(get_wear_rates(&pool).await, (5.0, 3.0));
        if let Some(def) = def {
            let r1 = rates(&pool).await;
            let after = load_components(&pool, "ECTST").await.unwrap();
            let probe_after = after.iter().find(|c| c.id == probe.id).unwrap();
            let rem0 = remaining(&probe, &def, &a0, &r0, Utc::now());
            let rem1 = remaining(probe_after, &def, &a0, &r1, Utc::now());
            assert!((rem0 - rem1).abs() < 0.01, "{rem0} vs {rem1}");
        }
        drop(before);

        // Reparar la inspección de aterrizaje duro desde el Hangar → cobro.
        let id = v.defects.iter().find(|d| d.mel_id.as_deref() == Some("05-51-01")).unwrap().id;
        let r = action(&pool, TlActionRequest {
            registration: "EC-TST".into(),
            action: "fix".into(),
            id: Some(id),
            ids: None,
            until: None,
            key: None,
            label: None,
        })
        .await
        .unwrap();
        assert!(r.ok, "{}", r.message);
        let r = action(&pool, TlActionRequest {
            registration: "EC-TST".into(),
            action: "check".into(),
            id: None,
            ids: None,
            until: None,
            key: Some("DAILY".into()),
            label: None,
        })
        .await
        .unwrap();
        assert!(r.ok);
        let rows: Vec<(String, f64)> = sqlx::query_as("SELECT component, cost FROM maintenance_log ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].0.starts_with("mx:defect:") && rows[0].1 > 0.0, "{rows:?}");
        assert_eq!(rows[1], ("mx:check:DAILY".to_string(), 350.0));
        let v = aircraft_view(&pool, "EC-TST", None).await.unwrap().unwrap();
        assert_eq!(v.defects.len(), 1, "queda el overspeed");
    }

    #[test]
    fn landing_grades() {
        let cat = catalog(Family::A320);
        let mut ev = Vec::new();
        let hard = LandingRec { sink_fpm: 650.0, peak_g: 1.6, weight_kg: 60_000.0, gs_kt: 130.0, ..Default::default() };
        let fx = landing_effects(&hard, cat, "A320", &mut ev);
        assert!(fx.iter().any(|e| matches!(e, Effect::Raise { mel: "05-51-01", .. })));
        let soft = LandingRec { sink_fpm: 180.0, peak_g: 1.2, weight_kg: 60_000.0, gs_kt: 130.0, ..Default::default() };
        let fx = landing_effects(&soft, cat, "A320", &mut ev);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Raise { .. })));
        // Por encima del MLW: umbral de aterrizaje duro más bajo (360 fpm).
        let heavy = LandingRec { sink_fpm: 400.0, peak_g: 1.3, weight_kg: 70_000.0, gs_kt: 140.0, ..Default::default() };
        let fx = landing_effects(&heavy, cat, "A320", &mut ev);
        assert!(fx.iter().any(|e| matches!(e, Effect::Raise { mel: "05-51-07", .. })));
        assert!(fx.iter().any(|e| matches!(e, Effect::Raise { mel: "05-51-01", .. })));
    }
}
