//! (24/10) Libro de mantenimiento y finanzas en la nube, y borrados.
//!
//! Hasta ahora la nube guardaba sólo los vuelos: cambiar de PC (o perder la
//! base local) perdía el Tech Log de SimFleet, los cobros de mantenimiento, las
//! líneas de GSX, las políticas, traslados, reservas y la operación autónoma.
//! Y un vuelo borrado volvía a bajar (la subida era una unión).
//!
//! El libro (`Ledger`) viaja dentro del snapshot de la nube:
//! - Cada AVIÓN del Tech Log (piezas, defectos, checks, servicios) es un
//!   documento: entre dos PCs gana su versión más reciente (`updated_at`, que
//!   pasa a "ahora" cuando su contenido cambia en una PC). Un avión recién
//!   creado y sin uso nunca pisa a uno con historial.
//! - Lo que sólo crece (registros por vuelo, cobros, eventos de TechLog
//!   vistos, líneas de GSX, traslados, días autónomos) se une sin duplicar.
//! - Los vuelos se referencian por su clave ESTABLE (el id es de cada PC).
//! - Los borrados (`sync_tombstone`) se unen en la nube y se aplican en todas
//!   las PCs: lo que se borró a propósito no vuelve. Si el vuelo se vuelve a
//!   añadir después (reimportado), el borrado queda sin efecto.
//!
//! Lo derivado (P&L, economía) no viaja: se recalcula.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

pub const KIND_FLIGHT: &str = "flight";
pub const KIND_MAINT: &str = "maint";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Tombstone {
    pub kind: String,
    pub key: String,
    pub deleted_at: String,
    /// Se volvió a añadir después de borrarlo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revived_at: Option<String>,
}

impl Tombstone {
    /// ¿El borrado sigue valiendo?
    pub fn active(&self) -> bool {
        self.revived_at.as_deref().map_or(true, |r| self.deleted_at.as_str() > r)
    }
}

/// Un avión del Tech Log de SimFleet con todo lo suyo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AirframeDoc {
    pub registration: String,
    pub updated_at: String,
    pub airframe: Value,
    pub components: Vec<Value>,
    /// Con su `id` de origen (las piezas lo referencian) y `raisedFlightKey`.
    pub defects: Vec<Value>,
    pub checks: Vec<Value>,
    pub servicing: Vec<Value>,
}

impl AirframeDoc {
    /// Creado al verlo y nunca usado (ni vuelos ni mantenimiento).
    fn blank(&self) -> bool {
        s(&self.airframe, "lastSeenAt").is_empty() && self.defects.is_empty() && self.checks.is_empty() && self.servicing.is_empty()
    }

    /// ¿`self` debe reemplazar a `cur`?
    fn wins_over(&self, cur: &AirframeDoc) -> bool {
        match (self.blank(), cur.blank()) {
            (false, true) => true,
            (true, false) => false,
            _ => self.updated_at > cur.updated_at,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Ledger {
    pub airframes: Vec<AirframeDoc>,
    /// Registro del Tech Log por vuelo, con `flightKey`.
    pub mx_flights: Vec<Value>,
    pub maintenance_log: Vec<Value>,
    pub techlog_seen: Vec<Value>,
    pub gsx_lines: Vec<Value>,
    pub policies: Vec<Value>,
    /// Traslados pedidos a mano (los automáticos se recalculan).
    pub ferries: Vec<Value>,
    pub reservations: Vec<Value>,
    pub auto_daily: Vec<Value>,
    pub settings: Vec<Value>,
}

/// Ajustes que viajan con el libro (el resto son de cada PC o secretos).
fn synced_setting(key: &str) -> bool {
    matches!(
        key,
        "ferry_auto_since"
            | "mx_wear_rate"
            | "mx_cabin_wear_rate"
            | "simbrief_excluded_ofps"
            | "simbrief_pilot_id"
            | "rec_enabled"
            | "rec_fps"
            | "rec_max_clips"
            | "rec_osd_enabled"
            | "rec_osd_position"
    )
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default()
}

fn obj(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = Value::Object(Map::new());
    }
    v.as_object_mut().expect("object")
}

// ---------------------------------------------------------------------------
// Claves estables de vuelos y borrados
// ---------------------------------------------------------------------------

/// Claves de un vuelo, iguales a las del snapshot de la nube:
/// (import = "source|externalId", sim = "startedAt|lat|lon").
pub fn flight_keys_of(source: Option<&str>, external_id: Option<&str>, started_at: &str, lat: f64, lon: f64) -> (Option<String>, String) {
    let source = source.unwrap_or("simconnect");
    (
        external_id.filter(|e| !e.is_empty()).map(|e| format!("{source}|{e}")),
        format!("{started_at}|{lat:.2}|{lon:.2}"),
    )
}

fn db_flight_keys(r: &SqliteRow) -> (Option<String>, String) {
    flight_keys_of(
        r.get::<Option<String>, _>("source").as_deref(),
        r.get::<Option<String>, _>("external_id").as_deref(),
        &r.get::<Option<String>, _>("started_at").unwrap_or_default(),
        r.get::<Option<f64>, _>("origin_lat").unwrap_or(0.0),
        r.get::<Option<f64>, _>("origin_lon").unwrap_or(0.0),
    )
}

/// Claves de una fila de vuelo del snapshot (JSON camelCase).
pub fn json_flight_keys(row: &Value) -> (Option<String>, String) {
    let f = |k: &str| row.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let src = row.get("source").and_then(|v| v.as_str());
    let ext = row.get("externalId").and_then(|v| v.as_str());
    flight_keys_of(src, ext, s(row, "startedAt"), f("originLat"), f("originLon"))
}

const FLIGHT_KEY_COLS: &str = "id, source, external_id, started_at, origin_lat, origin_lon";

/// (id local → clave sim, clave (sim o import) → id local) de todos los vuelos.
async fn local_flight_keys(pool: &SqlitePool) -> Result<(HashMap<i64, String>, HashMap<String, i64>)> {
    let rows = sqlx::query(&format!("SELECT {FLIGHT_KEY_COLS} FROM flight_log"))
        .fetch_all(pool)
        .await?;
    let mut id_key = HashMap::new();
    let mut key_id = HashMap::new();
    for r in rows {
        let id: i64 = r.get("id");
        let (ik, sk) = db_flight_keys(&r);
        if let Some(k) = ik {
            key_id.insert(k, id);
        }
        key_id.insert(sk.clone(), id);
        id_key.insert(id, sk);
    }
    Ok((id_key, key_id))
}

async fn upsert_tombstone(pool: &SqlitePool, t: &Tombstone) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_tombstone (kind, key, deleted_at, revived_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(kind, key) DO UPDATE SET
           deleted_at = MAX(deleted_at, excluded.deleted_at),
           revived_at = NULLIF(MAX(COALESCE(revived_at, ''), COALESCE(excluded.revived_at, '')), '')",
    )
    .bind(&t.kind)
    .bind(&t.key)
    .bind(&t.deleted_at)
    .bind(&t.revived_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Anota un borrado (vuelve a valer aunque se hubiera revivido antes).
pub async fn record(pool: &SqlitePool, kind: &str, key: &str) -> Result<()> {
    upsert_tombstone(
        pool,
        &Tombstone {
            kind: kind.into(),
            key: key.into(),
            deleted_at: chrono::Utc::now().to_rfc3339(),
            revived_at: None,
        },
    )
    .await
}

/// Anota el borrado de los vuelos `ids` por sus claves estables. Llamar ANTES
/// de borrarlos (después ya no hay de dónde sacar la clave).
pub async fn record_flights_deleted(pool: &SqlitePool, ids: &[i64]) -> Result<()> {
    for id in ids {
        let row = sqlx::query(&format!("SELECT {FLIGHT_KEY_COLS} FROM flight_log WHERE id = ?1"))
            .bind(id)
            .fetch_optional(pool)
            .await?;
        let Some(r) = row else { continue };
        let (ik, sk) = db_flight_keys(&r);
        record(pool, KIND_FLIGHT, &sk).await?;
        if let Some(k) = ik {
            record(pool, KIND_FLIGHT, &k).await?;
        }
    }
    Ok(())
}

pub async fn load_tombstones(pool: &SqlitePool) -> Result<Vec<Tombstone>> {
    let rows: Vec<(String, String, String, Option<String>)> =
        sqlx::query_as("SELECT kind, key, deleted_at, revived_at FROM sync_tombstone ORDER BY kind, key")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(kind, key, deleted_at, revived_at)| Tombstone { kind, key, deleted_at, revived_at })
        .collect())
}

/// Unión de borrados: por clave, el borrado y la reaparición más recientes.
pub fn union_tombstones(a: &[Tombstone], b: &[Tombstone]) -> Vec<Tombstone> {
    let mut map: HashMap<(String, String), Tombstone> = HashMap::new();
    for t in a.iter().chain(b) {
        match map.get_mut(&(t.kind.clone(), t.key.clone())) {
            None => {
                map.insert((t.kind.clone(), t.key.clone()), t.clone());
            }
            Some(cur) => {
                if t.deleted_at > cur.deleted_at {
                    cur.deleted_at = t.deleted_at.clone();
                }
                if t.revived_at > cur.revived_at {
                    cur.revived_at = t.revived_at.clone();
                }
            }
        }
    }
    let mut out: Vec<Tombstone> = map.into_values().collect();
    out.sort_by(|x, y| (&x.kind, &x.key).cmp(&(&y.kind, &y.key)));
    out
}

/// Claves con el borrado vigente.
pub fn active_keys(ts: &[Tombstone], kind: &str) -> HashSet<String> {
    ts.iter().filter(|t| t.kind == kind && t.active()).map(|t| t.key.clone()).collect()
}

/// ¿Esta fila de vuelo del snapshot está borrada?
pub fn flight_row_dead(row: &Value, dead: &HashSet<String>) -> bool {
    let (ik, sk) = json_flight_keys(row);
    dead.contains(&sk) || ik.is_some_and(|k| dead.contains(&k))
}

/// Clave de un cobro de mantenimiento (no tiene id estable).
pub fn maint_key(registration: &str, component: &str, serviced_at: &str) -> String {
    format!("{registration}|{component}|{serviced_at}")
}

/// Aplica en esta PC los borrados que llegan de la nube y que aún no se
/// habían aplicado aquí: borra esos vuelos (revirtiendo su Tech Log) y esos
/// cobros, y guarda los borrados. Devuelve cuántos vuelos borró.
pub async fn apply_remote_tombstones(pool: &SqlitePool, remote: &[Tombstone]) -> Result<usize> {
    if remote.is_empty() {
        return Ok(0);
    }
    let local = load_tombstones(pool).await?;
    let known: HashMap<(&str, &str), &str> = local
        .iter()
        .map(|t| ((t.kind.as_str(), t.key.as_str()), t.deleted_at.as_str()))
        .collect();
    let merged = union_tombstones(&local, remote);
    let fresh: HashSet<String> = merged
        .iter()
        .filter(|t| t.kind == KIND_FLIGHT && t.active())
        .filter(|t| known.get(&(t.kind.as_str(), t.key.as_str())).map_or(true, |d| t.deleted_at.as_str() > *d))
        .map(|t| t.key.clone())
        .collect();
    let mut ids: Vec<i64> = Vec::new();
    if !fresh.is_empty() {
        let (_, key_id) = local_flight_keys(pool).await?;
        ids = key_id.into_iter().filter(|(k, _)| fresh.contains(k)).map(|(_, id)| id).collect();
        ids.sort_unstable();
        ids.dedup();
    }
    let mut tx = pool.begin().await?;
    for id in &ids {
        sqlx::query("DELETE FROM flight_pnl WHERE flight_id = ?1").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM flight_log WHERE id = ?1").bind(id).execute(&mut *tx).await?;
    }
    let mut maint = 0;
    for k in active_keys(&merged, KIND_MAINT) {
        let mut p = k.splitn(3, '|');
        let (Some(reg), Some(comp), Some(at)) = (p.next(), p.next(), p.next()) else { continue };
        maint += sqlx::query("DELETE FROM maintenance_log WHERE registration = ?1 AND component = ?2 AND serviced_at = ?3")
            .bind(reg)
            .bind(comp)
            .bind(at)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    for t in remote {
        upsert_tombstone(pool, t).await?;
    }
    if !ids.is_empty() {
        // Revertir YA el Tech Log de esos vuelos (antes de traer aviones de la
        // nube, que pueden venir ya revertidos) sin contarlo como un cambio de
        // esta PC: si el avión de la nube es más reciente, debe ganar.
        crate::mx::store::revert_orphans(pool).await?;
        crate::flight_log::purge_orphan_pnl(pool).await?;
        let mut docs = local_airframes(pool).await?;
        stamp(pool, &mut docs, &HashMap::new(), true).await?;
    }
    if !ids.is_empty() || maint > 0 {
        tracing::info!(target: "cloud", "borrados de otra PC: {} vuelo(s), {maint} cobro(s) de mantenimiento", ids.len());
    }
    Ok(ids.len())
}

/// Vuelos que se volvieron a añadir aquí después de borrarlos (reimportados):
/// su borrado queda sin efecto. Llamar antes de subir.
pub async fn revive_readded(pool: &SqlitePool) -> Result<usize> {
    let dead = active_keys(&load_tombstones(pool).await?, KIND_FLIGHT);
    if dead.is_empty() {
        return Ok(0);
    }
    let (_, key_id) = local_flight_keys(pool).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut n = 0;
    for k in key_id.keys().filter(|k| dead.contains(*k)) {
        sqlx::query("UPDATE sync_tombstone SET revived_at = ?2 WHERE kind = ?3 AND key = ?1")
            .bind(k)
            .bind(&now)
            .bind(KIND_FLIGHT)
            .execute(pool)
            .await?;
        n += 1;
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// Exportar
// ---------------------------------------------------------------------------

async fn rows(pool: &SqlitePool, sql: &str, reg: Option<&str>) -> Result<Vec<Value>> {
    let mut q = sqlx::query(sql);
    if let Some(r) = reg {
        q = q.bind(r);
    }
    Ok(q.fetch_all(pool).await?.iter().map(crate::cloud_sync::row_to_json).collect())
}

/// Aviones de esta PC (sin `updated_at`; lo pone `stamp`).
async fn local_airframes(pool: &SqlitePool) -> Result<Vec<AirframeDoc>> {
    let (id_key, _) = local_flight_keys(pool).await?;
    let mut out = Vec::new();
    for af in rows(pool, "SELECT * FROM mx_airframe ORDER BY registration", None).await? {
        let reg = s(&af, "registration").to_string();
        let r = Some(reg.as_str());
        let mut defects = rows(pool, "SELECT * FROM mx_defect WHERE registration = ?1 ORDER BY id", r).await?;
        for d in &mut defects {
            let k = d.get("raisedFlightId").and_then(|x| x.as_i64()).and_then(|id| id_key.get(&id).cloned());
            obj(d).insert("raisedFlightKey".into(), k.map(Value::String).unwrap_or(Value::Null));
        }
        out.push(AirframeDoc {
            components: rows(pool, "SELECT * FROM mx_component WHERE registration = ?1 ORDER BY key, position", r).await?,
            defects,
            checks: rows(pool, "SELECT * FROM mx_check WHERE registration = ?1 ORDER BY key", r).await?,
            servicing: rows(pool, "SELECT * FROM mx_servicing WHERE registration = ?1 ORDER BY at, id", r).await?,
            airframe: af,
            registration: reg,
            updated_at: String::new(),
        });
    }
    Ok(out)
}

/// Huella del contenido (estable entre versiones, a diferencia del hash de
/// la std).
fn content_hash(d: &AirframeDoc) -> String {
    use sha2::{Digest, Sha256};
    let body = serde_json::json!([d.airframe, d.components, d.defects, d.checks, d.servicing]);
    Sha256::digest(body.to_string().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Fecha más reciente que aparece en el avión (primera vez que se versiona).
fn latest_date(d: &AirframeDoc) -> String {
    let mut best = String::new();
    let mut see = |v: &Value, k: &str| {
        let x = s(v, k);
        if x > best.as_str() {
            best = x.to_string();
        }
    };
    see(&d.airframe, "lastSeenAt");
    see(&d.airframe, "createdAt");
    d.components.iter().for_each(|c| see(c, "installedAt"));
    for x in &d.defects {
        see(x, "raisedAt");
        see(x, "rectifiedAt");
    }
    d.checks.iter().for_each(|c| see(c, "lastDoneAt"));
    d.servicing.iter().for_each(|c| see(c, "at"));
    best
}

/// Pone la versión (`updated_at`) de cada avión: igual que la última vez si
/// su contenido no cambió; ahora si cambió (salvo `quiet`: cambio que viene
/// de otra PC); la de `forced` si se acaba de traer de la nube.
async fn stamp(pool: &SqlitePool, docs: &mut [AirframeDoc], forced: &HashMap<String, String>, quiet: bool) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for d in docs {
        let h = content_hash(d);
        let prev: Option<(String, String)> = sqlx::query_as("SELECT hash, updated_at FROM sync_airframe_rev WHERE registration = ?1")
            .bind(&d.registration)
            .fetch_optional(pool)
            .await?;
        let t = match (forced.get(&d.registration), &prev) {
            (Some(t), _) => t.clone(),
            (None, Some((ph, pt))) if *ph == h || quiet => pt.clone(),
            (None, Some(_)) => now.clone(),
            (None, None) => latest_date(d),
        };
        if prev.as_ref() != Some(&(h.clone(), t.clone())) {
            sqlx::query(
                "INSERT INTO sync_airframe_rev (registration, hash, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(registration) DO UPDATE SET hash = excluded.hash, updated_at = excluded.updated_at",
            )
            .bind(&d.registration)
            .bind(&h)
            .bind(&t)
            .execute(pool)
            .await?;
        }
        d.updated_at = t;
    }
    Ok(())
}

/// El libro de esta PC.
pub async fn export(pool: &SqlitePool) -> Result<Ledger> {
    let (id_key, _) = local_flight_keys(pool).await?;
    let fkey = |v: &Value| v.get("flightId").and_then(|x| x.as_i64()).and_then(|id| id_key.get(&id).cloned());

    let mut airframes = local_airframes(pool).await?;
    stamp(pool, &mut airframes, &HashMap::new(), false).await?;

    let mut mx_flights = Vec::new();
    for mut f in rows(pool, "SELECT * FROM mx_flight ORDER BY flight_id", None).await? {
        if let Some(k) = fkey(&f) {
            obj(&mut f).insert("flightKey".into(), Value::String(k));
            mx_flights.push(f);
        }
    }
    let mut ferries = Vec::new();
    for mut f in rows(pool, "SELECT * FROM aircraft_ferry WHERE source = 'requested' ORDER BY requested_at", None).await? {
        let k = fkey(&f);
        obj(&mut f).insert("flightKey".into(), k.map(Value::String).unwrap_or(Value::Null));
        ferries.push(f);
    }
    let settings = rows(pool, "SELECT key, value, updated_at FROM settings ORDER BY key", None)
        .await?
        .into_iter()
        .filter(|v| synced_setting(s(v, "key")))
        .collect();

    let mut l = Ledger {
        airframes,
        mx_flights,
        maintenance_log: rows(pool, "SELECT registration, component, cost, serviced_at FROM maintenance_log ORDER BY serviced_at, id", None).await?,
        techlog_seen: rows(pool, "SELECT * FROM techlog_seen ORDER BY event_key", None).await?,
        gsx_lines: rows(
            pool,
            "SELECT receipt_id, line, kind, service, side, airport, registration, at, amount_usd, uplift_kg, is_cost
             FROM gsx_charge ORDER BY receipt_id, line",
            None,
        )
        .await?,
        policies: rows(pool, "SELECT * FROM airline_policy ORDER BY airline_key", None).await?,
        ferries,
        reservations: rows(pool, "SELECT * FROM aircraft_reservation ORDER BY reserved_at", None).await?,
        auto_daily: rows(pool, "SELECT * FROM auto_daily ORDER BY airline_key, day", None).await?,
        settings,
    };
    normalize(&mut l);
    Ok(l)
}

// ---------------------------------------------------------------------------
// Unir dos libros
// ---------------------------------------------------------------------------

fn k_mxf(v: &Value) -> String {
    s(v, "flightKey").to_string()
}
fn k_maint(v: &Value) -> String {
    maint_key(s(v, "registration"), s(v, "component"), s(v, "servicedAt"))
}
fn k_seen(v: &Value) -> String {
    s(v, "eventKey").to_string()
}
fn k_gsx(v: &Value) -> String {
    format!("{}|{}", s(v, "receiptId"), v.get("line").and_then(|x| x.as_i64()).unwrap_or(0))
}
fn k_policy(v: &Value) -> String {
    s(v, "airlineKey").to_string()
}
fn k_ferry(v: &Value) -> String {
    format!("{}|{}", s(v, "registration"), s(v, "requestedAt"))
}
fn k_resv(v: &Value) -> String {
    format!("{}|{}", s(v, "registration"), s(v, "reservedAt"))
}
fn k_day(v: &Value) -> String {
    format!("{}|{}", s(v, "airlineKey"), s(v, "day"))
}
fn k_setting(v: &Value) -> String {
    s(v, "key").to_string()
}

/// Orden estable (para comparar libros y subir sólo si cambió algo).
fn normalize(l: &mut Ledger) {
    l.airframes.sort_by(|a, b| a.registration.cmp(&b.registration));
    l.mx_flights.sort_by_key(k_mxf);
    l.maintenance_log.sort_by_key(k_maint);
    l.techlog_seen.sort_by_key(k_seen);
    l.gsx_lines.sort_by_key(k_gsx);
    l.policies.sort_by_key(k_policy);
    l.ferries.sort_by_key(k_ferry);
    l.reservations.sort_by_key(k_resv);
    l.auto_daily.sort_by_key(k_day);
    l.settings.sort_by_key(k_setting);
}

/// Una reserva terminada manda sobre la activa.
fn resv_newer(cur: &Value, new: &Value) -> bool {
    s(cur, "endedAt").is_empty() && !s(new, "endedAt").is_empty()
}
/// Un traslado cancelado manda sobre el activo.
fn ferry_newer(cur: &Value, new: &Value) -> bool {
    s(cur, "status") == "active" && s(new, "status") != "active"
}
fn policy_newer(cur: &Value, new: &Value) -> bool {
    s(new, "updatedAt") > s(cur, "updatedAt")
}
/// Un ajuste sólo cambia si cambió su valor (la fecha de un ajuste por
/// defecto es la de instalación de cada PC).
fn setting_newer(cur: &Value, new: &Value) -> bool {
    if s(new, "value") == s(cur, "value") {
        return false;
    }
    // Desde cuándo se cobran traslados automáticos: vale el de la primera PC.
    if s(cur, "key") == "ferry_auto_since" {
        return s(new, "value") < s(cur, "value");
    }
    s(new, "updatedAt") > s(cur, "updatedAt")
}
fn day_newer(cur: &Value, new: &Value) -> bool {
    s(new, "computedAt") > s(cur, "computedAt")
}
fn keep(_: &Value, _: &Value) -> bool {
    false
}

/// Une por clave; con la misma clave queda la de `a` salvo que `newer(a, b)`.
fn union_by(a: &[Value], b: &[Value], key: fn(&Value) -> String, newer: fn(&Value, &Value) -> bool) -> Vec<Value> {
    let mut map: HashMap<String, Value> = HashMap::new();
    for v in a {
        map.insert(key(v), v.clone());
    }
    for v in b {
        let k = key(v);
        if map.get(&k).map_or(true, |cur| newer(cur, v)) {
            map.insert(k, v.clone());
        }
    }
    map.into_values().collect()
}

/// Une dos libros (nube y local) sin perder nada ni duplicar, y quita lo
/// borrado. Con empates exactos queda `a`.
pub fn merge(a: &Ledger, b: &Ledger, tombs: &[Tombstone]) -> Ledger {
    let dead_flights = active_keys(tombs, KIND_FLIGHT);
    let dead_maint = active_keys(tombs, KIND_MAINT);
    let mut afs: HashMap<String, AirframeDoc> = HashMap::new();
    for d in a.airframes.iter().chain(&b.airframes) {
        if afs.get(&d.registration).map_or(true, |cur| d.wins_over(cur)) {
            afs.insert(d.registration.clone(), d.clone());
        }
    }
    let mut out = Ledger {
        airframes: afs.into_values().collect(),
        mx_flights: union_by(&a.mx_flights, &b.mx_flights, k_mxf, keep)
            .into_iter()
            .filter(|v| !dead_flights.contains(&k_mxf(v)))
            .collect(),
        maintenance_log: union_by(&a.maintenance_log, &b.maintenance_log, k_maint, keep)
            .into_iter()
            .filter(|v| !dead_maint.contains(&k_maint(v)))
            .collect(),
        techlog_seen: union_by(&a.techlog_seen, &b.techlog_seen, k_seen, keep),
        gsx_lines: union_by(&a.gsx_lines, &b.gsx_lines, k_gsx, keep),
        policies: union_by(&a.policies, &b.policies, k_policy, policy_newer),
        ferries: union_by(&a.ferries, &b.ferries, k_ferry, ferry_newer),
        reservations: union_by(&a.reservations, &b.reservations, k_resv, resv_newer),
        auto_daily: union_by(&a.auto_daily, &b.auto_daily, k_day, day_newer),
        settings: union_by(&a.settings, &b.settings, k_setting, setting_newer),
    };
    normalize(&mut out);
    out
}

// ---------------------------------------------------------------------------
// Aplicar en esta PC
// ---------------------------------------------------------------------------

fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

type ColCache = HashMap<String, HashSet<String>>;

async fn columns(tx: &mut Transaction<'_, Sqlite>, table: &str, cache: &mut ColCache) -> Result<HashSet<String>> {
    if let Some(c) = cache.get(table) {
        return Ok(c.clone());
    }
    let cols: HashSet<String> = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    cache.insert(table.to_string(), cols.clone());
    Ok(cols)
}

/// INSERT de una fila JSON (camelCase) en `table`: sólo las columnas que
/// existen aquí, sin las de `skip`, con `extra` (snake_case) encima.
async fn insert_json(
    tx: &mut Transaction<'_, Sqlite>,
    cache: &mut ColCache,
    verb: &str,
    table: &str,
    row: &Value,
    skip: &[&str],
    extra: &[(&str, Value)],
) -> Result<u64> {
    let cols = columns(tx, table, cache).await?;
    let mut names: Vec<String> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    for (k, v) in row.as_object().into_iter().flatten() {
        let col = camel_to_snake(k);
        if skip.contains(&k.as_str()) || !cols.contains(&col) || extra.iter().any(|(e, _)| *e == col) {
            continue;
        }
        names.push(col);
        vals.push(v.clone());
    }
    for (c, v) in extra {
        if cols.contains(*c) {
            names.push((*c).to_string());
            vals.push(v.clone());
        }
    }
    if names.is_empty() {
        return Ok(0);
    }
    let sql = format!("{verb} INTO {table} ({}) VALUES ({})", names.join(", "), vec!["?"; names.len()].join(", "));
    let mut q = sqlx::query(&sql);
    for v in vals {
        q = match v {
            Value::Null => q.bind(None::<String>),
            Value::Bool(b) => q.bind(b as i64),
            Value::Number(n) => match n.as_i64() {
                Some(i) => q.bind(i),
                None => q.bind(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => q.bind(s),
            other => q.bind(other.to_string()),
        };
    }
    Ok(q.execute(&mut **tx).await?.rows_affected())
}

async fn last_id(tx: &mut Transaction<'_, Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT last_insert_rowid()").fetch_one(&mut **tx).await?)
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub airframes: usize,
    pub mx_flights: usize,
    pub maintenance: usize,
    pub gsx_lines: usize,
    pub other: usize,
}

impl ApplyReport {
    pub fn any(&self) -> bool {
        self.airframes + self.mx_flights + self.maintenance + self.gsx_lines + self.other > 0
    }
}

/// Trae a esta PC lo que el libro `remote` tiene y ella no. No borra nada
/// propio salvo los aviones cuya versión remota es más reciente (se
/// reemplazan enteros). Lo borrado (`tombs`) no entra.
pub async fn apply(pool: &SqlitePool, remote: &Ledger, tombs: &[Tombstone]) -> Result<ApplyReport> {
    let local = export(pool).await?;
    let (_, key_id) = local_flight_keys(pool).await?;
    let flight_of = |v: &Value, field: &str| {
        v.get(field)
            .and_then(|x| x.as_str())
            .and_then(|k| key_id.get(k))
            .map(|id| Value::from(*id))
            .unwrap_or(Value::Null)
    };
    let dead_flights = active_keys(tombs, KIND_FLIGHT);
    let dead_maint = active_keys(tombs, KIND_MAINT);
    let mut rep = ApplyReport::default();
    let mut forced: HashMap<String, String> = HashMap::new();
    let mut cache = ColCache::new();
    let mut tx = pool.begin().await?;

    // Aviones: el más reciente, entero.
    for doc in &remote.airframes {
        let mine = local.airframes.iter().find(|d| d.registration == doc.registration);
        if mine.is_some_and(|m| !doc.wins_over(m)) || doc.airframe.is_null() {
            continue;
        }
        for t in ["mx_component", "mx_defect", "mx_check", "mx_servicing", "mx_airframe"] {
            sqlx::query(&format!("DELETE FROM {t} WHERE registration = ?1"))
                .bind(&doc.registration)
                .execute(&mut *tx)
                .await?;
        }
        insert_json(&mut tx, &mut cache, "INSERT", "mx_airframe", &doc.airframe, &[], &[]).await?;
        let mut defect_ids: HashMap<i64, i64> = HashMap::new();
        for d in &doc.defects {
            let skip = ["id", "raisedFlightKey", "raisedFlightId"];
            insert_json(&mut tx, &mut cache, "INSERT", "mx_defect", d, &skip, &[("raised_flight_id", flight_of(d, "raisedFlightKey"))]).await?;
            if let Some(old) = d.get("id").and_then(|x| x.as_i64()) {
                defect_ids.insert(old, last_id(&mut tx).await?);
            }
        }
        for c in &doc.components {
            let defect = c
                .get("defectId")
                .and_then(|x| x.as_i64())
                .and_then(|old| defect_ids.get(&old))
                .map(|id| Value::from(*id))
                .unwrap_or(Value::Null);
            insert_json(&mut tx, &mut cache, "INSERT", "mx_component", c, &["id", "defectId"], &[("defect_id", defect)]).await?;
        }
        for c in &doc.checks {
            insert_json(&mut tx, &mut cache, "INSERT OR REPLACE", "mx_check", c, &[], &[]).await?;
        }
        for sv in &doc.servicing {
            insert_json(&mut tx, &mut cache, "INSERT", "mx_servicing", sv, &["id"], &[]).await?;
        }
        forced.insert(doc.registration.clone(), doc.updated_at.clone());
        rep.airframes += 1;
    }

    // Registro del Tech Log por vuelo (si el vuelo está en esta PC).
    let have: HashSet<String> = local.mx_flights.iter().map(k_mxf).collect();
    for f in &remote.mx_flights {
        let k = k_mxf(f);
        let Some(id) = key_id.get(&k) else { continue };
        if have.contains(&k) || dead_flights.contains(&k) {
            continue;
        }
        let extra = [("flight_id", Value::from(*id))];
        rep.mx_flights += insert_json(&mut tx, &mut cache, "INSERT OR IGNORE", "mx_flight", f, &["flightKey"], &extra).await? as usize;
    }

    let have: HashSet<String> = local.maintenance_log.iter().map(k_maint).collect();
    for m in &remote.maintenance_log {
        let k = k_maint(m);
        if !have.contains(&k) && !dead_maint.contains(&k) {
            rep.maintenance += insert_json(&mut tx, &mut cache, "INSERT", "maintenance_log", m, &["id"], &[]).await? as usize;
        }
    }

    let have: HashSet<String> = local.techlog_seen.iter().map(k_seen).collect();
    for v in remote.techlog_seen.iter().filter(|v| !have.contains(&k_seen(v))) {
        rep.other += insert_json(&mut tx, &mut cache, "INSERT OR IGNORE", "techlog_seen", v, &[], &[]).await? as usize;
    }
    let have: HashSet<String> = local.gsx_lines.iter().map(k_gsx).collect();
    for v in remote.gsx_lines.iter().filter(|v| !have.contains(&k_gsx(v))) {
        rep.gsx_lines += insert_json(&mut tx, &mut cache, "INSERT OR IGNORE", "gsx_charge", v, &["flightId"], &[]).await? as usize;
    }
    let mine: HashMap<String, &Value> = local.policies.iter().map(|v| (k_policy(v), v)).collect();
    for v in &remote.policies {
        if mine.get(&k_policy(v)).map_or(true, |l| policy_newer(l, v)) {
            rep.other += insert_json(&mut tx, &mut cache, "INSERT OR REPLACE", "airline_policy", v, &[], &[]).await? as usize;
        }
    }
    let mine: HashMap<String, &Value> = local.ferries.iter().map(|v| (k_ferry(v), v)).collect();
    for v in &remote.ferries {
        match mine.get(&k_ferry(v)) {
            None => {
                let skip = ["id", "flightKey", "flightId"];
                rep.other += insert_json(&mut tx, &mut cache, "INSERT", "aircraft_ferry", v, &skip, &[("flight_id", flight_of(v, "flightKey"))])
                    .await? as usize;
            }
            Some(l) if ferry_newer(l, v) => {
                sqlx::query("UPDATE aircraft_ferry SET status = ?3, cancelled_at = ?4 WHERE registration = ?1 AND requested_at = ?2")
                    .bind(s(v, "registration"))
                    .bind(s(v, "requestedAt"))
                    .bind(s(v, "status"))
                    .bind(v.get("cancelledAt").and_then(|x| x.as_str()))
                    .execute(&mut *tx)
                    .await?;
                rep.other += 1;
            }
            _ => {}
        }
    }
    let mine: HashMap<String, &Value> = local.reservations.iter().map(|v| (k_resv(v), v)).collect();
    for v in &remote.reservations {
        match mine.get(&k_resv(v)) {
            None => {
                rep.other += insert_json(&mut tx, &mut cache, "INSERT", "aircraft_reservation", v, &["id"], &[]).await? as usize;
            }
            Some(l) if resv_newer(l, v) => {
                sqlx::query(
                    "UPDATE aircraft_reservation SET status = ?3, ended_at = ?4, landed_icao = ?5, landed_at = ?6
                     WHERE registration = ?1 AND reserved_at = ?2",
                )
                .bind(s(v, "registration"))
                .bind(s(v, "reservedAt"))
                .bind(s(v, "status"))
                .bind(s(v, "endedAt"))
                .bind(v.get("landedIcao").and_then(|x| x.as_str()))
                .bind(v.get("landedAt").and_then(|x| x.as_str()))
                .execute(&mut *tx)
                .await?;
                rep.other += 1;
            }
            _ => {}
        }
    }
    let mine: HashMap<String, &Value> = local.auto_daily.iter().map(|v| (k_day(v), v)).collect();
    for v in &remote.auto_daily {
        if mine.get(&k_day(v)).map_or(true, |l| day_newer(l, v)) {
            rep.other += insert_json(&mut tx, &mut cache, "INSERT OR REPLACE", "auto_daily", v, &[], &[]).await? as usize;
        }
    }
    let mine: HashMap<String, &Value> = local.settings.iter().map(|v| (k_setting(v), v)).collect();
    for v in remote.settings.iter().filter(|v| synced_setting(s(v, "key"))) {
        if mine.get(&k_setting(v)).map_or(true, |l| setting_newer(l, v)) {
            rep.other += insert_json(&mut tx, &mut cache, "INSERT OR REPLACE", "settings", v, &[], &[]).await? as usize;
        }
    }
    tx.commit().await?;

    // Los aviones traídos quedan con la versión de la nube (no cuentan como
    // un cambio de esta PC).
    if !forced.is_empty() {
        let mut docs = local_airframes(pool).await?;
        stamp(pool, &mut docs, &forced, false).await?;
    }
    if rep.any() {
        tracing::info!(
            target: "cloud",
            "libro de la nube aplicado: {} avión(es), {} registro(s) de vuelo, {} cobro(s) de mantenimiento, {} línea(s) GSX, {} otros",
            rep.airframes,
            rep.mx_flights,
            rep.maintenance,
            rep.gsx_lines,
            rep.other
        );
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tomb(kind: &str, key: &str, deleted: &str, revived: Option<&str>) -> Tombstone {
        Tombstone { kind: kind.into(), key: key.into(), deleted_at: deleted.into(), revived_at: revived.map(String::from) }
    }

    #[test]
    fn a_delete_after_a_revive_counts_again() {
        let a = vec![tomb(KIND_FLIGHT, "k", "2026-10-01", None)];
        let b = vec![tomb(KIND_FLIGHT, "k", "2026-09-01", Some("2026-10-05"))];
        let u = union_tombstones(&a, &b);
        assert_eq!(u.len(), 1);
        assert!(!u[0].active(), "se reimportó después del último borrado");
        assert_eq!(u, union_tombstones(&b, &a), "el orden de las PCs no importa");
        let again = union_tombstones(&u, &[tomb(KIND_FLIGHT, "k", "2026-10-09", None)]);
        assert!(again[0].active(), "borrado otra vez");
    }

    #[test]
    fn a_blank_airframe_never_overwrites_one_with_history() {
        let used = AirframeDoc {
            registration: "ECTST".into(),
            updated_at: "2026-10-01".into(),
            airframe: json!({ "lastSeenAt": "2026-10-01" }),
            ..Default::default()
        };
        let blank = AirframeDoc {
            updated_at: "2026-10-20".into(),
            airframe: json!({ "lastSeenAt": null }),
            ..used.clone()
        };
        let l = |d: &AirframeDoc| Ledger { airframes: vec![d.clone()], ..Default::default() };
        assert_eq!(merge(&l(&used), &l(&blank), &[]).airframes, vec![used.clone()]);
        assert_eq!(merge(&l(&blank), &l(&used), &[]).airframes, vec![used.clone()]);
        let newer = AirframeDoc { updated_at: "2026-10-05".into(), ..used.clone() };
        assert_eq!(merge(&l(&used), &l(&newer), &[]).airframes, vec![newer.clone()], "entre dos usados, el más reciente");
    }

    #[test]
    fn merge_unites_without_duplicates_and_drops_what_was_deleted() {
        let m = |c: &str, at: &str| json!({ "registration": "ECTST", "component": c, "cost": 100.0, "servicedAt": at });
        let a = Ledger {
            maintenance_log: vec![m("x", "t1"), m("y", "t2")],
            mx_flights: vec![json!({ "flightKey": "F1" }), json!({ "flightKey": "F2" })],
            ..Default::default()
        };
        let b = Ledger {
            maintenance_log: vec![m("y", "t2"), m("z", "t3")],
            mx_flights: vec![json!({ "flightKey": "F2" })],
            ..Default::default()
        };
        let tombs = [tomb(KIND_MAINT, "ECTST|x|t1", "t9", None), tomb(KIND_FLIGHT, "F1", "t9", None)];
        let out = merge(&a, &b, &tombs);
        let keys: Vec<String> = out.maintenance_log.iter().map(k_maint).collect();
        assert_eq!(keys, ["ECTST|y|t2", "ECTST|z|t3"]);
        assert_eq!(out.mx_flights, vec![json!({ "flightKey": "F2" })]);
        assert_eq!(out, merge(&b, &a, &tombs), "el orden de las PCs no importa");
    }

    #[test]
    fn a_setting_only_travels_when_its_value_changed() {
        let v = |val: &str, at: &str| json!({ "key": "mx_wear_rate", "value": val, "updatedAt": at });
        let a = Ledger { settings: vec![v("1", "2026-10-01")], ..Default::default() };
        let b = Ledger { settings: vec![v("1", "2026-10-09")], ..Default::default() };
        assert_eq!(merge(&a, &b, &[]), a, "mismo valor: no cambia nada");
        let c = Ledger { settings: vec![v("2", "2026-10-09")], ..Default::default() };
        assert_eq!(merge(&a, &c, &[]), c);
    }

    #[test]
    fn flight_keys_ignore_empty_external_ids() {
        let row = json!({ "source": "vas", "externalId": "", "startedAt": "2026-10-20T10:00:00Z", "originLat": 18.456, "originLon": -69.1 });
        assert_eq!(json_flight_keys(&row), (None, "2026-10-20T10:00:00Z|18.46|-69.10".to_string()));
        let row = json!({ "source": "vas", "externalId": "77", "startedAt": "x", "originLat": 0, "originLon": 0 });
        assert_eq!(json_flight_keys(&row).0.as_deref(), Some("vas|77"));
    }
}
